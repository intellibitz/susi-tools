//! The copied-code report printed beside the gates.

use std::collections::HashMap;
use std::path::Path;
use std::process::{Command, Stdio};

const MINIMUM_LINES: usize = 12;

#[derive(Clone, Debug, Eq, PartialEq)]
struct Line {
    text: String,
    number: usize,
}

struct AddedBlock {
    path: String,
    lines: Vec<Line>,
}

struct Source {
    path: String,
    lines: Vec<Line>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct Copy {
    added_path: String,
    added_line: usize,
    existing_path: String,
    existing_line: usize,
}

/// Return the report for the work not yet on the remote's main. A report is advisory: a Git
/// problem must not turn a copied-code notice into a failed gate.
pub fn report(work: &Path) -> String {
    let copies = find(work).unwrap_or_default();
    if copies.is_empty() {
        return "copies: none\n".to_owned();
    }
    copies
        .iter()
        .map(|copy| {
            format!(
                "copies: {}:{} and {}:{}\n",
                copy.added_path, copy.added_line, copy.existing_path, copy.existing_line
            )
        })
        .collect()
}

fn find(work: &Path) -> Result<Vec<Copy>, String> {
    [
        "refs/susi-tools/remote/main",
        "refs/susi-tools/land/main",
        "origin/main",
        "main",
    ]
    .iter()
    .find_map(|name| git(work, &["rev-parse", "--verify", name]).ok())
    .map(|reference| reference.trim().to_owned())
    .filter(|reference| !reference.is_empty())
    .ok_or_else(|| "the remote main is not available".to_owned())
    .and_then(|reference| git(work, &["merge-base", "HEAD", &reference]))
    .and_then(|base| {
        let diff = git(
            work,
            &[
                "diff",
                "--no-color",
                "--no-ext-diff",
                "--unified=0",
                "--no-renames",
                base.trim(),
                "HEAD",
                "--",
                "*.rs",
            ],
        )?;
        let added = added_blocks(&diff);
        if added.is_empty() {
            return Ok(Vec::new());
        }
        let sources = sources(work)?;
        Ok(matches(&added, &sources))
    })
}

fn git(work: &Path, args: &[&str]) -> Result<String, String> {
    let output = Command::new(crate::git_program::path())
        .current_dir(work)
        .args(args)
        .stdin(Stdio::null())
        .output()
        .map_err(|error| format!("git {}: {error}", args.join(" ")))?;
    if !output.status.success() {
        return Err(format!(
            "git {}: {}",
            args.join(" "),
            String::from_utf8_lossy(&output.stderr).trim()
        ));
    }
    String::from_utf8(output.stdout)
        .map_err(|error| format!("git {} printed non-UTF-8: {error}", args.join(" ")))
}

fn is_source(path: &str) -> bool {
    path.ends_with(".rs") && !path.split('/').any(|component| component == "tests")
}

fn sources(work: &Path) -> Result<Vec<Source>, String> {
    let listing = git(work, &["ls-tree", "-r", "-z", "--name-only", "HEAD"])?;
    listing
        .split('\0')
        .filter(|path| !path.is_empty() && is_source(path))
        .map(|path| {
            let text = std::fs::read_to_string(work.join(path))
                .map_err(|error| format!("read {path}: {error}"))?;
            Ok(Source {
                path: path.to_owned(),
                lines: source_lines(&text),
            })
        })
        .collect()
}

fn source_lines(text: &str) -> Vec<Line> {
    text.lines()
        .enumerate()
        .filter_map(|(index, text)| {
            let text = text.trim();
            (!text.is_empty()).then(|| Line {
                text: text.to_owned(),
                number: index + 1,
            })
        })
        .collect()
}

fn added_blocks(diff: &str) -> Vec<AddedBlock> {
    let mut blocks = Vec::new();
    let mut path = None;
    let mut next_line = 0;
    let mut current = Vec::new();

    let finish = |blocks: &mut Vec<AddedBlock>, path: &Option<String>, current: &mut Vec<Line>| {
        if !current.is_empty() {
            if let Some(path) = path.as_ref().filter(|path| is_source(path)) {
                blocks.push(AddedBlock {
                    path: path.clone(),
                    lines: std::mem::take(current),
                });
            } else {
                current.clear();
            }
        }
    };

    for raw in diff.lines() {
        if raw.starts_with("diff --git ") {
            finish(&mut blocks, &path, &mut current);
            path = None;
            next_line = 0;
        } else if let Some(file) = raw.strip_prefix("+++ b/") {
            finish(&mut blocks, &path, &mut current);
            path = Some(file.to_owned());
        } else if raw.starts_with("+++ ") {
            finish(&mut blocks, &path, &mut current);
            path = None;
        } else if raw.starts_with("@@ ") {
            finish(&mut blocks, &path, &mut current);
            next_line = new_line(raw).unwrap_or(0);
        } else if let Some(text) = raw.strip_prefix('+') {
            if next_line > 0 {
                current.push(Line {
                    text: text.to_owned(),
                    number: next_line,
                });
                next_line += 1;
            }
        } else if raw.starts_with(' ') {
            finish(&mut blocks, &path, &mut current);
            next_line += 1;
        }
    }
    finish(&mut blocks, &path, &mut current);
    blocks
}

fn new_line(header: &str) -> Option<usize> {
    header
        .split_whitespace()
        .find_map(|part| part.strip_prefix('+'))
        .and_then(|range| range.split(',').next())
        .and_then(|start| start.parse().ok())
}

fn matches(added: &[AddedBlock], sources: &[Source]) -> Vec<Copy> {
    let mut index: HashMap<Vec<String>, Vec<(usize, usize)>> = HashMap::new();
    for (source_index, source) in sources.iter().enumerate() {
        if source.lines.len() < MINIMUM_LINES {
            continue;
        }
        for start in 0..=source.lines.len() - MINIMUM_LINES {
            let key = source.lines[start..start + MINIMUM_LINES]
                .iter()
                .map(|line| line.text.clone())
                .collect();
            index.entry(key).or_default().push((source_index, start));
        }
    }

    let mut copies = Vec::new();
    for block in added {
        let lines: Vec<Line> = block
            .lines
            .iter()
            .filter_map(|line| {
                let text = line.text.trim();
                (!text.is_empty()).then(|| Line {
                    text: text.to_owned(),
                    number: line.number,
                })
            })
            .collect();
        if lines.len() < MINIMUM_LINES {
            continue;
        }
        for start in 0..=lines.len() - MINIMUM_LINES {
            let key = lines[start..start + MINIMUM_LINES]
                .iter()
                .map(|line| line.text.clone())
                .collect::<Vec<_>>();
            let Some(candidates) = index.get(&key) else {
                continue;
            };
            for &(source_index, source_start) in candidates {
                let source = &sources[source_index];
                let mut left = 0;
                while start > left
                    && source_start > left
                    && lines[start - left - 1].text == source.lines[source_start - left - 1].text
                {
                    left += 1;
                }
                let mut right = MINIMUM_LINES;
                while start + right < lines.len()
                    && source_start + right < source.lines.len()
                    && lines[start + right].text == source.lines[source_start + right].text
                {
                    right += 1;
                }
                let added_start = start - left;
                let source_start = source_start - left;
                if source.path == block.path
                    && source.lines[source_start].number == lines[added_start].number
                {
                    continue;
                }
                let copy = Copy {
                    added_path: block.path.clone(),
                    added_line: lines[added_start].number,
                    existing_path: source.path.clone(),
                    existing_line: source.lines[source_start].number,
                };
                if !copies.contains(&copy) {
                    copies.push(copy);
                }
            }
        }
    }
    copies.sort_by(|left, right| {
        (
            &left.added_path,
            left.added_line,
            &left.existing_path,
            left.existing_line,
        )
            .cmp(&(
                &right.added_path,
                right.added_line,
                &right.existing_path,
                right.existing_line,
            ))
    });
    copies
}

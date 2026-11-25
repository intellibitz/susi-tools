// SPDX-License-Identifier: Apache-2.0
//! Susi Tools' documents name only commands its program answers, and the
//! ecosystem page says what the ecosystem is (T-CLAUDE-34).
//!
//! Every `susi-tools <command>` a document names must be a command
//! `susi-tools --help` lists; where the help lists subcommands of that
//! command, a subcommand the document names must be one of them. A document
//! that names a command the program does not answer fails here, so the
//! documents cannot promise what does not exist.

use std::process::Command;

const DOCUMENTS: &[&str] = &["README.md", "ECOSYSTEM.md"];

fn usage() -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_susi-tools"))
        .arg("--help")
        .output()
        .unwrap();
    assert!(out.status.success(), "susi-tools --help failed");
    String::from_utf8(out.stdout).unwrap()
}

/// The words after each `susi-tools ` in `text`, with line breaks and
/// indentation read as one space, so a command wrapped over two lines counts.
fn named_commands(text: &str) -> Vec<(String, Option<String>)> {
    let flat = text.split_whitespace().collect::<Vec<_>>().join(" ");
    let mut found = Vec::new();
    let mut rest = flat.as_str();
    while let Some(at) = rest.find("susi-tools ") {
        let preceded_by_word = rest[..at]
            .chars()
            .last()
            .is_some_and(|c| c.is_ascii_alphanumeric() || c == '-' || c == '/');
        rest = &rest[at + "susi-tools ".len()..];
        if preceded_by_word {
            continue;
        }
        let mut words = rest.split(' ');
        let Some(raw) = words.next() else { continue };
        let first = raw.trim_end_matches(|c: char| !(c.is_ascii_lowercase() || c == '-'));
        if !is_command_word(first) {
            continue;
        }
        // A second word belongs to the command only when nothing closes the first.
        let second = if first.len() == raw.len() {
            words
                .next()
                .map(|w| w.trim_end_matches(|c: char| !(c.is_ascii_lowercase() || c == '-')))
                .filter(|w| is_command_word(w))
                .map(str::to_owned)
        } else {
            None
        };
        found.push((first.to_owned(), second));
    }
    found
}

fn is_command_word(word: &str) -> bool {
    word.starts_with(|c: char| c.is_ascii_lowercase())
        && word.chars().all(|c| c.is_ascii_lowercase() || c == '-')
}

#[test]
fn documents_name_only_commands_the_program_answers() {
    let usage = usage();
    let listed = named_commands(&usage);
    let mut wrong = Vec::new();
    for document in DOCUMENTS {
        let text = std::fs::read_to_string(document).unwrap();
        for (command, sub) in named_commands(&text) {
            if !listed.iter().any(|(c, _)| *c == command) {
                wrong.push(format!("{document}: susi-tools {command}"));
                continue;
            }
            // The help's lines for this command, each read as its words: a
            // subcommand the document names must be among them.
            let lines: Vec<Vec<&str>> = usage
                .lines()
                .filter(|l| l.contains(&format!("susi-tools {command} ")))
                .map(|l| l.split_whitespace().collect())
                .collect();
            let has_subs = listed.iter().any(|(c, s)| *c == command && s.is_some());
            if let Some(sub) = sub {
                if has_subs && !lines.iter().any(|words| words.contains(&sub.as_str())) {
                    wrong.push(format!("{document}: susi-tools {command} {sub}"));
                }
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "documents name commands `susi-tools --help` does not list: {wrong:?}"
    );
}

#[test]
fn a_command_the_program_lacks_is_caught() {
    let listed = named_commands(&usage());
    let named = named_commands("run `susi-tools frobnicate` and `susi-tools\n  tasks juggle`");
    assert_eq!(
        named,
        vec![
            ("frobnicate".to_owned(), None),
            ("tasks".to_owned(), Some("juggle".to_owned()))
        ]
    );
    assert!(!listed.iter().any(|(c, _)| c == "frobnicate"));
    assert!(!listed
        .iter()
        .any(|(c, s)| c == "tasks" && s.as_deref() == Some("juggle")));
    assert!(listed
        .iter()
        .any(|(c, s)| c == "tasks" && s.as_deref() == Some("next")));
}

#[test]
fn the_ecosystem_page_says_what_the_ecosystem_is() {
    let page = std::fs::read_to_string("ECOSYSTEM.md").unwrap();
    for needle in [
        "**Susi**",
        "**Susi Tools**",
        "**Susi Core**",
        "etired",
        "Apache-2.0",
    ] {
        assert!(page.contains(needle), "ECOSYSTEM.md lacks {needle:?}");
    }
}

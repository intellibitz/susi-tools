// SPDX-License-Identifier: Apache-2.0
//! Talking to repositories through the `git` program. Every call names the repository
//! explicitly and takes no optional locks. Reads write nothing. Writes go only through Git itself
//! (objects, refs, commits and pushes): no file of this program's own is ever left in a checkout.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Repo {
    git_dir: PathBuf,
    work_tree: Option<PathBuf>,
}

/// One ref change in an atomic push. `new: None` deletes the ref. `expect` is the value the ref
/// must still have on the remote (`None`: it must not exist), so a change made meanwhile by
/// anyone else refuses the whole push instead of being overwritten.
pub struct Update {
    pub refname: String,
    pub new: Option<String>,
    pub expect: Option<String>,
}

pub enum PushOutcome {
    Done,
    /// The remote moved under us (a lease did not hold); nothing was changed.
    Stale,
}

impl Repo {
    /// Open a repository: a bare repository, a checkout, or a linked worktree.
    pub fn open(path: &Path) -> Result<Repo, String> {
        let out = base_command()
            .arg("-C")
            .arg(path)
            .args([
                "rev-parse",
                "--absolute-git-dir",
                "--is-bare-repository",
                "--show-toplevel",
            ])
            .output()
            .map_err(|e| format!("cannot run git: {e}"))?;
        let text = String::from_utf8_lossy(&out.stdout).to_string();
        let mut lines = text.lines();
        let git_dir = lines.next().unwrap_or_default().trim().to_owned();
        if git_dir.is_empty() {
            return Err(format!(
                "{} is not a Git repository: {}",
                path.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let bare = lines.next().map(str::trim) == Some("true");
        let work_tree = if bare {
            None
        } else {
            lines.next().map(|t| PathBuf::from(t.trim()))
        };
        Ok(Repo {
            git_dir: PathBuf::from(git_dir),
            work_tree,
        })
    }

    pub fn work_tree(&self) -> Result<&Path, String> {
        self.work_tree
            .as_deref()
            .ok_or_else(|| "this needs a checkout, not a bare repository".to_owned())
    }

    fn command(&self) -> Command {
        let mut command = base_command();
        command.arg("--git-dir").arg(&self.git_dir);
        if let Some(work) = &self.work_tree {
            command.arg("--work-tree").arg(work).current_dir(work);
        }
        command
    }

    /// Run a git command and return its standard output, or its error text.
    pub fn read(&self, args: &[&str]) -> Result<String, String> {
        let out = self
            .command()
            .args(args)
            .output()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git {} failed: {}",
                args.join(" "),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        String::from_utf8(out.stdout)
            .map_err(|e| format!("git {} printed non-UTF-8: {e}", args.join(" ")))
    }

    /// Like `read`, but a failure is `None` rather than an error.
    pub fn try_read(&self, args: &[&str]) -> Option<String> {
        self.read(args).ok()
    }

    /// Run a git command with the hooks and terminal of the caller (commits, merges).
    pub fn run_visible(&self, args: &[&str]) -> Result<(), String> {
        let status = self
            .command()
            .args(args)
            .status()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if status.success() {
            Ok(())
        } else {
            Err(format!("git {} failed", args.join(" ")))
        }
    }

    /// `(refname, object id)` for every ref under `prefix`.
    pub fn refs(&self, prefix: &str) -> Result<Vec<(String, String)>, String> {
        let text = self.read(&["for-each-ref", "--format=%(refname) %(objectname)", prefix])?;
        Ok(pairs(&text))
    }

    /// `(refname, object id)` for the remote's refs matching `patterns`, read without fetching.
    pub fn remote_refs(
        &self,
        remote: &str,
        patterns: &[&str],
    ) -> Result<Vec<(String, String)>, String> {
        let mut args = vec!["ls-remote", remote];
        args.extend_from_slice(patterns);
        let text = self.read(&args)?;
        Ok(text
            .lines()
            .filter_map(|line| line.split_once('\t'))
            .map(|(oid, name)| (name.to_owned(), oid.to_owned()))
            .collect())
    }

    /// Store `bytes` as a blob and return its object id.
    pub fn write_blob(&self, bytes: &[u8]) -> Result<String, String> {
        let mut child = self
            .command()
            .args(["hash-object", "-w", "--stdin"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run git hash-object: {e}"))?;
        child
            .stdin
            .take()
            .ok_or("git hash-object has no input")?
            .write_all(bytes)
            .map_err(|e| format!("git hash-object: {e}"))?;
        let out = child
            .wait_with_output()
            .map_err(|e| format!("git hash-object: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "git hash-object failed: {}",
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        Ok(String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    /// Push every update at once or none of them, each guarded by its expected old value.
    pub fn push_atomic(&self, remote: &str, updates: &[Update]) -> Result<PushOutcome, String> {
        let mut args: Vec<String> = vec!["push".into(), "--atomic".into(), "--porcelain".into()];
        for u in updates {
            args.push(format!(
                "--force-with-lease={}:{}",
                u.refname,
                u.expect.as_deref().unwrap_or("")
            ));
        }
        args.push(remote.to_owned());
        for u in updates {
            args.push(format!("{}:{}", u.new.as_deref().unwrap_or(""), u.refname));
        }
        let refs: Vec<&str> = args.iter().map(String::as_str).collect();
        let out = self
            .command()
            .args(&refs)
            .output()
            .map_err(|e| format!("cannot run git push: {e}"))?;
        if out.status.success() {
            return Ok(PushOutcome::Done);
        }
        let report = format!(
            "{}{}",
            String::from_utf8_lossy(&out.stdout),
            String::from_utf8_lossy(&out.stderr)
        );
        if report.contains("stale info")
            || report.contains("fetch first")
            || report.contains("already exists")
        {
            Ok(PushOutcome::Stale)
        } else {
            Err(format!("git push to {remote} failed: {}", report.trim()))
        }
    }

    /// The contents of many objects in one `git cat-file --batch` process, in the order asked.
    /// A missing object is `None`.
    pub fn blobs(&self, oids: &[String]) -> Result<Vec<Option<Vec<u8>>>, String> {
        if oids.is_empty() {
            return Ok(Vec::new());
        }
        let mut child = self
            .command()
            .args(["cat-file", "--batch"])
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| format!("cannot run git cat-file: {e}"))?;
        let mut stdin = child.stdin.take().ok_or("git cat-file has no input")?;
        let request: String = oids.iter().map(|oid| format!("{oid}\n")).collect();
        let writer = std::thread::spawn(move || stdin.write_all(request.as_bytes()));
        let stdout = child.stdout.take().ok_or("git cat-file has no output")?;
        let mut reader = BufReader::new(stdout);
        let mut found = Vec::with_capacity(oids.len());
        for _ in oids {
            let mut header = String::new();
            reader
                .read_line(&mut header)
                .map_err(|e| format!("reading git cat-file: {e}"))?;
            let fields: Vec<&str> = header.split_whitespace().collect();
            if fields.len() == 2 && fields[1] == "missing" {
                found.push(None);
                continue;
            }
            let size: usize = fields
                .get(2)
                .and_then(|size| size.parse().ok())
                .ok_or_else(|| format!("unexpected git cat-file header {header:?}"))?;
            let mut body = vec![0u8; size + 1];
            reader
                .read_exact(&mut body)
                .map_err(|e| format!("reading git cat-file: {e}"))?;
            body.pop();
            found.push(Some(body));
        }
        let _ = writer.join();
        let status = child.wait().map_err(|e| format!("git cat-file: {e}"))?;
        if !status.success() {
            return Err("git cat-file failed".into());
        }
        Ok(found)
    }
}

fn pairs(text: &str) -> Vec<(String, String)> {
    text.lines()
        .filter_map(|line| line.split_once(' '))
        .map(|(name, oid)| (name.to_owned(), oid.to_owned()))
        .collect()
}

/// `git` with the caller's Git routing removed, so a stray `GIT_DIR` or `GIT_INDEX_FILE` can
/// never point a read or a write at another repository, and with no optional locks taken.
fn base_command() -> Command {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("GIT_")
            && upper != "GIT_CONFIG_GLOBAL"
            && upper != "GIT_CONFIG_NOSYSTEM"
            && upper != "GIT_SSH_COMMAND"
        {
            command.env_remove(&name);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

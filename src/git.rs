// SPDX-License-Identifier: Apache-2.0
//! Reading a repository through the `git` program. Every call names the repository explicitly,
//! takes no optional locks and writes nothing: reading the Board or the task queue never changes
//! a checkout or its index.

use std::io::{BufRead, BufReader, Read, Write};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};

pub struct Repo {
    git_dir: PathBuf,
}

impl Repo {
    /// Open a repository: a bare repository, a checkout, or a linked worktree.
    pub fn open(path: &Path) -> Result<Repo, String> {
        let out = base_command()
            .arg("-C")
            .arg(path)
            .args(["rev-parse", "--absolute-git-dir"])
            .output()
            .map_err(|e| format!("cannot run git: {e}"))?;
        if !out.status.success() {
            return Err(format!(
                "{} is not a Git repository: {}",
                path.display(),
                String::from_utf8_lossy(&out.stderr).trim()
            ));
        }
        let dir = String::from_utf8_lossy(&out.stdout).trim().to_owned();
        Ok(Repo {
            git_dir: PathBuf::from(dir),
        })
    }

    fn command(&self) -> Command {
        let mut command = base_command();
        command.arg("--git-dir").arg(&self.git_dir);
        command
    }

    /// Run a read-only git command and return its standard output.
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

    /// `(refname, object id)` for every ref under `prefix`.
    pub fn refs(&self, prefix: &str) -> Result<Vec<(String, String)>, String> {
        let text = self.read(&["for-each-ref", "--format=%(refname) %(objectname)", prefix])?;
        Ok(text
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(name, oid)| (name.to_owned(), oid.to_owned()))
            .collect())
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

/// `git` with the caller's Git routing removed, so a stray `GIT_DIR` or `GIT_INDEX_FILE` can
/// never point a read at another repository, and with no optional locks taken.
fn base_command() -> Command {
    let mut command = Command::new("git");
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("GIT_")
            && upper != "GIT_CONFIG_GLOBAL"
            && upper != "GIT_CONFIG_NOSYSTEM"
        {
            command.env_remove(&name);
        }
    }
    command
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0");
    command
}

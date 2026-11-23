// SPDX-License-Identifier: Apache-2.0
//! Backups after a ship. A clone may name backup repositories (`git config --add susi.backup
//! <url>`); once `ship` has landed `main` on the remote, a detached `susi-tools backup-push`
//! fetches the records from the remote and pushes `main`, release tags and the records to each
//! backup, never forced. The backups stay current from whichever machine ships, with no machine
//! kept on to copy them and no agent waiting on them (2026-11-23: "remove the laptop dependency
//! completely"). Each backup's outcome is a line of `<git dir>/susi-backup.log`.

use crate::git::Repo;
use std::path::Path;
use std::process::{Command, Stdio};

/// What a backup carries besides `main`: release tags, agents' published work, the Board and
/// the finished task records. Claims are leases, mutable by design, and never copied.
const CARRIED: [&str; 5] = [
    "refs/tags/*:refs/tags/*",
    "refs/agents/*:refs/agents/*",
    "refs/board/*:refs/board/*",
    "refs/closed/*:refs/closed/*",
    "refs/abandoned/*:refs/abandoned/*",
];

/// What a public repository's backups carry: its public face, `main` and release tags only.
const CARRIED_PUBLIC: [&str; 1] = ["refs/tags/*:refs/tags/*"];

/// What `repo`'s backups carry besides `main`.
fn carried(public: bool) -> &'static [&'static str] {
    if public {
        &CARRIED_PUBLIC
    } else {
        &CARRIED
    }
}

/// The backup URLs this clone names.
pub fn urls(repo: &Repo) -> Vec<String> {
    repo.try_read(&["config", "--get-all", "susi.backup"])
        .map(|text| {
            text.lines()
                .map(str::trim)
                .filter(|l| !l.is_empty())
                .map(str::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

/// Starts the background push of `head` to every backup; says whether one was started. A failure
/// to start it is reported and never fails the ship, which has already landed.
pub fn spawn(repo: &Repo, remote: &str, head: &str) -> bool {
    if urls(repo).is_empty() {
        return false;
    }
    let started = (|| {
        let exe = std::env::current_exe().map_err(|e| e.to_string())?;
        let log = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(repo.git_dir().join("susi-backup.log"))
            .map_err(|e| e.to_string())?;
        let dir: &Path = repo.work_tree().unwrap_or_else(|_| repo.git_dir());
        Command::new(exe)
            .args(["backup-push", remote, head])
            .current_dir(dir)
            .stdin(Stdio::null())
            .stdout(log.try_clone().map_err(|e| e.to_string())?)
            .stderr(log)
            .spawn()
            .map(drop)
            .map_err(|e| e.to_string())
    })();
    match started {
        Ok(()) => true,
        Err(why) => {
            eprintln!(
                "susi-tools: the backups were not started ({why}); the next ship tries again"
            );
            false
        }
    }
}

/// The detached half: the records from `remote`, then `head` and the records to each backup.
pub fn push(repo: &Repo, remote: &str, head: &str) -> Result<(), String> {
    let carried = carried(crate::githook::is_public(repo));
    let mut fetch = vec!["fetch", "--quiet", "--no-tags", remote];
    fetch.extend(carried);
    let fetched = repo.read(&fetch);
    let main = format!("{head}:refs/heads/main");
    let mut failed = Vec::new();
    for url in urls(repo) {
        let mut args = vec!["push", "--quiet", url.as_str(), main.as_str()];
        args.extend(carried);
        let outcome = repo.read(&args);
        println!(
            "{} backup {url}: {}",
            crate::time::now_unix(),
            match &outcome {
                Ok(_) => "current".to_owned(),
                Err(why) => format!("FAILED: {why}"),
            }
        );
        if outcome.is_err() {
            failed.push(url);
        }
    }
    if let Err(why) = fetched {
        println!(
            "{} the records were not fetched from {remote}: {why}",
            crate::time::now_unix()
        );
    }
    if failed.is_empty() {
        Ok(())
    } else {
        Err(format!("backups not current: {}", failed.join(", ")))
    }
}

#[cfg(test)]
mod tests {
    #[test]
    fn a_public_backup_carries_main_and_release_tags_only() {
        assert_eq!(super::carried(true), ["refs/tags/*:refs/tags/*"]);
        assert!(super::carried(false).contains(&"refs/board/*:refs/board/*"));
        assert!(super::carried(false)
            .iter()
            .all(|spec| !spec.contains("claims")));
    }
}

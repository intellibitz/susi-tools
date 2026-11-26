// SPDX-License-Identifier: Apache-2.0
//! Land: a company host gates the work agents shipped for landing and fast-forwards `main` (the
//! CEO, 2026-11-24: "Company host lands it"). A cloud agent's machine had 4 cores and took 37.6 s
//! for Susi's gates, against 6.6 s on a company host, so its `ship` pushes to
//! `refs/agents/<agent>` (`ship::landing_mode`) and a host runs this, in a clone of its own:
//!
//! - each agent ref ahead of `main` is checked out in the clone's own worktree and its gates run
//!   (`gates::run`, side by side); passing, it is pushed as `main`, a fast-forward, through the
//!   clone's own hooks;
//! - one whose gates fail, or that `main` has moved past, is left where it is, and its agent is
//!   told on the Board, once for each commit;
//! - one run at a time (a lock in the clone's Git folder), so a timer may start it every minute.

use std::path::{Path, PathBuf};

use crate::gates;
use crate::git::Repo;

/// Where the fetched refs are kept in the lander's clone.
const MAIN: &str = "refs/susi-tools/land/main";
const AGENTS: &str = "refs/susi-tools/land/agents/";

/// What happened to one agent ref's commit.
pub enum Outcome {
    Landed {
        agent_ref: String,
        tip: String,
        tasks: Vec<String>,
        seconds: f64,
    },
    Failed {
        agent_ref: String,
        tip: String,
        failed: Vec<(String, String)>,
        seconds: f64,
    },
    Behind {
        agent_ref: String,
        tip: String,
        main: String,
    },
}

impl Outcome {
    /// The agent the ref belongs to.
    pub fn agent(&self) -> &str {
        let agent_ref = match self {
            Outcome::Landed { agent_ref, .. }
            | Outcome::Failed { agent_ref, .. }
            | Outcome::Behind { agent_ref, .. } => agent_ref,
        };
        agent_ref.strip_prefix("refs/agents/").unwrap_or(agent_ref)
    }
}

/// The lock: one run at a time; a lock older than an hour is a run that died, and is taken over.
struct Lock(PathBuf);

impl Lock {
    fn take(git_dir: &Path) -> Result<Option<Lock>, String> {
        let path = git_dir.join("susi-land.lock");
        let stale = std::fs::metadata(&path)
            .and_then(|m| m.modified())
            .ok()
            .and_then(|t| t.elapsed().ok())
            .is_some_and(|age| age.as_secs() > 3600);
        if stale {
            let _ = std::fs::remove_file(&path);
        }
        match std::fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&path)
        {
            Ok(_) => Ok(Some(Lock(path))),
            Err(e) if e.kind() == std::io::ErrorKind::AlreadyExists => Ok(None),
            Err(e) => Err(format!("{}: {e}", path.display())),
        }
    }
}

impl Drop for Lock {
    fn drop(&mut self) {
        let _ = std::fs::remove_file(&self.0);
    }
}

/// Lands what is waiting in `remote`'s agent refs; `None` when another run holds the lock.
pub fn land(repo: &Repo, remote: &str, targets: &[String]) -> Result<Option<Vec<Outcome>>, String> {
    let git_dir = repo.git_dir().to_path_buf();
    let Some(_lock) = Lock::take(&git_dir)? else {
        return Ok(None);
    };
    repo.read(&[
        "fetch",
        "--quiet",
        "--prune",
        remote,
        &format!("+refs/heads/main:{MAIN}"),
        &format!("+refs/agents/*:{AGENTS}*"),
    ])?;
    let mut main = repo.read(&["rev-parse", MAIN])?.trim().to_owned();
    let listing = repo.read(&["for-each-ref", "--format=%(objectname) %(refname)", AGENTS])?;
    let done_path = git_dir.join("susi-land-done");
    let done = std::fs::read_to_string(&done_path).unwrap_or_default();
    let mut outcomes = Vec::new();
    for line in listing.lines() {
        let Some((tip, name)) = line.trim().split_once(' ') else {
            continue;
        };
        let agent_ref = format!("refs/agents/{}", name.trim_start_matches(AGENTS));
        let tip = tip.to_owned();
        let ancestor = |a: &str, b: &str| {
            repo.try_read(&["merge-base", "--is-ancestor", a, b])
                .is_some()
        };
        // Already on main, or already told: nothing to do.
        if ancestor(&tip, &main) || done.lines().any(|l| l.starts_with(&tip)) {
            continue;
        }
        let outcome = if !ancestor(&main, &tip) {
            Outcome::Behind {
                agent_ref,
                tip: tip.clone(),
                main: main.clone(),
            }
        } else {
            let work = checkout(repo, &git_dir, &tip)?;
            let mut gated = gates::run(&work, targets)?;
            if gated.measure_again() {
                gated = gates::run(&work, targets)?;
            }
            if gated.passed() {
                let tasks = crate::ship::trailers(repo, &format!("{main}..{tip}"))?;
                match repo.read(&["push", "--quiet", remote, &format!("{tip}:refs/heads/main")]) {
                    Ok(_) => {
                        main.clone_from(&tip);
                        Outcome::Landed {
                            agent_ref,
                            tip: tip.clone(),
                            tasks,
                            seconds: gated.seconds,
                        }
                    }
                    // Main moved on while the gates ran, or a hook refused: try again next run.
                    Err(why) if why.contains("rejected") || why.contains("non-fast-forward") => {
                        continue
                    }
                    Err(why) => Outcome::Failed {
                        agent_ref,
                        tip: tip.clone(),
                        failed: vec![("push".to_owned(), why)],
                        seconds: gated.seconds,
                    },
                }
            } else {
                // Functional gate failure or wall clock over the 10-second line (refused).
                Outcome::Failed {
                    agent_ref,
                    tip: tip.clone(),
                    failed: gated.failures(),
                    seconds: gated.seconds,
                }
            }
        };
        let word = match &outcome {
            Outcome::Landed { .. } => "landed",
            Outcome::Failed { .. } => "failed",
            Outcome::Behind { .. } => "behind",
        };
        let mut file = std::fs::OpenOptions::new()
            .create(true)
            .append(true)
            .open(&done_path)
            .map_err(|e| format!("{}: {e}", done_path.display()))?;
        std::io::Write::write_all(&mut file, format!("{tip} {word}\n").as_bytes())
            .map_err(|e| format!("{}: {e}", done_path.display()))?;
        outcomes.push(outcome);
    }
    Ok(Some(outcomes))
}

/// The lander's own worktree, at `tip`, clean.
fn checkout(repo: &Repo, git_dir: &Path, tip: &str) -> Result<PathBuf, String> {
    let work = git_dir.join("susi-land-work");
    if work.join(".git").exists() {
        let at = Repo::open(&work)?;
        at.read(&["checkout", "--quiet", "--detach", "--force", tip])?;
        at.read(&["clean", "-fdq"])?;
    } else {
        let _ = repo.read(&["worktree", "prune"]);
        repo.read(&[
            "worktree",
            "add",
            "--quiet",
            "--detach",
            &work.to_string_lossy(),
            tip,
        ])?;
    }
    Ok(work)
}

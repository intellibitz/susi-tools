// SPDX-License-Identifier: Apache-2.0
//! The single rules the Git hooks apply (`hook <stage>`, in `githook.rs`, runs them where a
//! stage needs them). Each refuses only what its rule names, says why and how to proceed, and
//! lets everything else through. The contracts are the earlier program's.
//!
//! - The agent's own root: every agent works under its own root, a folder marked by a
//!   `.susi-agent-root` file naming its agent. A commit inside another agent's root is refused,
//!   and so is a marker that cannot be read; a folder in no root passes (Mandate 77).
//!   `workflow root init --agent <name> [dir]` marks a root (default `~/susi-agents/<name>`).
//! - Freshness: a commit on a checkout more than `susi.maxBehind` (default 20) commits behind
//!   the remote's `main` is refused, when the remote answered (Mandate 74).
//! - Reuse: a commit for a task of size `m` or `l` whose record states no reuse decision is
//!   refused (Mandate 78).
//! - The worktree guard: no commit or merge in a primary checkout (one that is not a linked
//!   worktree), no plain commit on `main` (Mandate 49), and for a commit the root and freshness
//!   checks above.
//!
//! Escape hatches, deliberate and visible, never the default: `SUSI_ALLOW_FOREIGN_ROOT`,
//! `SUSI_ALLOW_STALE`, `SUSI_ALLOW_NO_REUSE`, `SUSI_ALLOW_PRIMARY`, `SUSI_ALLOW_MAIN` (set and
//! not `0`).

use crate::git::Repo;
use crate::json::{self, Json};
use std::path::{Path, PathBuf};

/// The file that marks an agent's root.
pub const MARKER: &str = ".susi-agent-root";
/// The marker's format, as the earlier program wrote and reads it.
pub const MARKER_SCHEMA: &str = "susi-agent-root-v1";
/// Commits a checkout may lag behind the remote's `main` before a commit is refused.
pub const DEFAULT_MAX_BEHIND: u64 = 20;
/// Task records created at or after this time owe a reuse decision; older ones never do.
pub const REUSE_ENFORCED_UNIX: i64 = 1_794_483_000;
/// The reuse decisions a task record may state.
pub const REUSE_DECISIONS: [&str; 4] = ["existing", "harvest", "dependency", "build"];

/// Why a hook check did not pass.
pub enum Stop {
    /// The command was called wrongly.
    Usage(String),
    /// The rule refuses; the text says why and how to proceed.
    Refused(String),
}

impl From<String> for Stop {
    fn from(text: String) -> Stop {
        Stop::Refused(text)
    }
}

/// `workflow root init [--agent <name>] [dir]`; `args` follow `init`.
pub fn root_init_cli(args: &[&str]) -> Result<(), Stop> {
    let mut agent = None;
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--agent" => {
                agent = Some(
                    it.next()
                        .map(|v| (*v).to_owned())
                        .ok_or_else(|| Stop::Usage("--agent needs a value".into()))?,
                )
            }
            other if other.starts_with('-') => {
                return Err(Stop::Usage(format!("root init does not take {other}")))
            }
            other => rest.push(other),
        }
    }
    let agent = agent
        .or_else(|| std::env::var("SUSI_AGENT").ok())
        .filter(|a| !a.trim().is_empty())
        .ok_or_else(|| Stop::Usage("root init needs --agent <name>".into()))?;
    let dir = match rest.as_slice() {
        [] => default_root(&agent)?,
        [dir] => PathBuf::from(dir),
        _ => return Err(Stop::Usage("root init takes at most one folder".into())),
    };
    let dir = init(&dir, &agent, crate::time::now_unix())?;
    println!("{} is the root of agent `{agent}`", dir.display());
    println!("   keep your clones, worktrees, build targets and scratch files under it");
    Ok(())
}

pub fn allowed(var: &str) -> bool {
    std::env::var_os(var).is_some_and(|v| !v.is_empty() && v != "0")
}

/// An agent's root: its folder, its agent and the other names that agent goes by.
pub struct Root {
    pub dir: PathBuf,
    pub agent: String,
    pub aliases: Vec<String>,
}

/// The nearest root that holds `dir`, or `None` when no marker is found above it. A marker that
/// cannot be read is an error, never a pass.
pub fn find_root(dir: &Path) -> Result<Option<Root>, String> {
    let start = dir.canonicalize().unwrap_or_else(|_| dir.to_path_buf());
    for folder in start.ancestors() {
        let marker = folder.join(MARKER);
        if !marker.is_file() {
            continue;
        }
        let text = std::fs::read_to_string(&marker)
            .map_err(|e| format!("cannot read {}: {e}", marker.display()))?;
        let record = json::parse(&text)
            .map_err(|e| format!("{} is not a valid agent-root marker: {e}", marker.display()))?;
        let agent = record.text("agent");
        if agent.trim().is_empty() {
            return Err(format!("{} names no agent", marker.display()));
        }
        return Ok(Some(Root {
            dir: folder.to_path_buf(),
            agent,
            aliases: record.str_list("aliases"),
        }));
    }
    Ok(None)
}

fn same_agent(root: &Root, asking: &str) -> bool {
    let Ok(me) = crate::agent::token(asking) else {
        return false;
    };
    std::iter::once(&root.agent)
        .chain(&root.aliases)
        .filter_map(|name| crate::agent::token(name).ok())
        .any(|token| token == me)
}

/// The root that holds `dir` when it is another agent's than `asking`'s, with who is asking.
/// The agent's own root, a folder in no root, and an agent who does not say who it is pass.
pub fn foreign_root(dir: &Path, asking: Option<&str>) -> Result<Option<(Root, String)>, String> {
    let Some(root) = find_root(dir)? else {
        return Ok(None);
    };
    Ok(match asking.filter(|a| !a.trim().is_empty()) {
        Some(asking) if !same_agent(&root, asking) => Some((root, asking.to_owned())),
        _ => None,
    })
}

pub fn root_refusal(foreign: Option<&(Root, String)>) -> Option<String> {
    let (root, asking) = foreign?;
    if allowed("SUSI_ALLOW_FOREIGN_ROOT") {
        return None;
    }
    Some(format!(
        "❌ Mandate 77: {} is the root of agent `{}`; you are `{asking}`.\n   Work only under your own root (susi-tools workflow root init --agent {asking}); ask `{}` on the Board for anything you need from it.",
        root.dir.display(),
        root.agent,
        root.agent
    ))
}

/// `~/susi-agents/<agent>`.
fn default_root(agent: &str) -> Result<PathBuf, Stop> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .ok_or_else(|| Stop::Usage("no HOME: name the folder".into()))?;
    Ok(PathBuf::from(home).join("susi-agents").join(agent))
}

/// Mark `dir` (made if missing) as the root of `agent`. Marking it again for the same agent
/// changes nothing; a folder that is, or is inside, another agent's root is refused.
pub fn init(dir: &Path, agent: &str, now: i64) -> Result<PathBuf, String> {
    crate::agent::token(agent)?;
    std::fs::create_dir_all(dir).map_err(|e| format!("cannot make {}: {e}", dir.display()))?;
    let dir = dir
        .canonicalize()
        .map_err(|e| format!("cannot resolve {}: {e}", dir.display()))?;
    if let Some(root) = find_root(&dir)? {
        if root.dir == dir && same_agent(&root, agent) {
            return Ok(dir);
        }
        return Err(format!(
            "{} is inside the root of agent `{}` ({}); an agent's root never holds another's",
            dir.display(),
            root.agent,
            root.dir.display()
        ));
    }
    let body = format!(
        "{{\"schema\":{},\"agent\":{},\"aliases\":[],\"created_unix\":{now},\"dir\":{}}}\n",
        json::quote(MARKER_SCHEMA),
        json::quote(agent),
        json::quote(&dir.to_string_lossy())
    );
    std::fs::write(dir.join(MARKER), body)
        .map_err(|e| format!("cannot write {}: {e}", dir.join(MARKER).display()))?;
    Ok(dir)
}

/// Refuse a checkout far behind the remote's `main`; pass when the remote did not answer.
pub fn fresh(repo: &Repo, remote: &str) -> Result<(), Stop> {
    if allowed("SUSI_ALLOW_STALE") {
        return Ok(());
    }
    let limit = repo
        .try_read(&["config", "--get", "susi.maxBehind"])
        .and_then(|v| v.trim().parse::<u64>().ok())
        .unwrap_or(DEFAULT_MAX_BEHIND);
    let Ok(snapshot) = crate::tasks::fetch_main(repo, remote) else {
        return Ok(());
    };
    let range = format!("HEAD..{snapshot}");
    let behind = repo
        .try_read(&["rev-list", "--count", &range])
        .and_then(|n| n.trim().parse::<u64>().ok())
        .unwrap_or(0);
    if behind > limit {
        return Err(Stop::Refused(format!(
            "❌ Mandate 74: this checkout is {behind} commits behind {remote}'s main (a commit is refused beyond {limit}).\n   Catch up first: git merge {snapshot}   (or, on a clean checkout: susi-tools workflow check)"
        )));
    }
    Ok(())
}

/// The task ids named by `Task:` trailers.
pub fn tasks_in_message(message: &str) -> Vec<String> {
    let mut tasks: Vec<String> = message
        .lines()
        .filter_map(|line| line.strip_prefix("Task:"))
        .map(|id| id.trim().to_owned())
        .filter(|id| {
            id.starts_with("T-") && id.chars().all(|c| c.is_ascii_alphanumeric() || c == '-')
        })
        .collect();
    tasks.sort();
    tasks.dedup();
    tasks
}

/// What a task record's `reuse` object is missing, when it owes one.
pub fn reuse_gap(record: &Json) -> Option<&'static str> {
    let created = record
        .get("created_unix")
        .and_then(Json::as_i64)
        .unwrap_or(0);
    if created < REUSE_ENFORCED_UNIX {
        return None;
    }
    let size = record.text("size").to_ascii_lowercase();
    if matches!(size.as_str(), "xs" | "s") {
        return None;
    }
    let Some(reuse) = record.get("reuse") else {
        return Some("it states no reuse decision");
    };
    let stated = |key: &str| !reuse.text(key).trim().is_empty();
    let decision = reuse.text("decision");
    if decision.is_empty() {
        return Some("it states no reuse decision");
    }
    if !REUSE_DECISIONS.contains(&decision.as_str()) {
        return Some("its reuse decision is not one of existing, harvest, dependency, build");
    }
    if !stated("searched") {
        return Some("it does not say where the agent looked (`searched`)");
    }
    match decision.as_str() {
        "harvest" if !stated("source") => {
            Some("its harvest names no `source` (upstream repository and pinned commit)")
        }
        "build" if !stated("reason") => {
            Some("it builds new code without a `reason` nothing existing was suitable")
        }
        _ => None,
    }
}

/// Refuse a commit for a task whose record in this checkout owes a reuse decision. A task with
/// no record here (another repository's queue, or closed) has nothing to check. (A commit that
/// changes only task records never asks: `tasks start` commits a new record before its agent
/// has looked, and commit-msg lets such a commit through first.)
pub fn reuse(work: &Path, tasks: &[String]) -> Result<(), Stop> {
    if allowed("SUSI_ALLOW_NO_REUSE") {
        return Ok(());
    }
    for id in tasks {
        let path = work.join(".agents/tasks").join(format!("{id}.json"));
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        let record =
            json::parse(&text).map_err(|e| format!("{} is not valid JSON: {e}", path.display()))?;
        if let Some(gap) = reuse_gap(&record) {
            return Err(Stop::Refused(format!(
                "❌ Mandate 78: {id} is a task of size {} and {gap}.\n   Look before you build: record in its `reuse` object where you looked (`searched`) and what you decided (`decision`: existing, harvest, dependency or build; a `source` for a harvest, a `reason` for a build), commit the record, then commit again.",
                record.text("size")
            )));
        }
    }
    Ok(())
}

/// Mandate 77: refuse a checkout inside another agent's root, or under a marker that cannot be
/// read.
pub fn own_root(repo: &Repo, agent: Option<&str>) -> Result<(), Stop> {
    let work = repo.work_tree()?;
    let asking = crate::agent::name(agent, repo).ok();
    let foreign = foreign_root(work, asking.as_deref())
        .map_err(|why| Stop::Refused(format!("❌ Mandate 77: {why}")))?;
    match root_refusal(foreign.as_ref()) {
        Some(why) => Err(Stop::Refused(why)),
        None => Ok(()),
    }
}

/// Whether `repo` is a primary checkout: its git directory is the common one, so it is not a
/// linked worktree.
fn primary(repo: &Repo) -> Result<bool, String> {
    let common = repo.read(&["rev-parse", "--path-format=absolute", "--git-common-dir"])?;
    let canon = |p: &Path| p.canonicalize().unwrap_or_else(|_| p.to_path_buf());
    Ok(canon(Path::new(common.trim())) == canon(repo.git_dir()))
}

/// Mandate 49 for a commit or a merge, then the root check, and for a commit the freshness
/// check.
pub fn guard(repo: &Repo, commit: bool, remote: &str, agent: Option<&str>) -> Result<(), Stop> {
    if !allowed("SUSI_ALLOW_PRIMARY") && primary(repo)? {
        return Err(Stop::Refused(
            "❌ Mandate 49: do not commit in the primary checkout.\n   Work in your own linked worktree on your own branch: git worktree add <your root>/wt/<name> -b <you>/<name> origin/main".into(),
        ));
    }
    own_root(repo, agent)?;
    if commit {
        let branch = repo.try_read(&["symbolic-ref", "-q", "--short", "HEAD"]);
        if !allowed("SUSI_ALLOW_MAIN") && branch.as_deref().map(str::trim) == Some("main") {
            return Err(Stop::Refused(
                "❌ Mandate 49: no direct commits on main. Commit on your branch; `susi-tools ship` lands it on main.".into(),
            ));
        }
        fresh(repo, remote)?;
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn record(text: &str) -> Json {
        json::parse(text).unwrap()
    }

    #[test]
    fn reuse_is_owed_only_by_sized_tasks_created_after_the_rule() {
        let old = record(r#"{"created_unix":1,"size":"m"}"#);
        assert_eq!(reuse_gap(&old), None);
        let small = record(r#"{"created_unix":1795000000,"size":"s"}"#);
        assert_eq!(reuse_gap(&small), None);
        let owed = record(r#"{"created_unix":1795000000,"size":"m"}"#);
        assert_eq!(reuse_gap(&owed), Some("it states no reuse decision"));
        let odd = record(
            r#"{"created_unix":1795000000,"size":"l","reuse":{"decision":"guess","searched":"x"}}"#,
        );
        assert!(reuse_gap(&odd).is_some_and(|g| g.contains("not one of")));
        let unlooked =
            record(r#"{"created_unix":1795000000,"size":"m","reuse":{"decision":"existing"}}"#);
        assert!(reuse_gap(&unlooked).is_some_and(|g| g.contains("searched")));
        let build = record(
            r#"{"created_unix":1795000000,"size":"m","reuse":{"decision":"build","searched":"x"}}"#,
        );
        assert!(reuse_gap(&build).is_some_and(|g| g.contains("reason")));
        let harvest = record(
            r#"{"created_unix":1795000000,"size":"m","reuse":{"decision":"harvest","searched":"x"}}"#,
        );
        assert!(reuse_gap(&harvest).is_some_and(|g| g.contains("source")));
        let done = record(
            r#"{"created_unix":1795000000,"size":"m","reuse":{"decision":"build","searched":"x","reason":"y"}}"#,
        );
        assert_eq!(reuse_gap(&done), None);
    }

    #[test]
    fn task_trailers_are_read_from_a_message() {
        let message =
            "subject\n\nbody\n\nTask: T-CLAUDE-7\nTask: T-A-1\nTask: LOCAL-x\nTask: T-A-1\n";
        assert_eq!(tasks_in_message(message), ["T-A-1", "T-CLAUDE-7"]);
    }
}

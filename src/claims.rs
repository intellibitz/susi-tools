// SPDX-License-Identifier: Apache-2.0
//! Claims on tasks. A claim is a compact JSON blob kept at three refs on the remote, changed
//! together in one atomic push, each guarded by the value it had when read:
//! - `refs/claims/<task>`: the claim on that task;
//! - `refs/claim-agents/<AGENT>`: the one claim an agent holds in this repository;
//! - `refs/claim-generation/current`: the latest claim made, a counter every claim moves.
//!
//! The earlier development program, which still answers on the Windows host the commands this
//! one does not have, reads and writes the same formats.

use crate::git::{PushOutcome, Repo, Update};
use crate::json::{self, Json};
use crate::tasks;

pub const GENERATION: &str = "refs/claim-generation/current";
pub const DEFAULT_HOURS: i64 = 1;

#[derive(Clone)]
pub struct Claim {
    pub branch: String,
    pub scopes: Vec<String>,
    pub task: String,
    pub agent: String,
    pub claimed_unix: i64,
    pub lease_until_unix: i64,
}

impl Claim {
    pub fn parse(bytes: &[u8]) -> Option<Claim> {
        let value = json::parse(std::str::from_utf8(bytes).ok()?).ok()?;
        Some(Claim {
            branch: value.text("branch"),
            scopes: value.str_list("scopes"),
            task: value.text("task"),
            agent: value.text("agent"),
            claimed_unix: value.get("claimed_unix").and_then(Json::as_i64)?,
            lease_until_unix: value.get("lease_until_unix").and_then(Json::as_i64)?,
        })
    }

    /// Compact JSON in the field order every claim has, no trailing newline.
    pub fn render(&self) -> Vec<u8> {
        Json::Object(vec![
            ("branch".into(), Json::Str(self.branch.clone())),
            (
                "scopes".into(),
                Json::Array(self.scopes.iter().cloned().map(Json::Str).collect()),
            ),
            ("task".into(), Json::Str(self.task.clone())),
            ("agent".into(), Json::Str(self.agent.clone())),
            (
                "claimed_unix".into(),
                Json::Number(self.claimed_unix.to_string()),
            ),
            (
                "lease_until_unix".into(),
                Json::Number(self.lease_until_unix.to_string()),
            ),
        ])
        .to_compact()
        .into_bytes()
    }

    pub fn live(&self, now: i64) -> bool {
        self.lease_until_unix > now
    }
}

/// One fetch of the remote's `main` and every claim ref into this program's own namespace,
/// made once per run (`Repo::fetch_once`) until this program pushes.
pub fn fetch(repo: &Repo, remote: &str) -> Result<(), String> {
    repo.fetch_once(&[
        "--no-tags",
        "--prune",
        remote,
        &format!("+refs/heads/main:{}", tasks::MAIN_SNAPSHOT),
        "+refs/claims/*:refs/susi-tools/remote/claims/*",
        "+refs/claim-agents/*:refs/susi-tools/remote/claim-agents/*",
        "+refs/claim-generation/*:refs/susi-tools/remote/claim-generation/*",
    ])
}

/// What the remote holds right now: every claim ref with its object id and claim.
pub struct State {
    pub claims: Vec<(String, String, Claim)>,
    pub agents: Vec<(String, String, Claim)>,
    pub generation: Option<String>,
}

impl State {
    /// One fetch brings the remote's `main` and every claim ref into this program's own
    /// namespace; the leases below are the values that fetch saw, so a change made by anyone
    /// since then refuses the push instead of being overwritten.
    pub fn read(repo: &Repo, remote: &str) -> Result<State, String> {
        fetch(repo, remote)?;
        let refs = repo.refs("refs/susi-tools/remote/")?;
        let wanted: Vec<(String, String)> = refs
            .iter()
            .filter(|(name, _)| {
                name.starts_with("refs/susi-tools/remote/claims/")
                    || name.starts_with("refs/susi-tools/remote/claim-agents/")
            })
            .cloned()
            .collect();
        let oids: Vec<String> = wanted.iter().map(|(_, oid)| oid.clone()).collect();
        let blobs = repo.blobs(&oids)?;
        let mut claims = Vec::new();
        let mut agents = Vec::new();
        for ((name, oid), blob) in wanted.into_iter().zip(blobs) {
            let Some(claim) = blob.as_deref().and_then(Claim::parse) else {
                continue;
            };
            if let Some(task) = name.strip_prefix("refs/susi-tools/remote/claims/") {
                claims.push((task.to_owned(), oid, claim));
            } else if let Some(agent) = name.strip_prefix("refs/susi-tools/remote/claim-agents/") {
                agents.push((agent.to_owned(), oid, claim));
            }
        }
        let generation = refs
            .into_iter()
            .find(|(name, _)| name == "refs/susi-tools/remote/claim-generation/current")
            .map(|(_, oid)| oid);
        Ok(State {
            claims,
            agents,
            generation,
        })
    }
    pub fn on_task(&self, task: &str) -> Option<&(String, String, Claim)> {
        self.claims.iter().find(|(t, _, _)| t == task)
    }

    pub fn of_agent(&self, token: &str) -> Option<&(String, String, Claim)> {
        self.agents.iter().find(|(a, _, _)| a == token)
    }
}

/// Two scopes overlap when one is the other or contains it.
fn overlaps(a: &str, b: &str) -> bool {
    let within = |inner: &str, outer: &str| {
        inner == outer
            || inner
                .strip_prefix(outer)
                .is_some_and(|rest| rest.starts_with('/'))
    };
    within(a, b) || within(b, a)
}

fn valid_scope(scope: &str) -> bool {
    !scope.is_empty()
        && !scope.starts_with('/')
        && !scope.contains('\\')
        && scope
            .split('/')
            .all(|part| !part.is_empty() && part != "." && part != "..")
}

pub struct Request<'a> {
    pub task: &'a str,
    pub agent: &'a str,
    pub scopes: &'a [String],
    pub hours: i64,
    pub branch: &'a str,
    /// Where the task's record is, when it is not in this repository's own queue: Susi's
    /// repository, which holds the ecosystem's queue and the Board. The claim itself is always
    /// kept on this repository's `remote`, where the files it covers live, so a task of Susi's
    /// queue can claim files in Susi Tools or Susi Core.
    pub queue: Option<&'a Repo>,
}

/// Claim a task on `remote`. Refused, with nothing changed, when the task is not open on the
/// remote's `main`, a dependency of it is still open, another agent holds it, a live claim of
/// another agent covers one of the scopes, or this agent already holds a live claim on another
/// task. Claiming a task this agent already holds renews it with the new scopes.
pub fn claim(repo: &Repo, remote: &str, req: &Request<'_>) -> Result<Claim, String> {
    let token = crate::agent::token(req.agent)?;
    if req.scopes.is_empty() {
        return Err(
            "name the files or directories you will change: --scope <path> (repeatable)".into(),
        );
    }
    if let Some(bad) = req.scopes.iter().find(|s| !valid_scope(s)) {
        return Err(format!(
            "scope {bad:?} must be a normalized repository-relative path"
        ));
    }
    if !(1..=24).contains(&req.hours) {
        return Err("a lease is 1 to 24 hours".into());
    }
    let now = crate::time::now_unix();
    let mut state = State::read(repo, remote)?;
    let own = tasks::open_records(repo, tasks::MAIN_SNAPSHOT)?;
    let open = match req.queue {
        Some(queue) if !own.iter().any(|t| t.id == req.task) => {
            tasks::fetch_main(queue, "origin")?;
            let theirs = tasks::open_records(queue, tasks::MAIN_SNAPSHOT)?;
            if theirs.iter().any(|t| t.id == req.task) {
                theirs
            } else {
                own
            }
        }
        _ => own,
    };
    let task = open.iter().find(|t| t.id == req.task).ok_or_else(|| {
        format!(
            "{} is not an open task on {remote}'s main, nor in Susi's queue",
            req.task
        )
    })?;
    if let Some(dep) = task
        .record
        .str_list("deps")
        .into_iter()
        .find(|d| open.iter().any(|t| &t.id == d))
    {
        return Err(format!("{} waits on {dep}, which is still open", req.task));
    }
    for attempt in 0..6u32 {
        if attempt > 0 {
            state = State::read(repo, remote)?;
        }
        let current = state.on_task(req.task);
        if let Some((_, _, held)) = current {
            if held.live(now) && held.agent != token {
                return Err(format!(
                    "{} is held by {} until unix {}",
                    req.task, held.agent, held.lease_until_unix
                ));
            }
        }
        if let Some((_, _, mine)) = state.of_agent(&token) {
            let still_open = open.iter().any(|t| t.id == mine.task);
            if mine.live(now) && mine.task != req.task && still_open {
                return Err(format!(
                    "{token} already holds {} until unix {}: release it first (one live claim per agent)",
                    mine.task, mine.lease_until_unix
                ));
            }
        }
        for (other_task, _, other) in &state.claims {
            if other_task == req.task || other.agent == token || !other.live(now) {
                continue;
            }
            if !open.iter().any(|t| &t.id == other_task) {
                continue;
            }
            for mine in req.scopes {
                if let Some(theirs) = other.scopes.iter().find(|s| overlaps(mine, s)) {
                    return Err(format!(
                        "scope {mine} overlaps {theirs}, claimed by {} for {other_task}",
                        other.agent
                    ));
                }
            }
        }
        let claimed_unix = match current {
            Some((_, _, held)) if held.agent == token && held.live(now) => held.claimed_unix,
            _ => now,
        };
        let claim = Claim {
            branch: req.branch.to_owned(),
            scopes: req.scopes.to_vec(),
            task: req.task.to_owned(),
            agent: token.clone(),
            claimed_unix,
            // Renewing never shortens a lease this agent already holds.
            lease_until_unix: match current {
                Some((_, _, held)) if held.agent == token && held.live(now) => {
                    held.lease_until_unix.max(now + req.hours * 3600)
                }
                _ => now + req.hours * 3600,
            },
        };
        let oid = repo.write_blob(&claim.render())?;
        let updates = vec![
            Update {
                refname: format!("refs/claims/{}", req.task),
                new: Some(oid.clone()),
                expect: current.map(|(_, o, _)| o.clone()),
            },
            Update {
                refname: format!("refs/claim-agents/{token}"),
                new: Some(oid.clone()),
                expect: state.of_agent(&token).map(|(_, o, _)| o.clone()),
            },
            Update {
                refname: GENERATION.into(),
                new: Some(oid),
                expect: state.generation.clone(),
            },
        ];
        match repo.push_atomic(remote, &updates)? {
            PushOutcome::Done => return Ok(claim),
            PushOutcome::Stale => {
                std::thread::sleep(std::time::Duration::from_millis(
                    50 + u64::from(attempt) * 100,
                ));
            }
        }
    }
    Err("the claims kept changing while this claim was made; nothing was claimed".into())
}

/// Give up this agent's claim on a task: its two refs leave the remote in one push.
pub fn release(repo: &Repo, remote: &str, task: &str, agent: &str) -> Result<bool, String> {
    let token = crate::agent::token(agent)?;
    for attempt in 0..6u32 {
        let state = State::read(repo, remote)?;
        let Some((_, oid, held)) = state.on_task(task) else {
            return Ok(false);
        };
        if held.agent != token {
            return Err(format!(
                "{task} is claimed by {}, not by {token}",
                held.agent
            ));
        }
        let mut updates = vec![Update {
            refname: format!("refs/claims/{task}"),
            new: None,
            expect: Some(oid.clone()),
        }];
        if let Some((_, agent_oid, agent_claim)) = state.of_agent(&token) {
            if agent_claim.task == task {
                updates.push(Update {
                    refname: format!("refs/claim-agents/{token}"),
                    new: None,
                    expect: Some(agent_oid.clone()),
                });
            }
        }
        match repo.push_atomic(remote, &updates)? {
            PushOutcome::Done => return Ok(true),
            PushOutcome::Stale => {
                std::thread::sleep(std::time::Duration::from_millis(
                    50 + u64::from(attempt) * 100,
                ));
            }
        }
    }
    Err("the claims kept changing; nothing was released".into())
}

/// Extend this agent's live claim on `task` (or its only live claim) by `hours` from now.
/// `queue`: Susi's repository, for a claim here on a task of Susi's queue (see `Request`).
pub fn renew(
    repo: &Repo,
    remote: &str,
    agent: &str,
    hours: i64,
    queue: Option<&Repo>,
) -> Result<Option<Claim>, String> {
    let token = crate::agent::token(agent)?;
    let state = State::read(repo, remote)?;
    let now = crate::time::now_unix();
    let Some((_, _, mine)) = state.of_agent(&token).filter(|(_, _, c)| c.live(now)) else {
        return Ok(None);
    };
    let request = Request {
        task: &mine.task,
        agent,
        scopes: &mine.scopes,
        hours,
        branch: &mine.branch,
        queue,
    };
    claim(repo, remote, &request).map(Some)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scopes_overlap_by_whole_path_parts() {
        assert!(overlaps("docs", "docs/plan.md"));
        assert!(overlaps("docs/plan.md", "docs"));
        assert!(overlaps("src", "src"));
        assert!(!overlaps("doc", "docs"));
        assert!(!overlaps("src/a.rs", "src/b.rs"));
    }

    #[test]
    fn a_claim_renders_in_its_fixed_field_order() {
        let claim = Claim {
            branch: "b".into(),
            scopes: vec!["src".into()],
            task: "T-A-1".into(),
            agent: "ALPHA".into(),
            claimed_unix: 1,
            lease_until_unix: 3601,
        };
        assert_eq!(
            String::from_utf8(claim.render()).unwrap(),
            r#"{"branch":"b","scopes":["src"],"task":"T-A-1","agent":"ALPHA","claimed_unix":1,"lease_until_unix":3601}"#
        );
        assert!(Claim::parse(&claim.render()).is_some());
    }

    #[test]
    fn scopes_must_be_plain_relative_paths() {
        for bad in ["", "/abs", "a/../b", "a//b", "./a", "a\\b"] {
            assert!(!valid_scope(bad), "{bad:?}");
        }
        assert!(valid_scope("crates/susi-tools/src"));
    }
}

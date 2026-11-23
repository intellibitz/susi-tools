// SPDX-License-Identifier: Apache-2.0
//! Ship: publish a checkout's committed work to the remote's `main` as a fast-forward, then say
//! so on the Board. Nothing is forced, merged or rewritten here: a `main` that has moved on is
//! merged by the agent, who resolves what the merge needs, and then ships again.
//!
//! In a clone set for landing (`git config susi.land host`: a cloud agent's, whose machine is too
//! small for the gates inside the 10-second line) the same work goes to `refs/agents/<agent>`
//! instead, for a company host to gate and land (`land`, the CEO, 2026-11-24: "Company host
//! lands it").

use crate::git::Repo;

pub enum Shipped {
    Nothing(String),
    FastForward {
        from: String,
        to: String,
        tasks: Vec<String>,
    },
    /// Pushed to `refs/agents/<agent>` for a company host to land.
    ForLanding {
        main: String,
        to: String,
        agent_ref: String,
        tasks: Vec<String>,
    },
}

/// Whether this clone ships for landing (`git config susi.land host`).
pub fn landing_mode(repo: &Repo) -> bool {
    repo.try_read(&["config", "--get", "susi.land"])
        .is_some_and(|v| v.trim() == "host")
}

/// The ref an agent's work waits in for landing: `refs/agents/<agent>`, one segment.
pub fn agent_ref(agent: &str) -> Result<String, String> {
    let name: String = agent
        .trim()
        .to_ascii_lowercase()
        .chars()
        .filter(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.'))
        .collect();
    if name.is_empty() || name.starts_with('.') {
        return Err(format!("`{agent}` names no agent ref"));
    }
    Ok(format!("refs/agents/{name}"))
}

pub fn ship(repo: &Repo, remote: &str, agent: &str) -> Result<Shipped, String> {
    repo.work_tree()?;
    let dirty = repo.read(&["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.trim().is_empty() {
        return Err("commit or set aside your changes first: only committed work ships".into());
    }
    let main = crate::tasks::fetch_main(repo, remote)?;
    // HEAD and the remote's main from one `rev-parse`, and where they meet from one `merge-base`:
    // HEAD already on main, a fast-forward, or main moved on.
    let ids = repo.read(&["rev-parse", "HEAD", &main])?;
    let mut ids = ids.lines().map(str::trim);
    let (Some(head), Some(tip)) = (ids.next(), ids.next()) else {
        return Err("git rev-parse did not name HEAD and main".into());
    };
    let (head, tip) = (head.to_owned(), tip.to_owned());
    let base = repo.read(&["merge-base", &head, &tip]).unwrap_or_default();
    let base = base.trim();
    if base == head {
        return Ok(Shipped::Nothing(tip));
    }
    if base != tip {
        return Err(format!(
            "{remote}'s main has moved on: run `git merge {main}` (resolve, commit), then ship again"
        ));
    }
    let tasks = trailers(repo, &format!("{tip}..{head}"))?;
    if landing_mode(repo) {
        let agent_ref = agent_ref(agent)?;
        repo.read(&["push", "--quiet", remote, &format!("{head}:{agent_ref}")])?;
        return Ok(Shipped::ForLanding {
            main: tip,
            to: head,
            agent_ref,
            tasks,
        });
    }
    repo.read(&[
        "push",
        "--quiet",
        remote,
        &format!("{head}:refs/heads/main"),
    ])?;
    repo.read(&["update-ref", crate::tasks::MAIN_SNAPSHOT, &head])?;
    Ok(Shipped::FastForward {
        from: tip,
        to: head,
        tasks,
    })
}

/// The `Task:` trailers of the commits in `range`, in order, once each.
pub fn trailers(repo: &Repo, range: &str) -> Result<Vec<String>, String> {
    let text = repo.read(&[
        "log",
        "--reverse",
        "--format=%(trailers:key=Task,valueonly)",
        range,
    ])?;
    let mut out: Vec<String> = Vec::new();
    for line in text.lines().map(str::trim).filter(|l| !l.is_empty()) {
        if !out.iter().any(|t| t == line) {
            out.push(line.to_owned());
        }
    }
    Ok(out)
}

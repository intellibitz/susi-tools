// SPDX-License-Identifier: Apache-2.0
//! The session-start check: bring a clean checkout up to the remote's `main`, show what on the
//! Board is addressed to you since you last posted, and say what you hold. It changes nothing
//! but a fast-forward of a clean checkout.

use crate::board;
use crate::git::Repo;

const BOARD_SNAPSHOT: &str = "refs/susi-tools/remote/board/";
const LOOK_BACK: usize = 60;

/// The newest `limit` items of the remote's Board, oldest first, fetched by name only. An item
/// that is not a Board item (not JSON, or of another schema) is not shown.
pub fn recent_items(
    repo: &Repo,
    remote: &str,
    limit: usize,
) -> Result<(Vec<board::Item>, u64), String> {
    let mut numbered: Vec<(u64, String)> = repo
        .remote_refs(remote, &["refs/board/items/*"])?
        .into_iter()
        .filter_map(|(name, oid)| {
            name.strip_prefix(board::ITEMS)
                .and_then(|n| n.parse::<u64>().ok())
                .map(|n| (n, oid))
        })
        .collect();
    numbered.sort_by_key(|(n, _)| *n);
    let last = numbered.last().map_or(0, |(n, _)| *n);
    let chosen = &numbered[numbered.len().saturating_sub(limit)..];
    if chosen.is_empty() {
        return Ok((Vec::new(), 0));
    }
    let mut args: Vec<String> = vec![
        "fetch".into(),
        "--quiet".into(),
        "--no-tags".into(),
        remote.into(),
    ];
    for (n, _) in chosen {
        args.push(format!("+{}{n}:{BOARD_SNAPSHOT}{n}", board::ITEMS));
    }
    let refs: Vec<&str> = args.iter().map(String::as_str).collect();
    repo.read(&refs)?;
    let oids: Vec<String> = chosen.iter().map(|(_, oid)| oid.clone()).collect();
    let blobs = repo.blobs(&oids)?;
    let mut items = Vec::new();
    for ((n, _), blob) in chosen.iter().zip(blobs) {
        let Some(body) = blob
            .and_then(|b| String::from_utf8(b).ok())
            .and_then(|t| crate::json::parse(&t).ok())
            .filter(|body| body.text("schema") == board::SCHEMA)
        else {
            continue;
        };
        items.push(board::Item { n: *n, body });
    }
    Ok((items, last))
}

/// Requests addressed to `agent` (or to all) that `agent` has not answered: no later item of
/// theirs names the request with `re`.
pub fn addressed<'a>(items: &'a [board::Item], agent: &str) -> Vec<&'a board::Item> {
    let me = agent.to_ascii_lowercase();
    let token = crate::agent::token(agent).unwrap_or_default();
    let mine = |item: &board::Item| {
        let who = item.body.text("agent");
        who.eq_ignore_ascii_case(&me) || who == token
    };
    items
        .iter()
        .filter(|item| {
            let to = item.body.text("to").to_ascii_lowercase();
            item.body.text("kind") == "request"
                && !mine(item)
                && (to == me || to == token.to_ascii_lowercase() || to == "all")
                && !items
                    .iter()
                    .any(|later| mine(later) && later.body.text("re") == item.n.to_string())
        })
        .collect()
}
pub fn board_check(repo: &Repo, remote: &str, agent: &str) -> Result<(), String> {
    let (items, last) = recent_items(repo, remote, LOOK_BACK)?;
    println!("board on `{remote}`: last item is {last}; you are `{agent}`");
    let waiting = addressed(&items, agent);
    if waiting.is_empty() {
        println!("nothing is waiting for your answer");
    } else {
        println!("waiting for your answer:");
        for item in waiting {
            println!("{}", board::line(item));
        }
    }
    Ok(())
}

pub fn workflow_check(repo: &Repo, remote: &str, board: &Repo, agent: &str) -> Result<(), String> {
    let main = crate::tasks::fetch_main(repo, remote)?;
    let head = repo.read(&["rev-parse", "HEAD"])?.trim().to_owned();
    let tip = repo.read(&["rev-parse", &main])?.trim().to_owned();
    let behind = repo
        .read(&["rev-list", "--count", &format!("{head}..{tip}")])?
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    let ahead = repo
        .read(&["rev-list", "--count", &format!("{tip}..{head}")])?
        .trim()
        .parse::<u64>()
        .unwrap_or(0);
    let dirty = !repo
        .read(&["status", "--porcelain", "--untracked-files=no"])?
        .trim()
        .is_empty();
    if behind == 0 {
        println!("up to date with {remote}'s main ({})", &tip[..12]);
    } else if ahead == 0 && !dirty {
        repo.read(&["merge", "--ff-only", "--quiet", &main])?;
        println!(
            "fast-forwarded {behind} commit(s) to {remote}'s main ({})",
            &tip[..12]
        );
    } else if dirty {
        println!(
            "{behind} commit(s) behind {remote}'s main; not merged: you have uncommitted changes"
        );
    } else {
        println!("{behind} commit(s) behind and {ahead} ahead of {remote}'s main: merge it (`git merge {main}`)");
    }
    if ahead > 0 {
        println!("{ahead} commit(s) of yours are not on main yet: ship them (`susi-tools ship`)");
    }
    board_check(board, "origin", agent)?;
    let state = crate::claims::State::read(repo, remote)?;
    let token = crate::agent::token(agent)?;
    let now = crate::time::now_unix();
    match state.of_agent(&token) {
        Some((_, _, c)) if c.live(now) => println!(
            "you hold {} until unix {} (scopes: {})",
            c.task,
            c.lease_until_unix,
            c.scopes.join(", ")
        ),
        _ => println!("you hold no live claim"),
    }
    Ok(())
}

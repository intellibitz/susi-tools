// SPDX-License-Identifier: Apache-2.0
//! The session-start check: bring a clean checkout up to the remote's `main`, show unanswered
//! requests addressed to you, the Board items addressed to you that this clone has not shown
//! you yet, and what you hold. Ship prints the same after it posts Landed. It changes nothing but
//! a fast-forward of a clean checkout and the number of the last item shown, in the clone's
//! config (`susi.boardshown-<you>`).

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
    let numbered = board::numbered(repo.remote_refs(remote, &["refs/board/items/*"])?);
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

fn by_agent(item: &board::Item, me: &str, token: &str) -> bool {
    let who = item.body.text("agent");
    who.eq_ignore_ascii_case(me) || who == token
}

fn to_agent(item: &board::Item, me: &str, token_lc: &str) -> bool {
    let to = item.body.text("to").to_ascii_lowercase();
    to == me || to == token_lc
}

/// Requests addressed to `agent` (or to all) that `agent` has not answered: no later item of
/// theirs names the request with `re`.
pub fn addressed<'a>(items: &'a [board::Item], agent: &str) -> Vec<&'a board::Item> {
    let me = agent.to_ascii_lowercase();
    let token = crate::agent::token(agent).unwrap_or_default();
    let token_lc = token.to_ascii_lowercase();
    items
        .iter()
        .filter(|item| {
            item.body.text("kind") == "request"
                && !by_agent(item, &me, &token)
                && (to_agent(item, &me, &token_lc)
                    || item.body.text("to").eq_ignore_ascii_case("all"))
                && !items.iter().any(|later| {
                    by_agent(later, &me, &token) && later.body.text("re") == item.n.to_string()
                })
        })
        .collect()
}

/// The config key under which a clone keeps the number of the last Board item it showed
/// `agent`, so each item addressed to the agent is shown once, whatever the agent posts in
/// between. Counting from the agent's last own item instead let a claim it made after a
/// heads-up hide that heads-up from its next ship (Codex, Board 5149, 2026-11-24).
fn shown_key(agent: &str) -> String {
    let token = crate::agent::token(agent).unwrap_or_default();
    format!("susi.boardshown-{}", token.to_ascii_lowercase())
}

/// Items whose `to` is `agent` (name or token; any kind) after `cutoff`, but not `own` (the
/// Landed a ship just posted).
fn for_you<'a>(
    items: &'a [board::Item],
    agent: &str,
    cutoff: u64,
    own: Option<u64>,
) -> Vec<&'a board::Item> {
    let me = agent.to_ascii_lowercase();
    let token_lc = crate::agent::token(agent)
        .unwrap_or_default()
        .to_ascii_lowercase();
    items
        .iter()
        .filter(|item| to_agent(item, &me, &token_lc) && item.n > cutoff && Some(item.n) != own)
        .collect()
}

/// Prints what the Board addressed to `agent` that this clone has not shown it yet, then
/// remembers `last`, the newest item read. The first time, it counts from the agent's last
/// Landed item before `own`.
fn show_for_you(
    repo: &Repo,
    items: &[board::Item],
    last: u64,
    agent: &str,
    own: Option<u64>,
) -> Result<(), String> {
    let key = shown_key(agent);
    let shown = repo
        .try_read(&["config", "--get", &key])
        .and_then(|n| n.trim().parse::<u64>().ok());
    let me = agent.to_ascii_lowercase();
    let token = crate::agent::token(agent).unwrap_or_default();
    let cutoff = shown.unwrap_or_else(|| {
        items
            .iter()
            .rev()
            .find(|item| {
                by_agent(item, &me, &token)
                    && item.body.text("kind") == "landed"
                    && Some(item.n) != own
            })
            .map_or(0, |item| item.n)
    });
    let new = for_you(items, agent, cutoff, own);
    if !new.is_empty() {
        println!("for you since your last item:");
        for item in new {
            println!("{}", board::line(item));
        }
    }
    if last > cutoff {
        repo.read(&["config", "--local", &key, &last.to_string()])?;
    }
    Ok(())
}

/// After ship posts Landed: what the Board addressed to `agent` that this clone has not shown it.
pub fn print_for_you_since_last(repo: &Repo, remote: &str, agent: &str) -> Result<(), String> {
    let (items, last) = recent_items(repo, remote, LOOK_BACK)?;
    let me = agent.to_ascii_lowercase();
    let token = crate::agent::token(agent).unwrap_or_default();
    let landed = items
        .iter()
        .rev()
        .find(|item| by_agent(item, &me, &token) && item.body.text("kind") == "landed")
        .map(|item| item.n);
    show_for_you(repo, &items, last, agent, landed)
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
    show_for_you(repo, &items, last, agent, None)
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

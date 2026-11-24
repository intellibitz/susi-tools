// SPDX-License-Identifier: Apache-2.0
//! The session-start check: bring a clean checkout up to the remote's `main`, show unanswered
//! requests addressed to you, Board items addressed to you since you last posted, and what you
//! hold. Ship prints the same since-last list after it posts Landed. It changes nothing but a
//! fast-forward of a clean checkout.

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

/// Items whose `to` is `agent` (name or token; any kind), posted after `agent`'s last own item.
/// `before` is an own item to ignore as the cutoff (the Landed ship just posted).
fn for_you_since_last<'a>(
    items: &'a [board::Item],
    agent: &str,
    before: Option<u64>,
) -> Vec<&'a board::Item> {
    let me = agent.to_ascii_lowercase();
    let token = crate::agent::token(agent).unwrap_or_default();
    let token_lc = token.to_ascii_lowercase();
    let last_own = items
        .iter()
        .rev()
        .find(|item| by_agent(item, &me, &token) && before.is_none_or(|n| item.n < n))
        .map(|item| item.n);
    items
        .iter()
        .filter(|item| {
            to_agent(item, &me, &token_lc)
                && last_own.is_none_or(|n| item.n > n)
                && before.is_none_or(|n| item.n < n)
        })
        .collect()
}

fn print_for_you(items: &[board::Item], agent: &str, before: Option<u64>) {
    let for_you = for_you_since_last(items, agent, before);
    if for_you.is_empty() {
        return;
    }
    println!("for you since your last item:");
    for item in for_you {
        println!("{}", board::line(item));
    }
}

/// After ship posts Landed: items addressed to `agent` since the own item before that Landed.
pub fn print_for_you_since_last(repo: &Repo, remote: &str, agent: &str) -> Result<(), String> {
    let (items, _) = recent_items(repo, remote, LOOK_BACK)?;
    let me = agent.to_ascii_lowercase();
    let token = crate::agent::token(agent).unwrap_or_default();
    let landed = items
        .iter()
        .rev()
        .find(|item| by_agent(item, &me, &token))
        .map(|item| item.n);
    print_for_you(&items, agent, landed);
    Ok(())
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
    print_for_you(&items, agent, None);
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

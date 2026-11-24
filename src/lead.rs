// SPDX-License-Identifier: Apache-2.0
//! The lead: a role, not a vendor (V-49, Susi's `docs/bricks.md`). Whichever agent the CEO is
//! talking to leads that session, holding the lead's claim, one ref on the Board's repository
//! (`refs/lead/current`), so two sessions never both lead. `susi-tools lead` gives a lead its
//! bearings in one command: who leads, the brick map (`.agents/bricks.json` on main) with each
//! brick's rung, next step and tasks, the foundation's tasks (for the whole, no one brick's),
//! what waits on the CEO, the visions placed nowhere, the open tasks the map names nowhere, and
//! what the Board addressed to it.

use crate::git::{PushOutcome, Repo, Update};
use crate::json::{self, Json};

/// The lead's claim on the Board's repository.
pub const LEAD_REF: &str = "refs/lead/current";
const LEAD_SNAPSHOT: &str = "refs/susi-tools/remote/lead";
/// A lead's claim lasts a working session unless asked otherwise.
pub const DEFAULT_HOURS: i64 = 8;
const BRICKS: &str = ".agents/bricks.json";
const VISIONS: &str = ".agents/visions/";

/// Who leads, until when.
pub struct Lead {
    pub agent: String,
    pub since_unix: i64,
    pub until_unix: i64,
}

impl Lead {
    pub fn live(&self, now: i64) -> bool {
        self.until_unix > now
    }

    fn render(&self) -> String {
        let mut text = Json::Object(vec![
            ("schema".into(), Json::Str("susi.lead/v1".into())),
            ("agent".into(), Json::Str(self.agent.clone())),
            (
                "since_unix".into(),
                Json::Number(self.since_unix.to_string()),
            ),
            (
                "until_unix".into(),
                Json::Number(self.until_unix.to_string()),
            ),
        ])
        .to_pretty();
        text.push('\n');
        text
    }
}

/// The lead's claim as the remote has it now, and the object it is.
pub fn current(board: &Repo, remote: &str) -> Result<Option<(String, Lead)>, String> {
    let Some((_, oid)) = board
        .remote_refs(remote, &[LEAD_REF])?
        .into_iter()
        .find(|(name, _)| name == LEAD_REF)
    else {
        return Ok(None);
    };
    board.read(&[
        "fetch",
        "--quiet",
        "--no-tags",
        remote,
        &format!("+{LEAD_REF}:{LEAD_SNAPSHOT}"),
    ])?;
    let blob = board
        .blobs(std::slice::from_ref(&oid))?
        .into_iter()
        .next()
        .flatten()
        .ok_or("the lead's claim is missing its object")?;
    let text = String::from_utf8(blob).map_err(|_| "the lead's claim is not UTF-8")?;
    let body = json::parse(&text).map_err(|e| format!("the lead's claim is not JSON: {e}"))?;
    let number = |key: &str| body.get(key).and_then(Json::as_i64).unwrap_or(0);
    Ok(Some((
        oid,
        Lead {
            agent: body.text("agent"),
            since_unix: number("since_unix"),
            until_unix: number("until_unix"),
        },
    )))
}

/// What a claim did: led already (renewed), led from nobody, or took it from another agent.
pub enum Claimed {
    Renewed(Lead),
    Took { lead: Lead, from: Option<String> },
}

/// Take the lead for `hours`: refused while another agent's claim is live, unless `take` (the CEO
/// is talking to this agent now, so it leads; the one it took from is told on the Board).
pub fn claim(
    board: &Repo,
    remote: &str,
    agent: &str,
    hours: i64,
    take: bool,
) -> Result<Claimed, String> {
    if !(1..=24).contains(&hours) {
        return Err("a lead's claim lasts 1 to 24 hours".into());
    }
    let token = crate::agent::token(agent)?;
    for attempt in 0..6u32 {
        let now = crate::time::now_unix();
        let held = current(board, remote)?;
        let (renewing, from) = match &held {
            Some((_, lead)) if lead.agent == token && lead.live(now) => (true, None),
            Some((_, lead)) if lead.live(now) && !take => {
                return Err(format!(
                    "{} leads until unix {}: the agent the CEO is talking to takes the lead \
                     with `susi-tools lead claim --take`, and the Board tells {}",
                    lead.agent, lead.until_unix, lead.agent
                ));
            }
            Some((_, lead)) if lead.live(now) => (false, Some(lead.agent.clone())),
            _ => (false, None),
        };
        let lead = Lead {
            agent: token.clone(),
            since_unix: match &held {
                Some((_, lead)) if renewing => lead.since_unix,
                _ => now,
            },
            until_unix: now + hours * 3600,
        };
        let oid = board.write_blob(lead.render().as_bytes())?;
        let update = Update {
            refname: LEAD_REF.into(),
            new: Some(oid),
            expect: held.as_ref().map(|(oid, _)| oid.clone()),
        };
        match board.push_atomic(remote, &[update])? {
            PushOutcome::Done => {
                return Ok(if renewing {
                    Claimed::Renewed(lead)
                } else {
                    Claimed::Took { lead, from }
                })
            }
            PushOutcome::Stale => std::thread::sleep(std::time::Duration::from_millis(
                50 + u64::from(attempt) * 100,
            )),
        }
    }
    Err("the lead's claim kept changing while it was taken; nothing was claimed".into())
}

/// Give up the lead: only the agent that holds it.
pub fn release(board: &Repo, remote: &str, agent: &str) -> Result<bool, String> {
    let token = crate::agent::token(agent)?;
    let Some((oid, lead)) = current(board, remote)? else {
        return Ok(false);
    };
    if lead.agent != token {
        return Err(format!("{} leads, not {token}", lead.agent));
    }
    let update = Update {
        refname: LEAD_REF.into(),
        new: None,
        expect: Some(oid),
    };
    match board.push_atomic(remote, &[update])? {
        PushOutcome::Done => Ok(true),
        PushOutcome::Stale => Err("the lead's claim changed meanwhile; it was not released".into()),
    }
}

fn strings(value: Option<&Json>) -> Vec<String> {
    match value {
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// The lead's bearings, as text: who leads, the brick map, what waits on the CEO, the visions
/// placed nowhere and the open tasks no brick names.
pub fn bearings(board: &Repo, remote: &str, agent: &str) -> Result<String, String> {
    let token = crate::agent::token(agent)?;
    let now = crate::time::now_unix();
    let mut out = String::new();
    match current(board, remote)? {
        Some((_, lead)) if lead.live(now) => out.push_str(&format!(
            "lead: {} leads until unix {}{}\n",
            lead.agent,
            lead.until_unix,
            if lead.agent == token {
                " (you)"
            } else {
                "; the agent the CEO is talking to takes it: susi-tools lead claim --take"
            }
        )),
        _ => out.push_str(
            "lead: nobody leads; the agent the CEO is talking to takes it: susi-tools lead claim\n",
        ),
    }
    let main = crate::tasks::fetch_main(board, remote)?;
    let open = crate::tasks::open_records(board, &main)?;
    let claims = crate::claims::State::read(board, remote)?;
    let Some(map_text) = board.try_read(&["cat-file", "blob", &format!("{main}:{BRICKS}")]) else {
        out.push_str(&format!(
            "no brick map on main ({BRICKS}): the lead writes one (Susi's docs/bricks.md)\n"
        ));
        return Ok(out);
    };
    let map = json::parse(&map_text).map_err(|e| format!("{BRICKS} is not JSON: {e}"))?;
    let bricks = match map.get("bricks") {
        Some(Json::Array(items)) => items.clone(),
        _ => Vec::new(),
    };
    let mut named_tasks: Vec<String> = Vec::new();
    let mut placed: Vec<String> = strings(map.get("rules"));
    let mut waits = Vec::new();
    out.push_str("bricks (.agents/bricks.json on main):\n");
    for brick in &bricks {
        let name = brick.text("name");
        let next = brick.text("next");
        out.push_str(&format!(
            "  {:<16} {:<9} {}\n",
            name,
            brick.text("rung"),
            if next.is_empty() { "-" } else { next.as_str() }
        ));
        let tasks = strings(brick.get("tasks"));
        if !tasks.is_empty() {
            let shown: Vec<String> = tasks
                .iter()
                .map(|id| {
                    if !open.iter().any(|t| &t.id == id) {
                        format!("{id} (closed)")
                    } else {
                        match claims.on_task(id) {
                            Some((_, _, c)) if c.live(now) => format!("{id} (held by {})", c.agent),
                            _ => format!("{id} (open)"),
                        }
                    }
                })
                .collect();
            out.push_str(&format!("  {:<16} tasks: {}\n", "", shown.join(", ")));
        }
        let wait = brick.text("waits_on_ceo");
        if !wait.is_empty() {
            waits.push(format!("  {name}: {wait}"));
        }
        named_tasks.extend(tasks);
        placed.extend(strings(brick.get("visions")));
    }
    // Tasks for the whole, no one brick's: the toolchain, a rename, the tooling, the documents.
    let foundation = strings(map.get("foundation"));
    if !foundation.is_empty() {
        let shown: Vec<String> = foundation
            .iter()
            .map(|id| {
                if open.iter().any(|t| &t.id == id) {
                    format!("{id} (open)")
                } else {
                    format!("{id} (closed)")
                }
            })
            .collect();
        out.push_str(&format!("foundation: {}\n", shown.join(", ")));
    }
    named_tasks.extend(foundation);
    if !waits.is_empty() {
        out.push_str("waits on the CEO:\n");
        for line in waits {
            out.push_str(&line);
            out.push('\n');
        }
    }
    let visions: Vec<String> = board
        .read(&["ls-tree", "--name-only", &main, "--", VISIONS])?
        .lines()
        .filter_map(|path| path.strip_prefix(VISIONS)?.strip_suffix(".json"))
        .map(str::to_owned)
        .collect();
    let unplaced: Vec<&String> = visions.iter().filter(|v| !placed.contains(v)).collect();
    if !unplaced.is_empty() {
        out.push_str(&format!(
            "visions placed nowhere (a brick's or the rules'): {}\n",
            unplaced
                .iter()
                .map(|v| v.as_str())
                .collect::<Vec<_>>()
                .join(", ")
        ));
    }
    let stray: Vec<String> = open
        .iter()
        .filter(|t| !named_tasks.contains(&t.id))
        .map(|t| {
            let vision = t.record.text("vision");
            format!(
                "  {} {}{}",
                t.id,
                if vision.is_empty() {
                    String::new()
                } else {
                    format!("[{vision}] ")
                },
                t.record.text("title")
            )
        })
        .collect();
    if !stray.is_empty() {
        out.push_str("open tasks no brick names:\n");
        for line in stray {
            out.push_str(&line);
            out.push('\n');
        }
    }
    Ok(out)
}

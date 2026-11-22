// SPDX-License-Identifier: Apache-2.0
//! The Susi Board: numbered, create-only items stored as JSON blobs under
//! `refs/board/items/<n>` (schema `susi-board-item-v1`), each naming the item before it.

use crate::git::{PushOutcome, Repo, Update};
use crate::json::{self, Json};

pub const ITEMS: &str = "refs/board/items/";
pub const SCHEMA: &str = "susi-board-item-v1";
/// The kinds agents post (Mandate 76 and the Board's own history).
pub const KINDS: [&str; 10] = [
    "claim", "landed", "cut", "heads-up", "request", "answer", "handoff", "decision", "blocked",
    "note",
];

pub struct Item {
    pub n: u64,
    pub oid: String,
    pub body: Json,
}

fn numbered(pairs: Vec<(String, String)>) -> Vec<(u64, String)> {
    let mut numbered: Vec<(u64, String)> = pairs
        .into_iter()
        .filter_map(|(name, oid)| {
            name.strip_prefix(ITEMS)
                .and_then(|n| n.parse::<u64>().ok())
                .map(|n| (n, oid))
        })
        .collect();
    numbered.sort_by_key(|(n, _)| *n);
    numbered
}

/// The newest `limit` items, oldest first, the way the Board is read.
pub fn latest(repo: &Repo, limit: usize) -> Result<(Vec<Item>, u64), String> {
    let numbered = numbered(repo.refs(ITEMS)?);
    let last = numbered.last().map_or(0, |(n, _)| *n);
    let start = numbered.len().saturating_sub(limit);
    let chosen = &numbered[start..];
    let oids: Vec<String> = chosen.iter().map(|(_, oid)| oid.clone()).collect();
    let blobs = repo.blobs(&oids)?;
    let mut items = Vec::with_capacity(chosen.len());
    for ((n, oid), blob) in chosen.iter().zip(blobs) {
        let blob =
            blob.ok_or_else(|| format!("board item {n} points at a missing object {oid}"))?;
        let text = String::from_utf8(blob).map_err(|_| format!("board item {n} is not UTF-8"))?;
        let body = json::parse(&text).map_err(|e| format!("board item {n} is not JSON: {e}"))?;
        if body.text("schema") != SCHEMA {
            return Err(format!(
                "board item {n} has schema {:?}, not {SCHEMA}",
                body.text("schema")
            ));
        }
        items.push(Item {
            n: *n,
            oid: oid.clone(),
            body,
        });
    }
    Ok((items, last))
}

/// One line per item: number, UTC date and time, kind, agent, and the title, with the addressee
/// and the item it answers when there are any.
pub fn line(item: &Item) -> String {
    let when = item
        .body
        .get("posted_unix")
        .and_then(Json::as_i64)
        .map_or_else(|| "????-??-?? ??:??".to_owned(), crate::time::utc_minutes);
    let to = item.body.text("to");
    let re = item.body.text("re");
    let mut title = String::new();
    if !to.is_empty() {
        title.push_str(&format!("to {to}: "));
    }
    if !re.is_empty() {
        title.push_str(&format!("re {re}: "));
    }
    title.push_str(&item.body.text("title"));
    format!(
        "{:>5}  {}  {:<9} {:<14} {}",
        item.n,
        when,
        item.body.text("kind"),
        item.body.text("agent"),
        title
    )
}

/// The same shape the earlier tool printed for `board list --json`: `{"items":[{"item":…,"n":…,"sha":…}]}`.
pub fn to_json(items: &[Item]) -> Json {
    Json::Object(vec![(
        "items".into(),
        Json::Array(
            items
                .iter()
                .map(|item| {
                    Json::Object(vec![
                        ("item".into(), item.body.clone()),
                        ("n".into(), Json::Number(item.n.to_string())),
                        ("sha".into(), Json::Str(item.oid.clone())),
                    ])
                })
                .collect(),
        ),
    )])
}

pub struct Post<'a> {
    pub kind: &'a str,
    pub agent: &'a str,
    pub title: &'a str,
    pub body: &'a str,
    pub to: Option<&'a str>,
    pub re: Option<u64>,
    pub task: Option<&'a str>,
}

/// The item's bytes, in the field order and layout every earlier item has.
fn render(post: &Post<'_>, n: u64, prev: &str, host: &str, now: i64) -> Vec<u8> {
    let mut fields = vec![
        ("schema".to_owned(), Json::Str(SCHEMA.into())),
        ("n".into(), Json::Number(n.to_string())),
        ("kind".into(), Json::Str(post.kind.into())),
        ("agent".into(), Json::Str(post.agent.into())),
        ("host".into(), Json::Str(host.into())),
        ("posted_unix".into(), Json::Number(now.to_string())),
        ("title".into(), Json::Str(post.title.into())),
        ("body".into(), Json::Str(post.body.into())),
    ];
    if let Some(to) = post.to {
        fields.push(("to".into(), Json::Str(to.into())));
    }
    if let Some(re) = post.re {
        fields.push(("re".into(), Json::Number(re.to_string())));
    }
    fields.push(("task".into(), Json::Str(post.task.unwrap_or("").into())));
    fields.push(("prev".into(), Json::Str(prev.into())));
    let mut text = Json::Object(fields).to_pretty();
    text.push('\n');
    text.into_bytes()
}

thread_local! {
    /// The item this run last posted, by Board (git directory and remote): a second post in the
    /// same run (`tasks start` posts Landed, then the claim) first tries the number after it,
    /// naming it, without listing the Board again. The push still only creates a ref that does
    /// not exist yet, so a number someone took meanwhile is refused and the Board is listed.
    static LAST_POSTED: std::cell::RefCell<std::collections::HashMap<(std::path::PathBuf, String), (u64, String)>> =
        std::cell::RefCell::new(std::collections::HashMap::new());
}

/// Post one item on `remote`'s Board: the next free number, naming the item before it. Two
/// posters never share a number: the push only creates a ref that does not exist yet, and a
/// poster that loses the race reads the Board again and takes the next number.
pub fn post(repo: &Repo, remote: &str, post: &Post<'_>) -> Result<(u64, String), String> {
    if !KINDS.contains(&post.kind) {
        return Err(format!("kind must be one of {}", KINDS.join(", ")));
    }
    if post.title.trim().is_empty() || post.title.contains('\n') {
        return Err("a title is one non-empty line".into());
    }
    let host = crate::agent::host();
    let board_key = (repo.git_dir().to_path_buf(), remote.to_owned());
    let mut guess = LAST_POSTED.with(|last| last.borrow().get(&board_key).cloned());
    for attempt in 0..12u32 {
        let (n, prev) = match guess.take() {
            Some((last, oid)) => (last + 1, oid),
            None => numbered(repo.remote_refs(remote, &["refs/board/items/*"])?)
                .last()
                .map_or((1, String::new()), |(n, oid)| (n + 1, oid.clone())),
        };
        let bytes = render(post, n, &prev, &host, crate::time::now_unix());
        let oid = repo.write_blob(&bytes)?;
        let update = Update {
            refname: format!("{ITEMS}{n}"),
            new: Some(oid.clone()),
            expect: None,
        };
        match repo.push_atomic(remote, &[update])? {
            PushOutcome::Done => {
                LAST_POSTED.with(|last| {
                    last.borrow_mut().insert(board_key, (n, oid.clone()));
                });
                return Ok((n, oid));
            }
            PushOutcome::Stale => {
                std::thread::sleep(std::time::Duration::from_millis(
                    40 + u64::from(attempt) * 60 + u64::from(std::process::id() % 50),
                ));
            }
        }
    }
    Err("the Board kept moving: twelve tries, each number taken first; nothing was posted".into())
}

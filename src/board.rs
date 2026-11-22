// SPDX-License-Identifier: Apache-2.0
//! The Susi Board, read only: numbered, create-only items stored as JSON blobs under
//! `refs/board/items/<n>` (schema `susi-board-item-v1`).

use crate::git::Repo;
use crate::json::{self, Json};

pub const ITEMS: &str = "refs/board/items/";
pub const SCHEMA: &str = "susi-board-item-v1";

pub struct Item {
    pub n: u64,
    pub oid: String,
    pub body: Json,
}

/// The newest `limit` items, oldest first, the way the Board is read.
pub fn latest(repo: &Repo, limit: usize) -> Result<(Vec<Item>, u64), String> {
    let mut numbered: Vec<(u64, String)> = repo
        .refs(ITEMS)?
        .into_iter()
        .filter_map(|(name, oid)| {
            name.strip_prefix(ITEMS)
                .and_then(|n| n.parse::<u64>().ok())
                .map(|n| (n, oid))
        })
        .collect();
    numbered.sort_by_key(|(n, _)| *n);
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

// SPDX-License-Identifier: Apache-2.0
//! The task queue, read only. An open task is a JSON record in `.agents/tasks/<id>.json` at the
//! named ref; a claim is a JSON blob at `refs/claims/<id>`, live while its lease has not passed.

use crate::git::Repo;
use crate::json::{self, Json};

pub const TASKS_DIR: &str = ".agents/tasks/";
pub const CLAIMS: &str = "refs/claims/";

pub struct OpenTask {
    pub id: String,
    pub record: Json,
    pub claim: Option<Json>,
    pub live: bool,
}

pub fn open(repo: &Repo, at: &str, now: i64) -> Result<Vec<OpenTask>, String> {
    let listing = repo.read(&["ls-tree", "-z", at, "--", TASKS_DIR])?;
    let mut ids = Vec::new();
    let mut oids = Vec::new();
    for entry in listing.split('\0').filter(|e| !e.is_empty()) {
        let Some((meta, path)) = entry.split_once('\t') else {
            continue;
        };
        let mut parts = meta.split_whitespace();
        let (_, kind, oid) = (parts.next(), parts.next(), parts.next());
        let Some(name) = path.strip_prefix(TASKS_DIR) else {
            continue;
        };
        if kind != Some("blob") || name.contains('/') {
            continue;
        }
        let Some(id) = name.strip_suffix(".json") else {
            continue;
        };
        ids.push(id.to_owned());
        oids.push(oid.unwrap_or_default().to_owned());
    }
    let records = repo.blobs(&oids)?;
    let claims = claims(repo)?;
    let mut open = Vec::with_capacity(ids.len());
    for (id, blob) in ids.into_iter().zip(records) {
        let blob = blob.ok_or_else(|| format!("task {id}: record object missing"))?;
        let text =
            String::from_utf8(blob).map_err(|_| format!("task {id}: record is not UTF-8"))?;
        let record =
            json::parse(&text).map_err(|e| format!("task {id}: record is not JSON: {e}"))?;
        let claim = claims
            .iter()
            .find(|(task, _)| *task == id)
            .map(|(_, c)| c.clone());
        let live = claim
            .as_ref()
            .and_then(|c| c.get("lease_until_unix").and_then(Json::as_i64))
            .is_some_and(|until| until > now);
        open.push(OpenTask {
            id,
            record,
            claim,
            live,
        });
    }
    open.sort_by_key(|t| natural_key(&t.id));
    Ok(open)
}

fn claims(repo: &Repo) -> Result<Vec<(String, Json)>, String> {
    let refs = repo.refs(CLAIMS)?;
    let oids: Vec<String> = refs.iter().map(|(_, oid)| oid.clone()).collect();
    let blobs = repo.blobs(&oids)?;
    let mut out = Vec::new();
    for ((name, _), blob) in refs.iter().zip(blobs) {
        let Some(task) = name.strip_prefix(CLAIMS) else {
            continue;
        };
        let Some(blob) = blob else { continue };
        if let Ok(claim) = String::from_utf8(blob)
            .map_err(|e| e.to_string())
            .and_then(|t| json::parse(&t))
        {
            out.push((task.to_owned(), claim));
        }
    }
    Ok(out)
}

/// `T-CLAUDE-9` sorts before `T-CLAUDE-10`.
fn natural_key(id: &str) -> (String, u64) {
    match id.rsplit_once('-') {
        Some((stem, n)) => (stem.to_owned(), n.parse().unwrap_or(u64::MAX)),
        None => (id.to_owned(), 0),
    }
}

pub fn accept_text(record: &Json) -> String {
    match record.get("accept").and_then(|a| a.get("cmd")) {
        Some(Json::Array(parts)) => parts
            .iter()
            .filter_map(Json::as_str)
            .collect::<Vec<_>>()
            .join(" "),
        _ => String::new(),
    }
}

pub fn to_json(tasks: &[OpenTask]) -> Json {
    let text = |s: String| Json::Str(s);
    Json::Object(vec![(
        "open".into(),
        Json::Array(
            tasks
                .iter()
                .map(|t| {
                    let claim = t.claim.as_ref();
                    let field = |key: &str| claim.map(|c| c.text(key)).unwrap_or_default();
                    let lease = claim
                        .and_then(|c| c.get("lease_until_unix"))
                        .cloned()
                        .unwrap_or(Json::Null);
                    Json::Object(vec![
                        ("id".into(), text(t.id.clone())),
                        ("title".into(), text(t.record.text("title"))),
                        ("size".into(), text(t.record.text("size"))),
                        ("accept".into(), text(accept_text(&t.record))),
                        (
                            "deps".into(),
                            Json::Array(
                                t.record
                                    .str_list("deps")
                                    .into_iter()
                                    .map(Json::Str)
                                    .collect(),
                            ),
                        ),
                        (
                            "claimed_by".into(),
                            if t.live {
                                text(field("agent"))
                            } else {
                                Json::Null
                            },
                        ),
                        (
                            "lapsed_claim_by".into(),
                            if claim.is_some() && !t.live {
                                text(field("agent"))
                            } else {
                                Json::Null
                            },
                        ),
                        (
                            "lease_until_unix".into(),
                            if claim.is_some() { lease } else { Json::Null },
                        ),
                        (
                            "scopes".into(),
                            Json::Array(
                                claim
                                    .map(|c| c.str_list("scopes"))
                                    .unwrap_or_default()
                                    .into_iter()
                                    .map(Json::Str)
                                    .collect(),
                            ),
                        ),
                    ])
                })
                .collect(),
        ),
    )])
}

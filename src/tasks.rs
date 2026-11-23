// SPDX-License-Identifier: Apache-2.0
//! The task queue. An open task is a JSON record in `.agents/tasks/<id>.json`; a finished task's
//! record is removed by the commit that closes it, so history keeps it. Claims are in `claims`.

use crate::git::Repo;
use crate::json::{self, Json};

pub const TASKS_DIR: &str = ".agents/tasks/";
pub const MAIN_SNAPSHOT: &str = "refs/susi-tools/remote/main";

pub struct OpenTask {
    pub id: String,
    pub record: Json,
}

/// Bring the remote's `main` into this program's own ref and return that ref's name. The same
/// fetch brings the claims, so a claim read later in the same run needs no fetch of its own.
pub fn fetch_main(repo: &Repo, remote: &str) -> Result<String, String> {
    crate::claims::fetch(repo, remote)?;
    Ok(MAIN_SNAPSHOT.to_owned())
}

/// The open task records at `at`; their claims are `claims::State`'s.
pub fn open_records(repo: &Repo, at: &str) -> Result<Vec<OpenTask>, String> {
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
    let mut open = Vec::with_capacity(ids.len());
    for (id, blob) in ids.into_iter().zip(records) {
        let blob = blob.ok_or_else(|| format!("task {id}: record object missing"))?;
        let text =
            String::from_utf8(blob).map_err(|_| format!("task {id}: record is not UTF-8"))?;
        let record =
            json::parse(&text).map_err(|e| format!("task {id}: record is not JSON: {e}"))?;
        open.push(OpenTask { id, record });
    }
    open.sort_by_key(|t| natural_key(&t.id));
    Ok(open)
}

/// `T-CLAUDE-9` sorts before `T-CLAUDE-10`.
fn natural_key(id: &str) -> (String, u64) {
    match id.rsplit_once('-') {
        Some((stem, n)) => (stem.to_owned(), n.parse().unwrap_or(u64::MAX)),
        None => (id.to_owned(), 0),
    }
}

pub fn accept_cmd(record: &Json) -> Vec<String> {
    match record.get("accept").and_then(|a| a.get("cmd")) {
        Some(Json::Array(parts)) => parts
            .iter()
            .filter_map(Json::as_str)
            .map(str::to_owned)
            .collect(),
        _ => Vec::new(),
    }
}

/// Every task id this repository has ever used for `stem` (`T-CLAUDE-`): open records, finished
/// records still in the tree, records removed by a close (from history), and close receipts.
fn used_numbers(repo: &Repo, stem: &str) -> Result<Vec<u64>, String> {
    let mut names: Vec<String> = Vec::new();
    names.extend(
        repo.read(&["ls-tree", "-r", "--name-only", "HEAD", "--", TASKS_DIR])
            .unwrap_or_default()
            .lines()
            .map(str::to_owned),
    );
    names.extend(
        repo.read(&[
            "log",
            "--all",
            "--format=",
            "--name-only",
            "--diff-filter=AD",
            "--",
            TASKS_DIR,
        ])?
        .lines()
        .map(str::to_owned),
    );
    // Close receipts, abandonments and claims, in one `for-each-ref`.
    names.extend(
        repo.read(&[
            "for-each-ref",
            "--format=%(refname)",
            "refs/closed/",
            "refs/abandoned/",
            "refs/claims/",
        ])?
        .lines()
        .map(str::to_owned),
    );
    Ok(names
        .iter()
        .filter_map(|name| {
            let file = name.rsplit('/').next()?;
            let id = file.strip_suffix(".json").unwrap_or(file);
            id.strip_prefix(stem)?.parse::<u64>().ok()
        })
        .collect())
}

pub struct NewTask<'a> {
    pub title: &'a str,
    pub goal: &'a str,
    pub size: &'a str,
    pub accept: &'a [String],
    pub deps: &'a [String],
    pub agent: &'a str,
}

/// Acceptance runs on whoever closes the task, so it is `cargo …` or a script of the repository.
fn safe_accept(cmd: &[String]) -> Result<(), String> {
    let Some(program) = cmd.first() else {
        return Err("an acceptance command is required: --accept \"cargo test …\"".into());
    };
    let script = program.strip_prefix("scripts/").is_some_and(|rest| {
        !rest.is_empty()
            && rest
                .split('/')
                .all(|p| !p.is_empty() && p != "." && p != "..")
    });
    if program == "cargo" || script {
        Ok(())
    } else {
        Err(format!(
            "an acceptance command runs `cargo` or a repository script under scripts/, not {program:?}"
        ))
    }
}

/// Write a new task record into the checkout and return its id and path; `tasks start` commits
/// it. Nothing else is changed.
pub fn add(repo: &Repo, new: &NewTask<'_>) -> Result<(String, std::path::PathBuf), String> {
    if new.title.trim().is_empty() || new.title.contains('\n') {
        return Err("a title is one non-empty line".into());
    }
    if !["xs", "s", "m", "l"].contains(&new.size) {
        return Err("size is xs, s, m or l".into());
    }
    safe_accept(new.accept)?;
    let token = crate::agent::token(new.agent)?;
    let stem = format!("T-{token}-");
    let next = used_numbers(repo, &stem)?.into_iter().max().unwrap_or(0) + 1;
    let id = format!("{stem}{next}");
    let record = Json::Object(vec![
        ("id".into(), Json::Str(id.clone())),
        ("title".into(), Json::Str(new.title.trim().into())),
        ("goal".into(), Json::Str(new.goal.into())),
        ("size".into(), Json::Str(new.size.into())),
        (
            "deps".into(),
            Json::Array(new.deps.iter().cloned().map(Json::Str).collect()),
        ),
        (
            "accept".into(),
            Json::Object(vec![(
                "cmd".into(),
                Json::Array(new.accept.iter().cloned().map(Json::Str).collect()),
            )]),
        ),
        ("created_by".into(), Json::Str(token)),
        (
            "created_unix".into(),
            Json::Number(crate::time::now_unix().to_string()),
        ),
    ]);
    let path = repo.work_tree()?.join(TASKS_DIR).join(format!("{id}.json"));
    if path.exists() {
        return Err(format!("{} already exists", path.display()));
    }
    std::fs::create_dir_all(path.parent().ok_or("no task directory")?)
        .map_err(|e| format!("cannot create the task directory: {e}"))?;
    let mut text = record.to_pretty();
    text.push('\n');
    std::fs::write(&path, text).map_err(|e| format!("cannot write {}: {e}", path.display()))?;
    Ok((id, path))
}

/// Run a task's acceptance in the checkout; only if it passes, commit the removal of its
/// record. The commit is the close: history keeps the record and the command that passed.
pub fn close(repo: &Repo, id: &str) -> Result<String, String> {
    let work = repo.work_tree()?.to_path_buf();
    let rel = format!("{TASKS_DIR}{id}.json");
    let dirty = repo.read(&["status", "--porcelain", "--untracked-files=no"])?;
    if !dirty.trim().is_empty() {
        return Err(
            "commit or set aside your changes first: closing makes a commit of its own".into(),
        );
    }
    let text = std::fs::read_to_string(work.join(&rel))
        .map_err(|_| format!("{id} has no open record at {rel} in this checkout"))?;
    let record = json::parse(&text).map_err(|e| format!("{rel} is not JSON: {e}"))?;
    let cmd = accept_cmd(&record);
    safe_accept(&cmd)?;
    eprintln!("susi-tools: running {id}'s acceptance: {}", cmd.join(" "));
    let status = std::process::Command::new(&cmd[0])
        .args(&cmd[1..])
        .current_dir(&work)
        .status()
        .map_err(|e| format!("cannot run {}: {e}", cmd[0]))?;
    if !status.success() {
        return Err(format!(
            "{id} is not done: its acceptance `{}` exited {}",
            cmd.join(" "),
            status
                .code()
                .map_or_else(|| "on a signal".to_owned(), |c| c.to_string())
        ));
    }
    repo.read(&["rm", "--quiet", "--", &rel])?;
    let message = format!(
        "chore(tasks): close {id}\n\nIts acceptance passed: `{}`.\n\nTask: {id}\n",
        cmd.join(" ")
    );
    repo.run_visible(&["commit", "--quiet", "-m", &message])?;
    repo.read(&["rev-parse", "HEAD"])
        .map(|s| s.trim().to_owned())
}

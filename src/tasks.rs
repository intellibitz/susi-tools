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
            // Every merge parent: default history simplification follows a merge's parent whose
            // tree matches, which hides a task added and closed on the other side.
            "--full-history",
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
    /// The vision it serves (`V-<n>`), checked on Susi's `main` before the record is written.
    pub vision: &'a str,
    /// The rung of the brick ladder it moves, when it moves one.
    pub rung: &'a str,
    /// The paths it may change: the scope its claim takes.
    pub scope: &'a [String],
    /// What to read before starting: the seam.
    pub read: &'a [String],
    /// The agent it is meant for, or empty for anyone.
    pub for_agent: &'a str,
}

/// The rungs of the brick ladder (Susi's `AGENTS.md`, "Walking the two tracks").
pub const RUNGS: [&str; 5] = ["built", "used", "ready", "released", "delivered"];

/// Where Susi keeps the CEO's visions, each `V-<n>.json` with its `statement`.
pub const VISIONS_DIR: &str = ".agents/visions/";

/// A vision's id has its shape: `V-` and a number.
pub fn vision_id(id: &str) -> Result<(), String> {
    match id.strip_prefix("V-") {
        Some(n) if !n.is_empty() && n.bytes().all(|b| b.is_ascii_digit()) => Ok(()),
        _ => Err(format!(
            "a task names the vision it serves: --vision V-<n>, one of Susi's .agents/visions, not {id:?}"
        )),
    }
}

/// The CEO's words for vision `id`, read from Susi's `main` as the remote has it (`board` is the
/// Susi clone, which holds the visions as it holds the Board): a task serves a vision the CEO
/// set, not one only a checkout knows.
pub fn vision_statement(board: &Repo, id: &str) -> Result<String, String> {
    vision_id(id)?;
    let main = fetch_main(board, "origin")?;
    let blob = board
        .try_read(&["cat-file", "blob", &format!("{main}:{VISIONS_DIR}{id}.json")])
        .ok_or_else(|| {
            format!("{id} is not a vision on Susi's main ({VISIONS_DIR}{id}.json): record it there, ship it, then name it")
        })?;
    let record = json::parse(&blob).map_err(|e| format!("{id}: the vision is not JSON: {e}"))?;
    let statement = record.text("statement");
    if statement.trim().is_empty() {
        return Err(format!("{id}: the vision states nothing"));
    }
    Ok(statement)
}

/// The words of a command line given as one string (`--accept`), split as a POSIX shell splits
/// them: blanks separate words; within single quotes every character is itself; within double
/// quotes a backslash keeps a following `"`, `\\`, `$` or `` ` ``; outside quotes a backslash keeps
/// the next character. An unclosed quote is an error. Until T-CLOUDCLAUDE-2 (Susi Tools) a line was
/// split at every blank, so a quoted nextest filter became a broken check.
pub fn words(line: &str) -> Result<Vec<String>, String> {
    let mut out = Vec::new();
    let mut word = String::new();
    let mut in_word = false;
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\'' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('\'') => break,
                        Some(other) => word.push(other),
                        None => return Err(format!("an unclosed ' in `{line}`")),
                    }
                }
            }
            '"' => {
                in_word = true;
                loop {
                    match chars.next() {
                        Some('"') => break,
                        Some('\\') => match chars.next() {
                            Some(kept @ ('"' | '\\' | '$' | '`')) => word.push(kept),
                            Some(other) => {
                                word.push('\\');
                                word.push(other);
                            }
                            None => return Err(format!("an unclosed \" in `{line}`")),
                        },
                        Some(other) => word.push(other),
                        None => return Err(format!("an unclosed \" in `{line}`")),
                    }
                }
            }
            '\\' => {
                in_word = true;
                if let Some(next) = chars.next() {
                    word.push(next);
                }
            }
            blank if blank.is_whitespace() => {
                if in_word {
                    out.push(std::mem::take(&mut word));
                    in_word = false;
                }
            }
            other => {
                in_word = true;
                word.push(other);
            }
        }
    }
    if in_word {
        out.push(word);
    }
    Ok(out)
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

/// A title as two titles are compared: lower-cased, its words single-spaced.
fn title_key(title: &str) -> String {
    title
        .split_whitespace()
        .map(str::to_lowercase)
        .collect::<Vec<_>>()
        .join(" ")
}

/// The open task `token`'s agent made with the title `title`, in this checkout or at origin's
/// `main` as last fetched (Board 7099: one agent registering one title twice left two open
/// copies, and every agent paid for the wrong one from `tasks next`). Another agent's task of the
/// same title is not one.
fn duplicate(repo: &Repo, token: &str, title: &str) -> Result<Option<String>, String> {
    let wanted = title_key(title);
    let mut places = vec!["HEAD".to_owned()];
    if repo
        .try_read(&["rev-parse", "--verify", "--quiet", MAIN_SNAPSHOT])
        .is_some()
    {
        places.push(MAIN_SNAPSHOT.to_owned());
    }
    for at in places {
        for task in open_records(repo, &at).unwrap_or_default() {
            let by = task.record.get("created_by").and_then(Json::as_str);
            let named = task.record.get("title").and_then(Json::as_str);
            if by == Some(token) && named.is_some_and(|t| title_key(t) == wanted) {
                return Ok(Some(task.id));
            }
        }
    }
    Ok(None)
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
    vision_id(new.vision)?;
    if !new.rung.is_empty() && !RUNGS.contains(&new.rung) {
        return Err(format!("a rung is one of {}", RUNGS.join(", ")));
    }
    if new.scope.is_empty() {
        return Err("name the paths the task may change: --scope <path> (repeatable)".into());
    }
    let for_agent = if new.for_agent.is_empty() {
        String::new()
    } else {
        crate::agent::token(new.for_agent)?;
        new.for_agent.trim().to_ascii_lowercase()
    };
    let token = crate::agent::token(new.agent)?;
    if let Some(id) = duplicate(repo, &token, new.title)? {
        return Err(format!(
            "{token} already has `{}` open as {id}: claim it (susi-tools tasks claim {id}) rather than add it again",
            new.title.trim()
        ));
    }
    let stem = format!("T-{token}-");
    let next = used_numbers(repo, &stem)?.into_iter().max().unwrap_or(0) + 1;
    let id = format!("{stem}{next}");
    let strings = |list: &[String]| Json::Array(list.iter().cloned().map(Json::Str).collect());
    let mut fields = vec![
        ("id".into(), Json::Str(id.clone())),
        ("title".into(), Json::Str(new.title.trim().into())),
        ("goal".into(), Json::Str(new.goal.into())),
        ("vision".into(), Json::Str(new.vision.into())),
    ];
    if !new.rung.is_empty() {
        fields.push(("rung".into(), Json::Str(new.rung.into())));
    }
    if !for_agent.is_empty() {
        fields.push(("for".into(), Json::Str(for_agent)));
    }
    fields.extend([
        ("size".into(), Json::Str(new.size.into())),
        ("deps".into(), strings(new.deps)),
        ("scope".into(), strings(new.scope)),
        ("read".into(), strings(new.read)),
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
    let record = Json::Object(fields);
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

/// The open task `id`'s record: in this checkout, on this repository's `main` as the remote has
/// it, or, for a task of Susi's queue worked on in Susi Tools, on Susi's `main` (`board`). With
/// the record, the name of the repository whose queue holds it.
pub fn find(repo: &Repo, board: Option<&Repo>, id: &str) -> Result<(Json, String), String> {
    let rel = format!("{TASKS_DIR}{id}.json");
    let parse = |text: &str| json::parse(text).map_err(|e| format!("{rel} is not JSON: {e}"));
    if let Ok(text) = std::fs::read_to_string(repo.work_tree()?.join(&rel)) {
        return Ok((parse(&text)?, repo_name(repo)));
    }
    let mut places = vec![repo];
    places.extend(board.filter(|b| b.git_dir() != repo.git_dir()));
    for place in places {
        let main = fetch_main(place, "origin")?;
        if let Some(text) = place.try_read(&["cat-file", "blob", &format!("{main}:{rel}")]) {
            return Ok((parse(&text)?, repo_name(place)));
        }
    }
    Err(format!("{id} is not an open task here or on Susi's main"))
}

/// A repository's name, from its `origin` (`susi`, `susi-tools`).
pub fn repo_name(repo: &Repo) -> String {
    let url = repo
        .try_read(&["config", "--get", "remote.origin.url"])
        .unwrap_or_default();
    let last = url
        .trim()
        .trim_end_matches('/')
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or_default();
    let name = last.strip_suffix(".git").unwrap_or(last);
    if name.is_empty() {
        "this repository".to_owned()
    } else {
        name.to_owned()
    }
}

/// The first open task for `agent`: unclaimed or its claim lapsed, every task it depends on
/// closed, and meant for `agent` or for anyone. Tasks meant for `agent` come first.
pub fn next<'a>(
    open: &'a [OpenTask],
    claims: &crate::claims::State,
    agent: &str,
    now: i64,
) -> Option<&'a OpenTask> {
    let me = crate::agent::token(agent).unwrap_or_default();
    let free = |t: &&OpenTask| {
        claims.on_task(&t.id).is_none_or(|(_, _, c)| !c.live(now))
            && t.record
                .str_list("deps")
                .iter()
                .all(|dep| open.iter().all(|o| &o.id != dep))
    };
    let meant = |t: &OpenTask| crate::agent::token(&t.record.text("for")).unwrap_or_default();
    open.iter()
        .filter(free)
        .find(|t| meant(t) == me)
        .or_else(|| open.iter().filter(free).find(|t| meant(t).is_empty()))
}

/// The words to give an agent so it can do task `id` from nothing: why (the vision, in the
/// CEO's words), what, where, what to read first, when it is done, and the loop that lands it.
pub fn brief(record: &Json, repository: &str, vision: Option<&str>) -> String {
    let id = record.text("id");
    let list = |key: &str| {
        let items = record.str_list(key);
        if items.is_empty() {
            "(none named)".to_owned()
        } else {
            items.join(", ")
        }
    };
    let mut out = format!("Task {id}: {}\n", record.text("title"));
    let meant = record.text("for");
    out.push_str(&format!(
        "For: {}. Repository: {repository} (your own clone of it, in your agent root; Susi's clone\n  sits in the folder susi-commercial-apps).\n",
        if meant.is_empty() {
            "any agent"
        } else {
            &meant
        }
    ));
    match (record.text("vision").as_str(), vision) {
        ("", _) => out.push_str("Why: this task names no vision (it was made before V-48).\n"),
        (v, Some(words)) => out.push_str(&format!("Why: {v}, in the CEO's words: \"{words}\"\n")),
        (v, None) => out.push_str(&format!("Why: {v} (Susi's .agents/visions/{v}.json).\n")),
    }
    let rung = record.text("rung");
    if !rung.is_empty() {
        out.push_str(&format!("It moves its brick to the rung: {rung}.\n"));
    }
    let goal = record.text("goal");
    if !goal.trim().is_empty() {
        out.push_str(&format!("Goal: {}\n", goal.trim()));
    }
    out.push_str(&format!("Change only: {}\n", list("scope")));
    out.push_str(&format!("Read first: {}\n", list("read")));
    out.push_str(&format!(
        "Done when this passes: {}\n",
        accept_cmd(record).join(" ")
    ));
    let deps = record.str_list("deps");
    if !deps.is_empty() {
        out.push_str(&format!("After: {} (closed first)\n", deps.join(", ")));
    }
    out.push_str(&format!(
        "\nHow: Susi's contract, AGENTS.md in your Susi clone (\"Working here\"), is the rule; in short:
1. In your agent root (~/susi-agents/<you>, its env.sh sourced: your own SUSI_HOME and build
   target), make a worktree of {repository} from origin/main and run `susi-tools workflow check`.
2. Claim the task there: `susi-tools tasks claim {id}` (it takes the paths above).
3. Look before you build: reuse what exists; a task of size m or l records where you looked in
   its `reuse` field, committed alone first.
4. Make the check pass, changing only those paths; commit small, each message ending with the
   trailer `Task: {id}`.
5. Run the gates, inside the 10-second line: in Susi, the command AGENTS.md gives (\"Working
   here\", step 6, with its five --target flags); in Susi Tools, `susi-tools gates` alone (it is
   built for Linux only). Then `susi-tools ship`, and read what it prints for you; when main has
   moved, merge it, test, ship again.
6. `susi-tools tasks finish {id}`, then remove your worktree.
7. Report: what you built, the commits, the check's tests, the gates' time, and anything not done.
"
    ));
    out
}

#[cfg(test)]
mod tests {
    use super::words;

    fn owned(list: &[&str]) -> Result<Vec<String>, String> {
        Ok(list.iter().map(|w| (*w).to_owned()).collect())
    }

    #[test]
    fn an_acceptance_line_splits_as_a_shell_splits_it() {
        assert_eq!(
            words("cargo nextest run --workspace -E 'package(x) | binary_id(y::z)'"),
            owned(&[
                "cargo",
                "nextest",
                "run",
                "--workspace",
                "-E",
                "package(x) | binary_id(y::z)"
            ])
        );
        assert_eq!(
            words(r#"  cargo  test "a \"b\" \\ c" it\ s''x  "#),
            owned(&["cargo", "test", r#"a "b" \ c"#, "it sx"])
        );
        assert_eq!(words(r#"say "back\slash""#), owned(&["say", r"back\slash"]));
        assert_eq!(words("''"), owned(&[""]));
        assert_eq!(words(""), owned(&[]));
        assert!(words("cargo 'open").is_err_and(|e| e.contains("unclosed '")));
        assert!(words("cargo \"open").is_err_and(|e| e.contains("unclosed \"")));
    }
}

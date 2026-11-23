// SPDX-License-Identifier: Apache-2.0
//! `susi-tools hook <stage>`: what the Git hooks of Susi and Susi Core run. Each repository's
//! `.githooks/<stage>` is one line that calls this, so the rules live in one place, written
//! once and tested here (until 2026-11-23 they were about 1,400 lines of shell copied between
//! the two repositories). What a repository asks for beyond the common rules, its hook says:
//!
//! - `--token-prefix <prefix>` (repeatable): the product's own licence tokens, `<prefix>.` or
//!   `<prefix>:` and 40 or more token characters, are refused like any other secret. The prefix
//!   comes from the repository, so this program names no product's tokens.
//! - `--worktrees`: commits and merges only in a linked worktree, and no plain commit on
//!   `main` (Mandate 49).
//! - `--isolation`: the agent's own `SUSI_HOME`, build target and scratch folders, outside every
//!   checkout (Mandate 97), for the repository whose programs read `SUSI_HOME`.
//!
//! The stages:
//!
//! - `pre-commit`: the agent's own root and a fresh checkout (`hooks.rs`), then the staged
//!   change: no new blob of 1 MiB or more, no secret-shaped line, no added `dbg!(`,
//!   identical `CLAUDE.md` and `GEMINI.md` pointers, and the staged Rust files formatted.
//! - `commit-msg <file>`: a subject that says what; a `Task:` trailer naming a task recorded or
//!   claimed here; for a claimed task, the claim is yours and covers every staged file; a sized
//!   task states where its agent looked (the reuse rule, `hooks.rs`).
//! - `pre-merge-commit --worktrees`: the worktree rule for a merge (Susi Core's hook).
//! - `pre-push <remote> [url]`: refs and records on standard input, as Git gives them. Only
//!   `main` is a branch, and it only moves forward; agents publish to `refs/agents/<name>`
//!   (fast-forwards, or deleted); `vX.Y.Z` tags are created once; the task queue's records are
//!   blobs; Board items are created once, as blobs. Anything else is refused.
//! - `post-checkout <old> <new> <flag>`: after `git worktree add`, lists the finished worktrees
//!   to remove first (Mandate 98). Advice only.
//!
//! The public repositories' boundary (nothing of the private product enters them) is not
//! checked here: each public repository's canonical server refuses it at push, with the same
//! patterns, and a public program does not spell out the private product's markers.
//!
//! Escape hatches, deliberate and visible, never the default: those of the single rules (see
//! `hooks.rs`) and `SUSI_ALLOW_SHARED_HOME` for the isolation check.

use crate::git::Repo;
use crate::hooks::{self, Stop};
use std::path::{Path, PathBuf};

/// A staged or pushed blob this large is refused when new, and advised about when it changes.
pub const MAX_BLOB_BYTES: u64 = 1_048_576;
/// The task queue's record namespaces: compare-and-swap blobs, not source history.
const QUEUE_NAMESPACES: [&str; 5] = [
    "refs/claims/",
    "refs/claim-agents/",
    "refs/claim-generation/",
    "refs/closed/",
    "refs/abandoned/",
];

const USAGE: &str = "\
  susi-tools hook pre-commit [--worktrees] [--isolation] [--token-prefix <prefix>]...
  susi-tools hook commit-msg <message-file>
  susi-tools hook pre-merge-commit --worktrees
  susi-tools hook pre-push <remote> [url]   (records on standard input)
  susi-tools hook post-checkout <old> <new> <flag>
Each repository's .githooks/<stage> is one line that runs the stage with the options it needs.";

/// What a repository's hook asks for beyond the common rules.
#[derive(Default)]
struct Policy {
    worktrees: bool,
    isolation: bool,
    token_prefixes: Vec<String>,
}

/// `hook <stage> ...`; `args` starts at the stage.
pub fn run_cli(args: &[&str]) -> Result<(), Stop> {
    if args.iter().any(|a| matches!(*a, "--help" | "-h")) {
        println!("{USAGE}");
        return Ok(());
    }
    let mut policy = Policy::default();
    let mut rest = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        match *arg {
            "--worktrees" => policy.worktrees = true,
            "--isolation" => policy.isolation = true,
            "--token-prefix" => policy.token_prefixes.push(
                it.next()
                    .map(|p| (*p).to_owned())
                    .filter(|p| !p.is_empty())
                    .ok_or_else(|| Stop::Usage("--token-prefix needs a value".into()))?,
            ),
            other => rest.push(other),
        }
    }
    let cwd = std::env::current_dir().map_err(|e| format!("no current folder: {e}"))?;
    let repo = Repo::open(&cwd)?;
    match rest.as_slice() {
        ["pre-commit"] => pre_commit(&repo, &policy),
        ["commit-msg", file] => {
            let message = std::fs::read_to_string(file)
                .map_err(|e| Stop::Usage(format!("cannot read {file}: {e}")))?;
            commit_msg(&repo, &message)
        }
        ["pre-merge-commit"] if policy.worktrees => hooks::guard(&repo, false, "origin", None),
        ["pre-merge-commit"] => Err(Stop::Usage(
            "pre-merge-commit applies only the worktree rule: give --worktrees".into(),
        )),
        ["pre-push", _remote, ..] => {
            let mut input = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut input)
                .map_err(|e| format!("cannot read the push records: {e}"))?;
            pre_push(&repo, &input)
        }
        ["post-checkout", old, ..] => {
            if is_zero(old) {
                advise_finished_worktrees(&repo);
            }
            Ok(())
        }
        _ => Err(Stop::Usage(format!(
            "unknown hook stage {:?}\n{USAGE}",
            rest.join(" ")
        ))),
    }
}

fn is_zero(oid: &str) -> bool {
    !oid.is_empty() && oid.bytes().all(|b| b == b'0')
}

fn warn(text: &str) {
    eprintln!("⚠ {text}");
}

/// Refusals gathered from several rules, reported together.
#[derive(Default)]
struct Refusals(Vec<String>);

impl Refusals {
    fn add(&mut self, text: String) {
        self.0.push(format!("❌ {text}"));
    }

    fn finish(self) -> Result<(), Stop> {
        if self.0.is_empty() {
            Ok(())
        } else {
            Err(Stop::Refused(self.0.join("\n")))
        }
    }
}

// --- pre-commit ---------------------------------------------------------------------------

fn pre_commit(repo: &Repo, policy: &Policy) -> Result<(), Stop> {
    let merging = repo.git_dir().join("MERGE_HEAD").is_file();
    if policy.worktrees {
        // A merge's own commit follows the merge rule: a merge on main is the remedy.
        hooks::guard(repo, !merging, "origin", None)?;
    } else {
        hooks::own_root(repo, None)?;
        if !merging {
            hooks::fresh(repo, "origin")?;
        }
    }
    if merging {
        return Ok(());
    }
    if policy.isolation {
        isolation(repo)?;
    }
    let staged = staged_files(repo)?;
    if staged.is_empty() {
        return Ok(());
    }
    let mut refusals = Refusals::default();
    for file in &staged {
        if file.size >= MAX_BLOB_BYTES {
            if file.status == 'A' {
                refusals.add(format!(
                    "Mandate 94: {} is {} bytes; blobs of 1 MiB or more stay out of source.",
                    file.path, file.size
                ));
            } else {
                warn(&format!(
                    "Mandate 94: {} is {} bytes; move it out of source or shrink it (advice).",
                    file.path, file.size
                ));
            }
        }
    }
    let diff = repo.read(&[
        "diff",
        "--cached",
        "-U0",
        "--no-color",
        "--no-renames",
        "--diff-filter=ACMR",
    ])?;
    check_added_lines(&added_lines(&diff), &policy.token_prefixes, &mut refusals);
    if staged
        .iter()
        .any(|f| f.path == "CLAUDE.md" || f.path == "GEMINI.md")
    {
        if let (Some(claude), Some(gemini)) = (
            repo.try_read(&["show", ":CLAUDE.md"]),
            repo.try_read(&["show", ":GEMINI.md"]),
        ) {
            if claude != gemini {
                refusals.add("CLAUDE.md and GEMINI.md must stay byte-identical pointers.".into());
            }
        }
    }
    refusals.finish()?;
    formatting(repo, &staged)
}

struct Staged {
    path: String,
    oid: String,
    status: char,
    size: u64,
}

/// The staged additions and changes (`--raw`), each sized with one `git cat-file`.
fn staged_files(repo: &Repo) -> Result<Vec<Staged>, String> {
    let raw = repo.read(&[
        "diff",
        "--cached",
        "--raw",
        "-z",
        "--no-abbrev",
        "--no-renames",
        "--diff-filter=ACMR",
    ])?;
    let mut fields = raw.split('\0');
    let mut files = Vec::new();
    while let (Some(meta), Some(path)) = (fields.next(), fields.next()) {
        let parts: Vec<&str> = meta.split_whitespace().collect();
        let (Some(oid), Some(status)) = (parts.get(3), parts.get(4)) else {
            continue;
        };
        files.push(Staged {
            path: path.to_owned(),
            oid: (*oid).to_owned(),
            status: status.chars().next().unwrap_or('M'),
            size: 0,
        });
    }
    let oids: Vec<String> = files.iter().map(|f| f.oid.clone()).collect();
    for (file, size) in files.iter_mut().zip(repo.sizes(&oids)?) {
        file.size = size;
    }
    Ok(files)
}

/// `(file, line)` for every added line of a zero-context diff, leaving out vendored upstream
/// code and test fixtures, which stay as their authors wrote them.
fn added_lines(diff: &str) -> Vec<(String, String)> {
    let mut file = String::new();
    let mut skip = false;
    let mut lines = Vec::new();
    for line in diff.lines() {
        if let Some(path) = line.strip_prefix("+++ ") {
            file = path.strip_prefix("b/").unwrap_or(path).to_owned();
            skip = file.starts_with("vendor/")
                || file.contains("/vendor/")
                || file.contains("/tests/fixtures/");
        } else if let Some(text) = line.strip_prefix('+') {
            if !skip {
                lines.push((file.clone(), text.to_owned()));
            }
        }
    }
    lines
}

fn check_added_lines(
    lines: &[(String, String)],
    token_prefixes: &[String],
    refusals: &mut Refusals,
) {
    let mut secrets = Vec::new();
    let mut debug = Vec::new();
    for (file, line) in lines {
        if secret_shaped(line, token_prefixes) && !secrets.contains(file) {
            secrets.push(file.clone());
        }
        if file.ends_with(".rs") && has_dbg(line) && !debug.contains(file) {
            debug.push(file.clone());
        }
    }
    if !secrets.is_empty() {
        refusals.add(format!(
            "Secrets never enter source (rotate any that did): a key, licence token or API token in: {}",
            secrets.join(" ")
        ));
    }
    if !debug.is_empty() {
        refusals.add(format!("Mandate 93: remove dbg! from: {}", debug.join(" ")));
    }
}

/// A private-key header alone on its line (as in a key file, so code and documents that name
/// the header pass), a well-known API token shape, or a licence token of one of the
/// repository's `token_prefixes`.
pub fn secret_shaped(line: &str, token_prefixes: &[String]) -> bool {
    let trimmed = line.trim();
    if let Some(rest) = trimmed.strip_prefix("-----BEGIN ") {
        // `PRIVATE KEY-----`, or one upper-case word before it (`RSA`, `OPENSSH`, `EC`).
        let typed = rest.split_once(' ').is_some_and(|(word, after)| {
            !word.is_empty()
                && word.bytes().all(|b| b.is_ascii_uppercase())
                && after == "PRIVATE KEY-----"
        });
        if rest == "PRIVATE KEY-----" || typed {
            return true;
        }
    }
    let alnum = |c: char| c.is_ascii_alphanumeric();
    let token = |c: char| c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-');
    let upper_digit = |c: char| c.is_ascii_uppercase() || c.is_ascii_digit();
    let licence = token_prefixes.iter().any(|prefix| {
        run_after(line, &format!("{prefix}."), token, 40)
            || run_after(line, &format!("{prefix}:"), token, 40)
    });
    licence
        || run_after(line, "ghp_", alnum, 36)
        || run_after(line, "github_pat_", |c| alnum(c) || c == '_', 40)
        || run_after(line, "sk-ant-", |c| alnum(c) || c == '_' || c == '-', 30)
        || run_after(line, "AKIA", upper_digit, 16)
}

/// Whether some `prefix` in `line` is followed by at least `min` characters of `class`.
fn run_after(line: &str, prefix: &str, class: impl Fn(char) -> bool, min: usize) -> bool {
    line.match_indices(prefix).any(|(at, _)| {
        line[at + prefix.len()..]
            .chars()
            .take_while(|c| class(*c))
            .count()
            >= min
    })
}

/// An added `dbg!(` that is not the tail of a longer name.
pub fn has_dbg(line: &str) -> bool {
    line.match_indices("dbg!(").any(|(at, _)| {
        line[..at]
            .chars()
            .next_back()
            .is_none_or(|c| !(c.is_ascii_alphanumeric() || c == '_'))
    })
}

/// The staged Rust files with `rustfmt --check`, each with its crate's edition (well under a
/// second); only when that fails, the authoritative `cargo fmt --all --check`, so a file
/// outside every crate's module tree never refuses what the full formatter accepts.
fn formatting(repo: &Repo, staged: &[Staged]) -> Result<(), Stop> {
    let work = repo.work_tree()?;
    let mut by_edition: Vec<(String, Vec<PathBuf>)> = Vec::new();
    for file in staged {
        if !file.path.ends_with(".rs")
            || file.path.starts_with("vendor/")
            || file.path.contains("/vendor/")
        {
            continue;
        }
        let path = work.join(&file.path);
        if !path.is_file() {
            continue;
        }
        let edition = edition_of(work, &file.path);
        match by_edition.iter_mut().find(|(e, _)| *e == edition) {
            Some((_, files)) => files.push(path),
            None => by_edition.push((edition, vec![path])),
        }
    }
    let quick = by_edition.iter().all(|(edition, files)| {
        std::process::Command::new("rustfmt")
            .args(["--edition", edition, "--check"])
            .args(files)
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null())
            .status()
            .is_ok_and(|s| s.success())
    });
    if quick {
        return Ok(());
    }
    let full = std::process::Command::new("cargo")
        .args(["fmt", "--all", "--check"])
        .current_dir(work)
        .status()
        .is_ok_and(|s| s.success());
    if full {
        return Ok(());
    }
    Err(Stop::Refused(
        "❌ Formatting: run `cargo fmt --all` and commit again.".into(),
    ))
}

/// The edition named by the nearest `Cargo.toml` above `path` that names one (2021 otherwise).
fn edition_of(work: &Path, path: &str) -> String {
    let mut dir = Path::new(path).parent();
    while let Some(d) = dir {
        if let Ok(text) = std::fs::read_to_string(work.join(d).join("Cargo.toml")) {
            let named = text.lines().find_map(|line| {
                let value = line.trim().strip_prefix("edition")?.trim_start();
                let value = value.strip_prefix('=')?.trim().strip_prefix('"')?;
                let year = value.split('"').next()?;
                (!year.is_empty() && year.bytes().all(|b| b.is_ascii_digit()))
                    .then(|| year.to_owned())
            });
            if let Some(year) = named {
                return year;
            }
        }
        dir = d.parent();
    }
    "2021".into()
}

// --- isolation (Mandate 97) ---------------------------------------------------------------

/// Refuse when this instance would build, test or run a Susi program in state another instance
/// shares: `SUSI_HOME` unset, relative, a shared default or inside a checkout, or claimed by
/// another live worktree; a build target or scratch folder inside a checkout. On success the
/// home records which worktree owns it (`<SUSI_HOME>/.agent-isolation`).
fn isolation(repo: &Repo) -> Result<(), Stop> {
    if hooks::allowed("SUSI_ALLOW_SHARED_HOME") {
        eprintln!("[isolation] SUSI_ALLOW_SHARED_HOME is set: isolation checks skipped on purpose");
        return Ok(());
    }
    let top = canon(repo.work_tree()?);
    let checkouts = worktrees(repo)
        .into_iter()
        .map(|w| canon(&w.path))
        .collect::<Vec<_>>();
    let refuse = |why: String, how: &str| {
        Err(Stop::Refused(format!(
            "❌ Mandate 97: {why}\n   {how}\n   (a human who deliberately wants a shared environment sets SUSI_ALLOW_SHARED_HOME=1)"
        )))
    };
    let home_var = std::env::var("SUSI_HOME").unwrap_or_default();
    if home_var.is_empty() {
        return refuse(
            "SUSI_HOME is unset, so every Susi program falls back to the shared default home".into(),
            "set your own before the first build, test or run: export SUSI_HOME=<your agent root>/susi-home",
        );
    }
    if !Path::new(&home_var).is_absolute() {
        return refuse(
            format!("SUSI_HOME='{home_var}' is relative; only an absolute SUSI_HOME is read"),
            "set an absolute path under your agent root",
        );
    }
    let home = canon(Path::new(&home_var));
    for shared in default_homes() {
        if under(&home, &canon(&shared)) {
            return refuse(
                format!(
                    "SUSI_HOME='{home_var}' is the shared default {} or inside it",
                    shared.display()
                ),
                "one home per running instance, under your agent root",
            );
        }
    }
    if let Some(checkout) = checkouts.iter().find(|c| under(&home, c)) {
        return refuse(
            format!(
                "SUSI_HOME='{home_var}' is inside the checkout {}",
                checkout.display()
            ),
            "put your home under your agent root, outside every checkout",
        );
    }
    let marker = home.join(".agent-isolation");
    if let Ok(text) = std::fs::read_to_string(&marker) {
        let first = text.lines().next().unwrap_or("");
        let claimed = PathBuf::from(first.strip_prefix("worktree=").unwrap_or(first));
        if !claimed.as_os_str().is_empty()
            && !same_path(&claimed, &top)
            && checkouts.iter().any(|c| same_path(c, &claimed))
        {
            return refuse(
                format!(
                    "SUSI_HOME='{home_var}' was claimed by the worktree {}, which still exists; this one is {}",
                    claimed.display(),
                    top.display()
                ),
                "one home per worktree and per running instance: give this one its own under your agent root",
            );
        }
    }
    for var in [
        "CARGO_TARGET_DIR",
        "CARGO_BUILD_TARGET_DIR",
        "TMPDIR",
        "TEMP",
        "TMP",
    ] {
        let Ok(value) = std::env::var(var) else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        let path = Path::new(&value);
        let target = var.starts_with("CARGO_");
        if target && !path.is_absolute() {
            return refuse(
                format!(
                    "{var}='{value}' is relative; cargo resolves it against where the build runs"
                ),
                "set an absolute folder under your agent root, outside every checkout",
            );
        }
        let resolved = canon(&if path.is_absolute() {
            path.to_path_buf()
        } else {
            top.join(path)
        });
        if let Some(checkout) = checkouts.iter().find(|c| under(&resolved, c)) {
            return refuse(
                format!(
                    "{var}='{value}' is inside the checkout {}",
                    checkout.display()
                ),
                "use your own folder under your agent root, outside every checkout",
            );
        }
    }
    if home.is_dir() {
        let temp = home.join(format!(".agent-isolation.tmp.{}", std::process::id()));
        let written = std::fs::write(&temp, format!("worktree={}\n", top.display()))
            .and_then(|()| std::fs::rename(&temp, &marker));
        if written.is_err() {
            let _ = std::fs::remove_file(&temp);
        }
    }
    Ok(())
}

/// The homes every un-isolated instance shares: `~/.susi` and the per-user config folder.
fn default_homes() -> Vec<PathBuf> {
    let home = std::env::var_os("HOME")
        .or_else(|| std::env::var_os("USERPROFILE"))
        .filter(|h| !h.is_empty())
        .map(PathBuf::from);
    let mut homes = Vec::new();
    if let Some(home) = &home {
        homes.push(home.join(".susi"));
    }
    if cfg!(windows) {
        if let Some(appdata) = std::env::var_os("APPDATA").filter(|v| !v.is_empty()) {
            homes.push(PathBuf::from(appdata).join("intellibitz/susi/config"));
        }
    } else if cfg!(target_os = "macos") {
        if let Some(home) = &home {
            homes.push(home.join("Library/Application Support/intellibitz.susi"));
        }
    } else if let Some(config) = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|v| !v.is_empty())
        .map(PathBuf::from)
        .or_else(|| home.as_ref().map(|h| h.join(".config")))
    {
        homes.push(config.join("susi"));
    }
    homes
}

fn canon(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

/// Paths compared as the host compares them: Windows folds case.
fn key(path: &Path) -> String {
    let text = path.to_string_lossy().replace('\\', "/");
    let text = text.trim_end_matches('/').to_owned();
    if cfg!(windows) {
        text.to_lowercase()
    } else {
        text
    }
}

fn same_path(a: &Path, b: &Path) -> bool {
    key(&canon(a)) == key(&canon(b))
}

/// `path` is `dir` or lies inside it.
fn under(path: &Path, dir: &Path) -> bool {
    let (path, dir) = (key(path), key(dir));
    !dir.is_empty() && (path == dir || path.starts_with(&format!("{dir}/")))
}

// --- commit-msg -----------------------------------------------------------------------------

fn commit_msg(repo: &Repo, message: &str) -> Result<(), Stop> {
    if repo.git_dir().join("MERGE_HEAD").is_file() {
        return Ok(());
    }
    let message: String = message
        .lines()
        .filter(|l| !l.starts_with('#'))
        .map(|l| format!("{l}\n"))
        .collect();
    let subject = message
        .lines()
        .find(|l| !l.trim().is_empty())
        .unwrap_or("")
        .to_owned();
    if ["Merge ", "Revert ", "fixup! ", "squash! ", "amend! "]
        .iter()
        .any(|p| subject.starts_with(p))
    {
        return Ok(());
    }
    let staged: Vec<String> = repo
        .read(&["diff", "--cached", "--name-only", "--no-renames"])?
        .lines()
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    if !staged.is_empty() && staged.iter().all(|p| p.starts_with(".agents/tasks/")) {
        return Ok(());
    }
    if subject.chars().count() > 72 {
        warn(&format!(
            "Mandate 94: the subject is {} characters; keep it to 72 or fewer (advice).",
            subject.chars().count()
        ));
    }
    if subject.ends_with('.') {
        warn("Mandate 94: the subject ends with a period; drop it (advice).");
    }
    if matches!(
        subject.trim().to_lowercase().as_str(),
        "wip" | "update" | "wip:" | "update:"
    ) {
        return Err(Stop::Refused(format!(
            "❌ Mandate 94: say what the commit does, not \"{subject}\"."
        )));
    }
    let tasks: Vec<String> = hooks::tasks_in_message(&message)
        .into_iter()
        .filter(|id| task_id_shaped(id))
        .collect();
    if tasks.is_empty() {
        return Err(Stop::Refused(
            "❌ Mandate 50: this commit names no task. End the message with a trailer:\n     Task: T-<AGENT>-<n>   (a task recorded or claimed here: susi-tools tasks start ...)".into(),
        ));
    }
    let work = repo.work_tree()?;
    let me = crate::agent::name(None, repo)
        .ok()
        .and_then(|n| crate::agent::token(&n).ok());
    for id in &tasks {
        let recorded = work
            .join(".agents/tasks")
            .join(format!("{id}.json"))
            .is_file();
        let claim = claim_here(repo, id);
        if !recorded && claim.is_none() {
            return Err(Stop::Refused(format!(
                "❌ Mandate 50: {id} is neither an open task in this checkout nor claimed on this repository. Claim it here first (susi-tools tasks claim {id} --scope <path>), or merge origin/main if its record is new."
            )));
        }
        if let Some(claim) = claim {
            let owner = crate::agent::token(&claim.agent).ok();
            if let (Some(owner), Some(me)) = (&owner, &me) {
                if owner != me {
                    return Err(Stop::Refused(format!(
                        "❌ {id} is claimed by {}, not by you ({me}): ask on the Board, or work on a task of your own.",
                        claim.agent
                    )));
                }
            }
            scope(id, &claim.scopes, &staged)?;
        }
    }
    hooks::reuse(work, &tasks)
}

/// `T-<LETTERS AND DIGITS>-<digits>`.
fn task_id_shaped(id: &str) -> bool {
    let Some(rest) = id.strip_prefix("T-") else {
        return false;
    };
    let Some((agent, number)) = rest.rsplit_once('-') else {
        return false;
    };
    !agent.is_empty()
        && agent
            .bytes()
            .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
        && !number.is_empty()
        && number.bytes().all(|b| b.is_ascii_digit())
}

/// The claim on `task` as this checkout last saw the remote's claims (no fetch: every
/// `susi-tools` command, and this hook's freshness check, refreshes them).
fn claim_here(repo: &Repo, task: &str) -> Option<crate::claims::Claim> {
    let oid = repo.try_read(&[
        "rev-parse",
        "-q",
        "--verify",
        &format!("refs/susi-tools/remote/claims/{task}"),
    ])?;
    let blob = repo.blobs(&[oid.trim().to_owned()]).ok()?.pop()??;
    crate::claims::Claim::parse(&blob)
}

/// Every staged file except task records lies inside one of the claim's scopes.
fn scope(task: &str, scopes: &[String], staged: &[String]) -> Result<(), Stop> {
    let code: Vec<&String> = staged
        .iter()
        .filter(|p| !p.starts_with(".agents/tasks/"))
        .collect();
    if code.is_empty() {
        return Ok(());
    }
    if scopes.is_empty() {
        return Err(Stop::Refused(format!(
            "❌ {task} reserves no paths: release it and claim it again with a scope that covers this change (susi-tools tasks claim {task} --scope <path>)."
        )));
    }
    let outside: Vec<&str> = code
        .iter()
        .filter(|p| {
            !scopes.iter().any(|s| {
                let s = s.trim_end_matches('/');
                p.as_str() == s || p.starts_with(&format!("{s}/"))
            })
        })
        .map(|p| p.as_str())
        .collect();
    if !outside.is_empty() {
        return Err(Stop::Refused(format!(
            "❌ files outside the scopes {task} claims ({}): {}",
            scopes.join(", "),
            outside.join(", ")
        )));
    }
    Ok(())
}

// --- pre-push -------------------------------------------------------------------------------

fn pre_push(repo: &Repo, records: &str) -> Result<(), Stop> {
    for line in records.lines().filter(|l| !l.trim().is_empty()) {
        let fields: Vec<&str> = line.split_whitespace().collect();
        let [_local_ref, local, remote_ref, remote] = fields[..] else {
            return Err(Stop::Refused(format!(
                "❌ malformed pre-push record: {line}"
            )));
        };
        push_rule(repo, local, remote_ref, remote)
            .map_err(|why| Stop::Refused(format!("❌ pre-push: {why}")))?;
    }
    Ok(())
}

fn object_type(repo: &Repo, oid: &str) -> String {
    repo.try_read(&["cat-file", "-t", oid])
        .map(|t| t.trim().to_owned())
        .unwrap_or_default()
}

fn contains(repo: &Repo, ancestor: &str, tip: &str) -> bool {
    repo.read(&["merge-base", "--is-ancestor", ancestor, tip])
        .is_ok()
}

/// The rule for one pushed ref.
fn push_rule(repo: &Repo, local: &str, remote_ref: &str, remote: &str) -> Result<(), String> {
    let deleting = is_zero(local);
    let fast_forward = || {
        if is_zero(remote) || contains(repo, remote, local) {
            Ok(())
        } else {
            Err(format!(
                "{local} does not contain {remote_ref} at {remote} (no force pushes; merge it first)"
            ))
        }
    };
    if remote_ref == "refs/heads/main" {
        if deleting {
            return Err("main is never deleted".into());
        }
        return fast_forward();
    }
    if let Some(name) = remote_ref.strip_prefix("refs/agents/") {
        if name.is_empty() || name.contains('/') {
            return Err(format!(
                "{remote_ref}: an agent publishes to refs/agents/<name>, one segment"
            ));
        }
        if deleting {
            return Ok(());
        }
        if object_type(repo, local) != "commit" {
            return Err(format!("{remote_ref} must name a commit"));
        }
        return fast_forward();
    }
    if let Some(tag) = remote_ref.strip_prefix("refs/tags/") {
        if deleting || !is_zero(remote) {
            return Err(format!(
                "tags are created once and never moved or deleted: {remote_ref}"
            ));
        }
        let mut chars = tag.chars();
        if !(chars.next() == Some('v') && chars.next().is_some_and(|c| c.is_ascii_digit())) {
            return Err(format!("{remote_ref} is not a release tag vX.Y.Z"));
        }
        if object_type(repo, &format!("{local}^{{commit}}")) != "commit" {
            return Err(format!("{remote_ref} must name a commit"));
        }
        return Ok(());
    }
    if QUEUE_NAMESPACES.iter().any(|ns| remote_ref.starts_with(ns)) {
        if !deleting && object_type(repo, local) != "blob" {
            return Err(format!("task record {remote_ref} must name a blob"));
        }
        return Ok(());
    }
    if remote_ref.starts_with("refs/board/") {
        // A public repository's server refuses every Board ref; here only their shape is checked.
        let number = remote_ref.strip_prefix("refs/board/items/").unwrap_or("");
        if number.is_empty()
            || number.starts_with('0')
            || !number.bytes().all(|b| b.is_ascii_digit())
        {
            return Err(format!(
                "Board refs are refs/board/items/<n> only: {remote_ref}"
            ));
        }
        if deleting || !is_zero(remote) {
            return Err(format!("the Board is append-only: {remote_ref}"));
        }
        if object_type(repo, local) != "blob" {
            return Err(format!("Board item {remote_ref} must name a blob"));
        }
        return Ok(());
    }
    if remote_ref.starts_with("refs/heads/") {
        return Err(format!(
            "{remote_ref}: only main reaches the remote as a branch; publish your work to refs/agents/<name>"
        ));
    }
    Err(format!("unsupported ref namespace: {remote_ref}"))
}

// --- post-checkout (Mandate 98) -------------------------------------------------------------

struct Worktree {
    path: PathBuf,
    head: String,
    branch: Option<String>,
}

fn worktrees(repo: &Repo) -> Vec<Worktree> {
    let text = repo
        .try_read(&["worktree", "list", "--porcelain"])
        .unwrap_or_default();
    let mut list = Vec::new();
    for block in text.split("\n\n") {
        let mut tree = Worktree {
            path: PathBuf::new(),
            head: String::new(),
            branch: None,
        };
        for line in block.lines() {
            if let Some(path) = line.strip_prefix("worktree ") {
                tree.path = PathBuf::from(path);
            } else if let Some(head) = line.strip_prefix("HEAD ") {
                tree.head = head.to_owned();
            } else if let Some(branch) = line.strip_prefix("branch refs/heads/") {
                tree.branch = Some(branch.to_owned());
            }
        }
        if !tree.path.as_os_str().is_empty() {
            list.push(tree);
        }
    }
    list
}

/// After `git worktree add`, the clean worktrees whose branch is fully on the remote's main:
/// finished work to remove before starting fresh (git cannot refuse a checkout here).
fn advise_finished_worktrees(repo: &Repo) {
    let main = [crate::tasks::MAIN_SNAPSHOT, "refs/remotes/origin/main"]
        .into_iter()
        .find(|r| repo.try_read(&["rev-parse", "-q", "--verify", r]).is_some());
    let (Some(main), Ok(here)) = (main, repo.work_tree()) else {
        return;
    };
    let finished: Vec<String> = worktrees(repo)
        .into_iter()
        .skip(1)
        .filter(|w| !same_path(&w.path, here))
        .filter_map(|w| {
            let branch = w.branch.clone().filter(|b| b != "main")?;
            if !contains(repo, &w.head, main) {
                return None;
            }
            let clean = Repo::open(&w.path)
                .ok()
                .and_then(|r| r.try_read(&["status", "--porcelain"]))
                .is_some_and(|s| s.trim().is_empty());
            clean.then(|| format!("    {} ({branch})", w.path.display()))
        })
        .collect();
    if finished.is_empty() {
        return;
    }
    eprintln!(
        "⚠ Mandate 98: remove finished worktrees before starting a fresh one. These are clean and fully on main (check they are yours first):\n{}\n  Remove one with: git worktree remove <path> && git branch -d <branch>",
        finished.join("\n")
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secret_shapes_are_found_and_their_names_alone_are_not() {
        let none: &[String] = &[];
        // Assembled here, so this public source holds no key header of its own.
        let header = format!("-----BEGIN {}", "PRIVATE KEY-----");
        let typed = format!("-----BEGIN OPENSSH {}", "PRIVATE KEY-----");
        assert!(secret_shaped(&header, none));
        assert!(secret_shaped(&format!("  {typed}  "), none));
        assert!(!secret_shaped(&format!("let header = \"{header}\";"), none));
        assert!(secret_shaped(
            &format!("token = ghp_{}", "a".repeat(36)),
            none
        ));
        assert!(!secret_shaped(&format!("ghp_{}", "a".repeat(35)), none));
        assert!(secret_shaped(&format!("AKIA{}", "Z9".repeat(8)), none));
        assert!(secret_shaped(&format!("sk-ant-{}", "x-".repeat(15)), none));
        assert!(secret_shaped(
            &format!("github_pat_{}", "a_".repeat(20)),
            none
        ));
        let acme = ["ACME-LIC-v1".to_owned()];
        let licence = format!("ACME-LIC-v1.{}", "A".repeat(40));
        assert!(secret_shaped(&licence, &acme));
        assert!(
            !secret_shaped(&licence, none),
            "only the repository's own prefixes"
        );
        assert!(!secret_shaped("ACME-LIC-v1.aa.bb", &acme));
    }

    #[test]
    fn only_a_real_dbg_call_counts() {
        assert!(has_dbg("    dbg!(x);"));
        assert!(has_dbg("let y = (dbg!(x));"));
        assert!(!has_dbg("my_dbg!(x)"));
        assert!(!has_dbg("// say dbg! without a call"));
    }

    #[test]
    fn added_lines_leave_out_vendored_code_and_fixtures() {
        let diff = "+++ b/src/a.rs\n+one\n+++ b/vendor/x/b.rs\n+two\n+++ b/c/tests/fixtures/k.pem\n+three\n";
        let lines = added_lines(diff);
        assert_eq!(lines, [("src/a.rs".to_owned(), "one".to_owned())]);
    }

    #[test]
    fn task_ids_have_their_shape() {
        assert!(task_id_shaped("T-CLAUDE-33"));
        assert!(task_id_shaped("T-A1-7"));
        assert!(!task_id_shaped("T-claude-3"));
        assert!(!task_id_shaped("T-A-"));
    }

    #[test]
    fn a_scope_covers_its_folder_and_its_exact_file() {
        let scopes = ["src".to_owned(), "README.md".to_owned()];
        let staged = |p: &[&str]| p.iter().map(|s| (*s).to_owned()).collect::<Vec<_>>();
        assert!(scope("T-A-1", &scopes, &staged(&["src/x.rs", "README.md"])).is_ok());
        assert!(scope("T-A-1", &scopes, &staged(&["srcx/y.rs"])).is_err());
        assert!(scope("T-A-1", &[], &staged(&[".agents/tasks/T-A-1.json"])).is_ok());
        assert!(scope("T-A-1", &[], &staged(&["src/x.rs"])).is_err());
    }
}

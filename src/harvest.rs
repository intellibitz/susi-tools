// SPDX-License-Identifier: Apache-2.0
//! `harvest audit`: the legal ledger of harvested Apache-2.0 code, checked against Git.
//!
//! The same checks, messages and output as the audit of the earlier `local-harvest` program
//! (Susi's `tools/core-xtask/src/harvest.rs` and its `harvest_retired.rs` and
//! `harvest_corrections.rs`, commit `5404fd12`), rebuilt on 2026-11-22. That program started one
//! `git` process per question, 89 for Susi's ledger, and took 4.5 s on Windows for 0.5 s of Git
//! work; this one keeps one `cat-file --batch` running for every tree lookup and blob, hashes
//! every file with one `hash-object --stdin-paths`, and asks each history question with one
//! `log`, so the same audit starts about a dozen processes. Checked:
//!
//! - each live manifest (`.agents/harvest/<ID>.json`): its shape; its license evidence and source
//!   files are the recorded blobs at the pinned upstream commit, with the recorded SHA-256; every
//!   destination file exists, carries an Apache-2.0 4(b) "Modified by" notice when it differs
//!   from upstream, and keeps every upstream copyright line (4(c));
//! - `THIRD_PARTY_NOTICES` is exactly the notices the manifests render (4(d)), and
//!   `LICENSES/Apache-2.0.txt` is the Apache License 2.0 text (4(a));
//! - each retired record (`.agents/harvest/retired/<ID>.json`): its code is gone, its notice
//!   block is gone, it is not also live, the commit it names removed its last file on the date
//!   it gives, and it is the manifest as it stood before that commit; no manifest ever left the
//!   ledger any other way;
//! - every `Harvest:` citation in the audited range is answered by a live manifest, a valid
//!   retired record or an entry of `.agents/harvest-corrections.json` bound to Git evidence.

use crate::json::{self, Json};
use std::cell::{OnceCell, RefCell};
use std::collections::{BTreeMap, BTreeSet, HashMap, HashSet};
use std::fs;
use std::io::{BufRead as _, BufReader, Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, ChildStdout, Command, Output, Stdio};

type Result<T> = std::result::Result<T, String>;

const MANIFEST_SCHEMA: &str = "susi-harvest-manifest-v1";
const MANIFEST_DIR: &str = ".agents/harvest";
const RETIRED_DIR: &str = ".agents/harvest/retired";
const NOTICES_FILE: &str = "THIRD_PARTY_NOTICES";
const APACHE_TEXT_PATH: &str = "LICENSES/Apache-2.0.txt";
const CORRECTIONS_PATH: &str = ".agents/harvest-corrections.json";
const CORRECTIONS_SCHEMA: &str = "susi-harvest-trailer-corrections-v1";
const NOTE: &str = "retired";
const NOTE_FIELDS: [&str; 3] = ["removed_on", "removed_by_commit", "reason"];
const MAX_REASON_CHARS: usize = 400;
const REJECT_PHRASES: [&str; 7] = [
    "GNU General Public License",
    "GNU Lesser General Public",
    "GNU Affero",
    "Mozilla Public License",
    "Server Side Public License",
    "Business Source License",
    "Commons Clause",
];

/// What one audit found: whether it passed, and its text.
pub struct Report {
    pub ok: bool,
    pub text: String,
}

// ---------------------------------------------------------------- git, in batches

struct Git {
    root: PathBuf,
    ancestors: OnceCell<HashSet<String>>,
    /// One `git cat-file --batch` for every lookup and blob of the audit: on Windows each process
    /// start costs tens of milliseconds, more than the questions themselves.
    objects: RefCell<Option<Batch>>,
    /// Tree objects already read, by id, and each revision's root tree: a `<rev>:<path>` lookup
    /// walks these instead of asking Git to walk the trees again for every path.
    trees: RefCell<HashMap<String, Entries>>,
    roots: RefCell<HashMap<String, Option<String>>>,
}

/// A running `git cat-file --batch`, asked one object at a time.
struct Batch {
    child: Child,
    input: ChildStdin,
    output: BufReader<ChildStdout>,
}

impl Drop for Batch {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

/// One object lookup: its id and type, or nothing when Git has no such object.
type Found = Option<(String, String)>;

/// A tree's entries (name, mode, id), shared from the cache.
type Entries = std::rc::Rc<Vec<(String, String, String)>>;

impl Git {
    fn open(cwd: &Path) -> Result<Git> {
        let probe = Git {
            root: cwd.to_path_buf(),
            ancestors: OnceCell::new(),
            objects: RefCell::new(None),
            trees: RefCell::new(HashMap::new()),
            roots: RefCell::new(HashMap::new()),
        };
        let top = probe
            .text(&["rev-parse", "--show-toplevel"])
            .map_err(|e| format!("not inside a Git worktree: {e}"))?;
        Ok(Git {
            root: PathBuf::from(top),
            ancestors: OnceCell::new(),
            objects: RefCell::new(None),
            trees: RefCell::new(HashMap::new()),
            roots: RefCell::new(HashMap::new()),
        })
    }

    fn output(&self, args: &[&str], input: Option<&[u8]>) -> Result<Output> {
        let mut command = Command::new(crate::git_program::path());
        command
            .current_dir(&self.root)
            .arg("--literal-pathspecs")
            .arg("--no-replace-objects")
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .stdin(if input.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            });
        for key in [
            "GIT_DIR",
            "GIT_WORK_TREE",
            "GIT_COMMON_DIR",
            "GIT_INDEX_FILE",
            "GIT_OBJECT_DIRECTORY",
            "GIT_ALTERNATE_OBJECT_DIRECTORIES",
            "GIT_GRAFT_FILE",
        ] {
            command.env_remove(key);
        }
        let mut child = command
            .spawn()
            .map_err(|e| format!("cannot start git: {e}"))?;
        let Some(bytes) = input else {
            return child
                .wait_with_output()
                .map_err(|e| format!("cannot wait for git: {e}"));
        };
        let mut stdin = child
            .stdin
            .take()
            .ok_or_else(|| "git stdin unavailable".to_owned())?;
        std::thread::scope(|scope| {
            // Written from its own thread: git answers while it reads.
            let writer = scope.spawn(move || stdin.write_all(bytes));
            let out = child
                .wait_with_output()
                .map_err(|e| format!("cannot wait for git: {e}"))?;
            let written = writer
                .join()
                .map_err(|_| "git stdin writer panicked".to_owned())?;
            if out.status.success() {
                written.map_err(|e| format!("cannot write git input: {e}"))?;
            }
            Ok(out)
        })
    }

    fn checked(&self, args: &[&str], input: Option<&[u8]>) -> Result<Vec<u8>> {
        let out = self.output(args, input)?;
        if out.status.success() {
            return Ok(out.stdout);
        }
        let stderr = String::from_utf8_lossy(&out.stderr);
        let detail = stderr
            .lines()
            .map(str::trim)
            .rfind(|line| !line.is_empty())
            .unwrap_or("no diagnostic");
        Err(format!(
            "`git {}` failed ({}): {detail}",
            args.join(" "),
            out.status
        ))
    }

    fn text(&self, args: &[&str]) -> Result<String> {
        let bytes = self.checked(args, None)?;
        String::from_utf8(bytes)
            .map(|s| s.trim().to_owned())
            .map_err(|_| "git returned non-UTF-8 output".to_owned())
    }

    /// Ask the running `cat-file --batch` for one object: its id, type and bytes, or nothing.
    fn object(&self, query: &str) -> Result<Option<(String, String, Vec<u8>)>> {
        if query.is_empty() || query.contains(['\n', '\r']) {
            return Ok(None);
        }
        let mut slot = self.objects.borrow_mut();
        if slot.is_none() {
            let mut command = Command::new(crate::git_program::path());
            command
                .current_dir(&self.root)
                .args([
                    "--literal-pathspecs",
                    "--no-replace-objects",
                    "cat-file",
                    "--batch",
                ])
                .env("GIT_TERMINAL_PROMPT", "0")
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null());
            for key in [
                "GIT_DIR",
                "GIT_WORK_TREE",
                "GIT_COMMON_DIR",
                "GIT_INDEX_FILE",
                "GIT_OBJECT_DIRECTORY",
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                "GIT_GRAFT_FILE",
            ] {
                command.env_remove(key);
            }
            let mut child = command
                .spawn()
                .map_err(|e| format!("cannot start git: {e}"))?;
            let input = child.stdin.take().ok_or("git stdin unavailable")?;
            let output = BufReader::new(child.stdout.take().ok_or("git stdout unavailable")?);
            *slot = Some(Batch {
                child,
                input,
                output,
            });
        }
        let batch = slot.as_mut().ok_or("git cat-file is not running")?;
        writeln!(batch.input, "{query}").map_err(|e| format!("cannot ask git cat-file: {e}"))?;
        batch
            .input
            .flush()
            .map_err(|e| format!("cannot ask git cat-file: {e}"))?;
        let mut header = String::new();
        batch
            .output
            .read_line(&mut header)
            .map_err(|e| format!("cannot read git cat-file: {e}"))?;
        let parts: Vec<&str> = header.trim_end().split(' ').collect();
        if parts.len() != 3 || !is_hex_oid(parts[0]) {
            // `<query> missing`, `<query> ambiguous`, or a query git could not parse.
            return Ok(None);
        }
        let size: usize = parts[2]
            .parse()
            .map_err(|_| format!("bad object size in `{header}`"))?;
        let mut body = vec![0u8; size + 1];
        batch
            .output
            .read_exact(&mut body)
            .map_err(|e| format!("truncated git cat-file output: {e}"))?;
        body.truncate(size);
        Ok(Some((parts[0].to_owned(), parts[1].to_owned(), body)))
    }

    /// The entries of tree `oid` (name, mode, id), read once.
    fn tree(&self, oid: &str) -> Result<Entries> {
        if let Some(entries) = self.trees.borrow().get(oid) {
            return Ok(entries.clone());
        }
        let Some((_, kind, raw)) = self.object(oid)? else {
            return Err(format!("cannot read tree {oid}"));
        };
        if kind != "tree" {
            return Err(format!("{oid} is a {kind}, not a tree"));
        }
        let id_len = oid.len() / 2;
        let mut entries = Vec::new();
        let mut at = 0;
        while at < raw.len() {
            let space = raw[at..]
                .iter()
                .position(|b| *b == b' ')
                .ok_or("malformed tree")?
                + at;
            let nul = raw[space..]
                .iter()
                .position(|b| *b == 0)
                .ok_or("malformed tree")?
                + space;
            let id = raw.get(nul + 1..nul + 1 + id_len).ok_or("malformed tree")?;
            entries.push((
                String::from_utf8_lossy(&raw[space + 1..nul]).into_owned(),
                String::from_utf8_lossy(&raw[at..space]).into_owned(),
                id.iter().map(|b| format!("{b:02x}")).collect(),
            ));
            at = nul + 1 + id_len;
        }
        let entries = std::rc::Rc::new(entries);
        self.trees
            .borrow_mut()
            .insert(oid.to_owned(), entries.clone());
        Ok(entries)
    }

    /// `<rev>:<path>` by walking cached trees: the object's id and type, or nothing.
    fn at_path(&self, rev: &str, path: &str) -> Result<Found> {
        let cached = self.roots.borrow().get(rev).cloned();
        let root = match cached {
            Some(root) => root,
            None => {
                let found = self
                    .object(&format!("{rev}^{{tree}}"))?
                    .map(|(oid, _, _)| oid);
                self.roots
                    .borrow_mut()
                    .insert(rev.to_owned(), found.clone());
                found
            }
        };
        let Some(mut oid) = root else { return Ok(None) };
        let mut kind = "tree".to_owned();
        for name in path.split('/') {
            if kind != "tree" || name.is_empty() {
                return Ok(None);
            }
            let entries = self.tree(&oid)?;
            let Some((_, mode, id)) = entries.iter().find(|(n, _, _)| n == name) else {
                return Ok(None);
            };
            kind = match mode.as_str() {
                "40000" | "040000" => "tree",
                "160000" => "commit",
                _ => "blob",
            }
            .to_owned();
            oid = id.clone();
        }
        Ok(Some((oid, kind)))
    }

    /// Every query (`<rev>`, `<rev>^{commit}`, `<rev>:<path>`) answered in order.
    fn lookup(&self, queries: &[String]) -> Result<Vec<Found>> {
        queries
            .iter()
            .map(|q| match q.split_once(':') {
                Some((rev, path)) if !rev.is_empty() && !rev.contains(['^', '{']) => {
                    self.at_path(rev, path)
                }
                _ => Ok(self.object(q)?.map(|(oid, kind, _)| (oid, kind))),
            })
            .collect()
    }

    /// Whether each id names a commit of this repository.
    fn commits(&self, oids: &[String]) -> Result<Vec<bool>> {
        let queries: Vec<String> = oids.iter().map(|o| format!("{o}^{{commit}}")).collect();
        Ok(self
            .lookup(&queries)?
            .into_iter()
            .map(|f| f.is_some_and(|(_, kind)| kind == "commit"))
            .collect())
    }

    /// Whether `commit` is `HEAD` or one of its ancestors (one `rev-list` for the whole audit).
    fn in_head_history(&self, commit: &str) -> Result<bool> {
        if self.ancestors.get().is_none() {
            let raw = self.checked(&["rev-list", "HEAD"], None)?;
            let set = String::from_utf8_lossy(&raw)
                .lines()
                .map(|l| l.trim().to_owned())
                .collect();
            let _ = self.ancestors.set(set);
        }
        Ok(self.ancestors.get().is_some_and(|s| s.contains(commit)))
    }

    fn read_blobs(&self, oids: &BTreeSet<String>) -> Result<BTreeMap<String, Vec<u8>>> {
        let mut blobs = BTreeMap::new();
        for oid in oids {
            match self.object(oid)? {
                Some((_, kind, body)) if kind == "blob" => {
                    blobs.insert(oid.clone(), body);
                }
                _ => return Err(format!("cannot read blob {oid}")),
            }
        }
        Ok(blobs)
    }

    /// Blob ids of working-tree files, with (`filters`) and without clean filters.
    fn hash_paths(&self, paths: &[String], filters: bool) -> Result<Vec<String>> {
        if paths.is_empty() {
            return Ok(Vec::new());
        }
        let input: String = paths.iter().map(|p| format!("{p}\n")).collect();
        let mut args = vec!["hash-object", "--stdin-paths"];
        if !filters {
            args.push("--no-filters");
        }
        let out = self.checked(&args, Some(input.as_bytes()))?;
        let ids: Vec<String> = String::from_utf8_lossy(&out)
            .lines()
            .map(str::to_owned)
            .collect();
        if ids.len() != paths.len() {
            return Err("git hash-object returned an unexpected number of ids".to_owned());
        }
        Ok(ids)
    }

    /// Every blob and link of `commit`'s tree, by path (`ls-tree -r`).
    fn list_tree(&self, commit: &str) -> Result<BTreeMap<String, TreeEntry>> {
        let raw = self.checked(&["ls-tree", "-r", "-z", "--full-tree", commit], None)?;
        let mut tree = BTreeMap::new();
        for record in raw.split(|b| *b == 0).filter(|r| !r.is_empty()) {
            let line = std::str::from_utf8(record)
                .map_err(|_| "upstream tree has a non-UTF-8 path; refusing".to_owned())?;
            let (meta, path) = line
                .split_once('\t')
                .ok_or_else(|| format!("unexpected ls-tree record `{line}`"))?;
            let mut parts = meta.split(' ');
            let (Some(mode), Some(kind), Some(oid)) = (parts.next(), parts.next(), parts.next())
            else {
                return Err(format!("unexpected ls-tree record `{line}`"));
            };
            tree.insert(
                path.to_owned(),
                TreeEntry {
                    mode: mode.to_owned(),
                    kind: kind.to_owned(),
                    oid: oid.to_owned(),
                },
            );
        }
        Ok(tree)
    }

    /// `Harvest:` trailer ids of every commit in `range` that has any (one `git log`).
    fn harvest_trailers(&self, range: &str) -> Result<Vec<(String, Vec<String>)>> {
        if range.is_empty() || range.starts_with('-') || range.chars().any(char::is_whitespace) {
            return Err(format!("invalid --range `{range}`"));
        }
        let raw = self.checked(
            &[
                "log",
                "-z",
                "--format=%H%n%(trailers:key=Harvest,valueonly)",
                range,
                "--",
            ],
            None,
        )?;
        let mut commits = Vec::new();
        for record in String::from_utf8_lossy(&raw).split('\0') {
            let mut lines = record.lines();
            let Some(commit) = lines.next().map(str::trim).filter(|c| !c.is_empty()) else {
                continue;
            };
            let ids: Vec<String> = lines
                .flat_map(|l| l.split([',', ' ', '\t']))
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(str::to_owned)
                .collect();
            if !ids.is_empty() {
                commits.push((commit.to_owned(), ids));
            }
        }
        Ok(commits)
    }
}

#[derive(Clone)]
struct TreeEntry {
    mode: String,
    kind: String,
    oid: String,
}

// ---------------------------------------------------------------- small rules

fn is_hex_oid(value: &str) -> bool {
    matches!(value.len(), 40 | 64)
        && value
            .bytes()
            .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
}

fn check_labelled_id(id: &str, prefix: &str) -> Result<()> {
    let ok = id
        .strip_prefix(prefix)
        .and_then(|rest| rest.rsplit_once('-'))
        .is_some_and(|(agent, n)| {
            !agent.is_empty()
                && agent
                    .bytes()
                    .all(|b| b.is_ascii_uppercase() || b.is_ascii_digit())
                && !n.is_empty()
                && n.bytes().all(|b| b.is_ascii_digit())
        });
    if ok {
        Ok(())
    } else {
        Err(format!("invalid id `{id}`: expected {prefix}<AGENT>-<n>"))
    }
}

fn normalize_rel(raw: &str, dest: bool) -> Result<String> {
    let slashed = raw.replace('\\', "/");
    let path = slashed.trim_end_matches('/');
    if path.is_empty() || path == "." {
        return if dest {
            Err("destination path is empty".to_owned())
        } else {
            Ok(String::new())
        };
    }
    let bad = path.starts_with('/')
        || path.chars().any(char::is_control)
        || (dest && path.contains(':'))
        || path.split('/').any(|c| {
            c.is_empty() || c == "." || c == ".." || (dest && c.eq_ignore_ascii_case(".git"))
        });
    if bad {
        return Err(format!(
            "path `{raw}` must be a confined relative path without `..`"
        ));
    }
    Ok(path.to_owned())
}

/// Lowercase alphanumeric words separated by single spaces.
fn prose(text: &str) -> String {
    let mapped: String = text
        .chars()
        .map(|c| {
            if c.is_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                ' '
            }
        })
        .collect();
    mapped.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn classify_license_text(text: &str) -> Option<&'static str> {
    let words = prose(text);
    let padded = format!(" {words} ");
    if REJECT_PHRASES
        .iter()
        .any(|phrase| padded.contains(&format!(" {} ", prose(phrase))))
    {
        return None;
    }
    if words.contains("apache license") && words.contains("version 2 0 january 2004") {
        Some("Apache-2.0")
    } else if words.contains("permission is hereby granted free of charge") {
        Some("MIT")
    } else if words
        .contains("permission to use copy modify and or distribute this software for any purpose")
    {
        Some("ISC")
    } else if words.contains("redistribution and use in source and binary forms") {
        if words.contains("neither the name") {
            Some("BSD-3-Clause")
        } else {
            Some("BSD-2-Clause")
        }
    } else {
        None
    }
}

fn strip_comment(line: &str) -> &str {
    let mut s = line
        .trim()
        .trim_start_matches(|c: char| "/*#;!%-'\"<>".contains(c) || c.is_whitespace());
    loop {
        let before = s.len();
        s = s.trim_end();
        for suffix in ["*/", "-->", "*)"] {
            s = s.strip_suffix(suffix).unwrap_or(s);
        }
        if s.len() == before {
            return s;
        }
    }
}

fn is_copyright_line(line: &str) -> bool {
    let rest = if let Some(rest) = line.strip_prefix('©') {
        rest
    } else {
        match line.get(..9) {
            Some(word) if word.eq_ignore_ascii_case("copyright") => &line[9..],
            _ => return false,
        }
    };
    if !rest.is_empty() && !rest.starts_with([' ', '\t', ':', '(', '©']) {
        return false;
    }
    let mut body = rest.trim_start_matches([' ', '\t', ':']);
    loop {
        let before = body.len();
        for mark in ["(c)", "(C)", "©"] {
            body = body.strip_prefix(mark).unwrap_or(body);
        }
        body = body.trim_start();
        if body.len() == before {
            break;
        }
    }
    body.chars()
        .next()
        .is_some_and(|c| c.is_ascii_digit() || c.is_uppercase())
}

fn copyright_lines(text: &str) -> Vec<String> {
    let set: BTreeSet<String> = text
        .lines()
        .map(strip_comment)
        .filter(|line| is_copyright_line(line))
        .map(str::to_owned)
        .collect();
    set.into_iter().collect()
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn normalized_disk_text(path: &Path) -> Option<String> {
    fs::read(path)
        .ok()
        .map(|bytes| lossy(&bytes).replace("\r\n", "\n"))
}

fn has_modified_notice(text: &str) -> bool {
    text.lines()
        .any(|line| line.to_lowercase().contains("modified by"))
}

/// JSON equality with object keys in any order, as the earlier audit compared records.
fn same_json(a: &Json, b: &Json) -> bool {
    match (a, b) {
        (Json::Object(x), Json::Object(y)) => {
            x.len() == y.len()
                && x.iter().all(|(k, v)| {
                    y.iter()
                        .filter(|(k2, _)| k2 == k)
                        .exactly_one()
                        .is_some_and(|(_, v2)| same_json(v, v2))
                })
        }
        (Json::Array(x), Json::Array(y)) => {
            x.len() == y.len() && x.iter().zip(y).all(|(v, w)| same_json(v, w))
        }
        _ => a == b,
    }
}

trait ExactlyOne: Iterator + Sized {
    fn exactly_one(mut self) -> Option<Self::Item> {
        let first = self.next()?;
        self.next().is_none().then_some(first)
    }
}
impl<I: Iterator> ExactlyOne for I {}

fn duplicate_key(value: &Json) -> bool {
    match value {
        Json::Object(fields) => {
            let mut seen = BTreeSet::new();
            fields
                .iter()
                .any(|(k, v)| !seen.insert(k) || duplicate_key(v))
        }
        Json::Array(items) => items.iter().any(duplicate_key),
        _ => false,
    }
}

fn read_json(path: &Path, ctx: &str) -> Result<Json> {
    let bytes = fs::read(path).map_err(|e| format!("{ctx}: {e}"))?;
    json::parse(&lossy(&bytes)).map_err(|e| format!("{ctx}: {e}"))
}

// ---------------------------------------------------------------- manifests

struct ManifestFile {
    source_path: String,
    dest_path: String,
    blob: String,
    copyright: Vec<String>,
}

struct Manifest {
    id: String,
    url: String,
    commit: String,
    license: String,
    evidence: Vec<(String, String, String)>,
    license_texts: Vec<(String, String, String)>,
    notice_text: String,
    copyright: Vec<String>,
    files: Vec<ManifestFile>,
}

fn str_field(value: &Json, key: &str, ctx: &str, errors: &mut Vec<String>) -> String {
    match value.get(key).and_then(Json::as_str) {
        Some(s) => s.to_owned(),
        None => {
            errors.push(format!("{ctx}: missing string field `{key}`"));
            String::new()
        }
    }
}

fn str_list(value: &Json, key: &str, ctx: &str, errors: &mut Vec<String>) -> Vec<String> {
    match value.get(key) {
        Some(Json::Array(items)) => items
            .iter()
            .filter_map(|item| {
                let s = item.as_str().map(str::to_owned);
                if s.is_none() {
                    errors.push(format!("{ctx}: `{key}` must hold strings"));
                }
                s
            })
            .collect(),
        _ => {
            errors.push(format!("{ctx}: missing array field `{key}`"));
            Vec::new()
        }
    }
}

fn parse_manifest(name: &str, v: &Json) -> std::result::Result<Manifest, Vec<String>> {
    let mut errors = Vec::new();
    let e = &mut errors;
    let ctx = format!("{MANIFEST_DIR}/{name}");
    if v.get("schema").and_then(Json::as_str) != Some(MANIFEST_SCHEMA) {
        e.push(format!("{ctx}: schema must be {MANIFEST_SCHEMA}"));
    }
    let id = str_field(v, "id", &ctx, e);
    if let Err(err) = check_labelled_id(&id, "H-") {
        e.push(format!("{ctx}: {err}"));
    }
    if format!("{id}.json") != name {
        e.push(format!("{ctx}: id `{id}` does not match the file name"));
    }
    let task = str_field(v, "task", &ctx, e);
    if let Err(err) = check_labelled_id(&task, "T-") {
        e.push(format!("{ctx}: {err}"));
    }
    if !v
        .get("feature")
        .is_some_and(|f| matches!(f, Json::Null | Json::Str(_)))
    {
        e.push(format!("{ctx}: `feature` must be a string or null"));
    }
    let url = str_field(v, "source_url", &ctx, e);
    if url.is_empty() {
        e.push(format!("{ctx}: empty source_url"));
    }
    let commit = str_field(v, "source_commit", &ctx, e);
    if !is_hex_oid(&commit) {
        e.push(format!("{ctx}: source_commit is not a full commit id"));
    }
    let mode = str_field(v, "mode", &ctx, e);
    if mode != "transplant" && mode != "port" {
        e.push(format!("{ctx}: mode must be transplant or port"));
    }
    let license = str_field(v, "license", &ctx, e);
    if license != "Apache-2.0" {
        e.push(format!(
            "{ctx}: license `{license}` is not Apache-2.0; Pure Apache-2.0 Compliance (Mandate 75) requires `\"Apache-2.0\"`"
        ));
    }
    let notice_text = str_field(v, "notice_text", &ctx, e);
    let copyright = str_list(v, "copyright_lines", &ctx, e);
    str_field(v, "harvested_by", &ctx, e);
    let unix_ok =
        matches!(v.get("harvested_unix"), Some(Json::Number(n)) if n.parse::<u64>().is_ok());
    if !unix_ok {
        e.push(format!("{ctx}: missing integer `harvested_unix`"));
    }
    let evidence = match v.get("license_evidence") {
        Some(Json::Array(items)) => items
            .iter()
            .map(|item| {
                let path = str_field(item, "path", &ctx, e);
                let blob = str_field(item, "blob_sha1", &ctx, e);
                let sha = str_field(item, "sha256", &ctx, e);
                if !is_hex_oid(&blob) || sha.len() != 64 || !is_hex_oid(&sha) {
                    e.push(format!("{ctx}: evidence `{path}` has malformed hashes"));
                }
                (path, blob, sha)
            })
            .collect(),
        _ => {
            e.push(format!("{ctx}: missing array `license_evidence`"));
            Vec::new()
        }
    };
    let license_texts = match v.get("license_texts") {
        Some(Json::Array(items)) => items
            .iter()
            .map(|t| {
                (
                    str_field(t, "path", &ctx, e),
                    str_field(t, "spdx", &ctx, e),
                    str_field(t, "text", &ctx, e),
                )
            })
            .collect(),
        _ => Vec::new(),
    };
    let items: &[Json] = match v.get("files") {
        Some(Json::Array(items)) if !items.is_empty() => items,
        Some(Json::Array(items)) => {
            e.push(format!("{ctx}: `files` must be a non-empty array"));
            items
        }
        _ => {
            e.push(format!("{ctx}: `files` must be a non-empty array"));
            &[]
        }
    };
    let files = items
        .iter()
        .map(|f| {
            let source_path = str_field(f, "source_path", &ctx, e);
            let dest_path = str_field(f, "dest_path", &ctx, e);
            if normalize_rel(&dest_path, true).as_deref() != Ok(dest_path.as_str()) {
                e.push(format!(
                    "{ctx}: dest_path `{dest_path}` is not a confined relative path"
                ));
            }
            let blob = str_field(f, "source_blob_sha1", &ctx, e);
            if !is_hex_oid(&blob) {
                e.push(format!(
                    "{ctx}: `{source_path}` has a malformed source_blob_sha1"
                ));
            }
            let spdx = str_field(f, "spdx", &ctx, e);
            if spdx != "Apache-2.0" {
                e.push(format!(
                    "{ctx}: `{source_path}` license `{spdx}` is not Apache-2.0; Pure Apache-2.0 Compliance (Mandate 75) requires `\"Apache-2.0\"`"
                ));
            }
            ManifestFile {
                source_path,
                dest_path,
                blob,
                copyright: str_list(f, "copyright_lines", &ctx, e),
            }
        })
        .collect();
    if errors.is_empty() {
        Ok(Manifest {
            id,
            url,
            commit,
            license,
            evidence,
            license_texts,
            notice_text,
            copyright,
            files,
        })
    } else {
        Err(errors)
    }
}

/// Every `*.json` in `dir`, by name, with its parsed JSON.
fn json_files(root: &Path, dir: &str) -> Result<Vec<(String, Result<Json>)>> {
    let Ok(entries) = fs::read_dir(root.join(dir)) else {
        return Ok(Vec::new());
    };
    let mut out = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|e| format!("cannot list {dir}: {e}"))?;
        let name = entry.file_name().to_string_lossy().into_owned();
        if !name.ends_with(".json") || !entry.path().is_file() {
            continue;
        }
        let parsed = read_json(&entry.path(), &format!("{dir}/{name}"));
        out.push((name, parsed));
    }
    out.sort_by(|a, b| a.0.cmp(&b.0));
    Ok(out)
}

fn load_manifests(root: &Path) -> (Vec<Manifest>, Vec<String>) {
    let mut manifests = Vec::new();
    let mut errors = Vec::new();
    match json_files(root, MANIFEST_DIR) {
        Ok(values) => {
            for (name, parsed) in values {
                match parsed {
                    Ok(value) => match parse_manifest(&name, &value) {
                        Ok(m) => manifests.push(m),
                        Err(errs) => errors.extend(errs),
                    },
                    Err(err) => errors.push(err),
                }
            }
        }
        Err(err) => errors.push(err),
    }
    manifests.sort_by(|a, b| a.id.cmp(&b.id));
    (manifests, errors)
}

fn render_notices(manifests: &[Manifest]) -> String {
    let mut out = String::from(
        "THIRD-PARTY NOTICES\n\n\
         Generated by `local-harvest notices` from .agents/harvest/*.json\n\
         (susi-harvest-manifest-v1). Do not edit by hand; regenerate after every harvest.\n\
         Harvested code keeps its upstream license. The Apache License 2.0 text is in\n\
         LICENSES/Apache-2.0.txt.\n",
    );
    if manifests.is_empty() {
        out.push_str("\nNo harvested third-party code.\n");
    }
    for m in manifests {
        out.push('\n');
        out.push_str(&"=".repeat(78));
        out.push_str(&format!(
            "\n{}\nSource: {}@{}\nLicense: {}\nFiles:\n",
            m.id, m.url, m.commit, m.license
        ));
        let mut files: Vec<&ManifestFile> = m.files.iter().collect();
        files.sort_by(|a, b| (&a.dest_path, &a.source_path).cmp(&(&b.dest_path, &b.source_path)));
        for f in files {
            out.push_str(&format!("  {} -> {}\n", f.source_path, f.dest_path));
        }
        if !m.copyright.is_empty() {
            out.push_str("Copyright notices:\n");
            for line in &m.copyright {
                out.push_str(&format!("  {line}\n"));
            }
        }
        if !m.notice_text.trim().is_empty() {
            out.push_str(&format!("NOTICE:\n{}\n", m.notice_text.trim_end()));
        }
        for (path, spdx, text) in &m.license_texts {
            out.push_str(&format!(
                "License text ({path}, {spdx}):\n{}\n",
                text.trim_end()
            ));
        }
    }
    out
}

/// The live manifests' checks, every Git question asked once for all of them.
fn audit_manifests(
    git: &Git,
    manifests: &[Manifest],
    violations: &mut Vec<String>,
) -> Result<usize> {
    let upstream_known = git.commits(
        &manifests
            .iter()
            .map(|m| m.commit.clone())
            .collect::<Vec<_>>(),
    )?;
    // Every evidence and source path at its manifest's upstream commit, in one lookup.
    let mut queries = Vec::new();
    for (m, known) in manifests.iter().zip(&upstream_known) {
        if *known {
            queries.extend(
                m.evidence
                    .iter()
                    .map(|(p, _, _)| format!("{}:{p}", m.commit)),
            );
            queries.extend(
                m.files
                    .iter()
                    .map(|f| format!("{}:{}", m.commit, f.source_path)),
            );
        }
    }
    let mut answers = git.lookup(&queries)?.into_iter();
    let mut matched: Vec<(Vec<bool>, Vec<bool>)> = Vec::new();
    let mut oids = BTreeSet::new();
    for (m, known) in manifests.iter().zip(&upstream_known) {
        if !known {
            matched.push((Vec::new(), Vec::new()));
            continue;
        }
        let mut ev = Vec::new();
        for (_, blob, _) in &m.evidence {
            let ok = answers
                .next()
                .flatten()
                .is_some_and(|(oid, _)| oid == *blob);
            if ok {
                oids.insert(blob.clone());
            }
            ev.push(ok);
        }
        let mut fl = Vec::new();
        for f in &m.files {
            let ok = answers
                .next()
                .flatten()
                .is_some_and(|(oid, _)| oid == f.blob);
            if ok {
                oids.insert(f.blob.clone());
            }
            fl.push(ok);
        }
        matched.push((ev, fl));
    }
    let blobs = git.read_blobs(&oids)?;
    // Every destination file present on disk, hashed in two calls for all manifests.
    let present: Vec<Vec<bool>> = manifests
        .iter()
        .map(|m| {
            m.files
                .iter()
                .map(|f| git.root.join(&f.dest_path).is_file())
                .collect()
        })
        .collect();
    let dests: Vec<String> = manifests
        .iter()
        .zip(&present)
        .flat_map(|(m, p)| {
            m.files
                .iter()
                .zip(p)
                .filter(|(_, here)| **here)
                .map(|(f, _)| f.dest_path.clone())
        })
        .collect();
    let filtered = git.hash_paths(&dests, true)?;
    let raw = git.hash_paths(&dests, false)?;
    let mut hashed = filtered.into_iter().zip(raw);

    let mut files = 0;
    for ((m, known), ((ev_ok, file_ok), here)) in manifests
        .iter()
        .zip(&upstream_known)
        .zip(matched.iter().zip(&present))
    {
        let mut upstream_copyright: BTreeMap<String, Vec<String>> = BTreeMap::new();
        if *known {
            let recorded: BTreeSet<&str> = m.copyright.iter().map(String::as_str).collect();
            for ((path, _, _), ok) in m.evidence.iter().zip(ev_ok) {
                if !ok {
                    violations.push(format!(
                        "{}: evidence {path} does not match upstream {}",
                        m.id, m.commit
                    ));
                }
            }
            for (f, ok) in m.files.iter().zip(file_ok) {
                if !ok {
                    violations.push(format!(
                        "{}: {} is not blob {} at upstream {}",
                        m.id, f.source_path, f.blob, m.commit
                    ));
                }
            }
            for (path, blob, sha) in &m.evidence {
                if blobs
                    .get(blob)
                    .is_some_and(|b| crate::sha256::of(b) != *sha)
                {
                    violations.push(format!(
                        "{}: sha256 of evidence {path} does not match",
                        m.id
                    ));
                }
            }
            for f in &m.files {
                if let Some(bytes) = blobs.get(&f.blob) {
                    let lines = copyright_lines(&lossy(bytes))
                        .into_iter()
                        .filter(|l| recorded.contains(l.as_str()))
                        .collect();
                    upstream_copyright.insert(f.source_path.clone(), lines);
                }
            }
        }
        for (f, is_here) in m.files.iter().zip(here) {
            if !is_here {
                violations.push(format!("{}: destination {} is missing", m.id, f.dest_path));
            }
        }
        for (f, _) in m.files.iter().zip(here).filter(|(_, h)| **h) {
            let (with_filters, without) = hashed
                .next()
                .ok_or_else(|| "git hash-object answered too few files".to_owned())?;
            let bytes = fs::read(git.root.join(&f.dest_path))
                .map_err(|e| format!("cannot read {}: {e}", f.dest_path))?;
            let text = lossy(&bytes);
            let unmodified = with_filters == f.blob || without == f.blob;
            if !unmodified && !has_modified_notice(&text) {
                violations.push(format!(
                    "{}: {} differs from upstream {} but has no \"Modified by\" notice (Apache-2.0 4(b))",
                    m.id, f.dest_path, f.source_path
                ));
            }
            let mut expected: BTreeSet<&str> = f.copyright.iter().map(String::as_str).collect();
            if let Some(lines) = upstream_copyright.get(&f.source_path) {
                expected.extend(lines.iter().map(String::as_str));
            }
            for line in expected {
                if !text.contains(line) {
                    violations.push(format!(
                        "{}: {} lost upstream copyright line \"{line}\" (Apache-2.0 4(c))",
                        m.id, f.dest_path
                    ));
                }
            }
            files += 1;
        }
    }
    Ok(files)
}

// ---------------------------------------------------------------- retired records

struct Record {
    ctx: String,
    name: String,
    kept: Json,
    manifest: Manifest,
    removed_on: String,
    commit: String,
}

fn note_text(note: &Json, field: &str) -> Option<String> {
    note.get(field)
        .and_then(Json::as_str)
        .map(str::trim)
        .filter(|text| !text.is_empty())
        .map(str::to_owned)
}

fn parse_record(name: &str, value: Json, violations: &mut Vec<String>) -> Option<Record> {
    let ctx = format!("{RETIRED_DIR}/{name}");
    let Json::Object(mut fields) = value else {
        violations.push(format!(
            "{ctx}: a retired record needs a `{NOTE}` note ({})",
            NOTE_FIELDS.join(", ")
        ));
        return None;
    };
    let Some(at) = fields.iter().position(|(k, _)| k == NOTE) else {
        violations.push(format!(
            "{ctx}: a retired record needs a `{NOTE}` note ({})",
            NOTE_FIELDS.join(", ")
        ));
        return None;
    };
    let note = fields.remove(at).1;
    let value = Json::Object(fields);
    let shape_ok = matches!(&note, Json::Object(f) if f.len() == NOTE_FIELDS.len()
        && NOTE_FIELDS.iter().all(|n| f.iter().any(|(k, _)| k == n)));
    if !shape_ok {
        violations.push(format!(
            "{ctx}: the `{NOTE}` note must hold exactly {}",
            NOTE_FIELDS.join(", ")
        ));
        return None;
    }
    let (Some(removed_on), Some(commit), Some(reason)) = (
        note_text(&note, "removed_on"),
        note_text(&note, "removed_by_commit"),
        note_text(&note, "reason"),
    ) else {
        violations.push(format!(
            "{ctx}: every field of the `{NOTE}` note must be a non-empty string"
        ));
        return None;
    };
    if reason.chars().count() > MAX_REASON_CHARS || reason.chars().any(char::is_control) {
        violations.push(format!(
            "{ctx}: the reason must be one plain sentence of at most {MAX_REASON_CHARS} characters"
        ));
        return None;
    }
    let live_ctx = format!("{MANIFEST_DIR}/{name}: ");
    match parse_manifest(name, &value) {
        Ok(manifest) => Some(Record {
            ctx,
            name: name.to_owned(),
            kept: value,
            manifest,
            removed_on,
            commit,
        }),
        Err(errors) => {
            violations.extend(errors.into_iter().map(|error| {
                let detail = error.strip_prefix(&live_ctx).unwrap_or(&error).to_owned();
                format!("{ctx}: {detail}")
            }));
            None
        }
    }
}

/// What Git says about one removal commit, or why it cannot be one.
struct Removal {
    parent: String,
    date: String,
}

fn removals(
    git: &Git,
    commits: &BTreeSet<String>,
) -> Result<BTreeMap<String, std::result::Result<Removal, String>>> {
    let mut out = BTreeMap::new();
    let ids: Vec<String> = commits.iter().cloned().collect();
    let known = git.commits(&ids)?;
    let mut usable = Vec::new();
    for (commit, known) in ids.iter().zip(known) {
        if !is_hex_oid(commit) || !known {
            out.insert(
                commit.clone(),
                Err("is not a full commit id of this repository".into()),
            );
        } else if !git.in_head_history(commit)? {
            out.insert(
                commit.clone(),
                Err("is not in this checkout's history".into()),
            );
        } else {
            usable.push(commit.clone());
        }
    }
    if usable.is_empty() {
        return Ok(out);
    }
    let mut args = vec!["log", "--no-walk=unsorted", "--format=%H %P%x09%cs"];
    args.extend(usable.iter().map(String::as_str));
    let text = git.text(&args)?;
    for line in text.lines() {
        let Some((ids, date)) = line.split_once('\t') else {
            continue;
        };
        let ids: Vec<&str> = ids.split_whitespace().collect();
        let Some(commit) = ids.first() else { continue };
        let entry = match ids.as_slice() {
            [_, parent] => Ok(Removal {
                parent: (*parent).to_owned(),
                date: date.trim().to_owned(),
            }),
            _ => Err("must be an ordinary commit with exactly one parent".to_owned()),
        };
        out.insert((*commit).to_owned(), entry);
    }
    Ok(out)
}

fn check_nothing_erased(
    git: &Git,
    live: &[Manifest],
    on_disk: &BTreeSet<String>,
    violations: &mut Vec<String>,
) -> Result<()> {
    let raw = git.checked(
        &[
            "log",
            "--no-renames",
            "--diff-filter=D",
            "--name-only",
            "--format=",
            "HEAD",
            "--",
            MANIFEST_DIR,
        ],
        None,
    )?;
    let retired_prefix = format!("{RETIRED_DIR}/");
    let live_prefix = format!("{MANIFEST_DIR}/");
    let text = String::from_utf8(raw)
        .map_err(|_| "git returned a non-UTF-8 path under the harvest ledger".to_owned())?;
    let deleted: BTreeSet<&str> = text
        .lines()
        .map(str::trim)
        .filter(|path| path.ends_with(".json"))
        .collect();
    for path in deleted {
        if let Some(name) = path.strip_prefix(&retired_prefix) {
            if !name.contains('/') && !on_disk.contains(name) {
                violations.push(format!(
                    "{path} was deleted; a retired harvest record is never deleted"
                ));
            }
        } else if let Some(name) = path.strip_prefix(&live_prefix) {
            let id = name.trim_end_matches(".json");
            if !name.contains('/')
                && !on_disk.contains(name)
                && !live.iter().any(|manifest| manifest.id == id)
            {
                violations.push(format!(
                    "{path} was removed from the ledger without being retired; keep it as {RETIRED_DIR}/{name}"
                ));
            }
        }
    }
    Ok(())
}

/// Audits every retired record and returns the valid ones, which history may cite.
fn audit_retired(
    git: &Git,
    live: &[Manifest],
    head_exists: bool,
    violations: &mut Vec<String>,
) -> Result<Vec<Manifest>> {
    let files = json_files(&git.root, RETIRED_DIR)?;
    let on_disk: BTreeSet<String> = files.iter().map(|(name, _)| name.clone()).collect();
    if head_exists {
        check_nothing_erased(git, live, &on_disk, violations)?;
    }
    let mut records = Vec::new();
    for (name, parsed) in files {
        match parsed {
            Ok(value) => records.extend(parse_record(&name, value, violations)),
            Err(error) => violations.push(error),
        }
    }
    if records.is_empty() {
        return Ok(Vec::new());
    }
    let commits: BTreeSet<String> = records.iter().map(|r| r.commit.clone()).collect();
    let removals = removals(git, &commits)?;
    // In one lookup: each file in HEAD, before and after its removal, and each manifest before.
    let mut queries = Vec::new();
    for r in &records {
        for f in &r.manifest.files {
            queries.push(format!("HEAD:{}", f.dest_path));
        }
        if let Some(Ok(removal)) = removals.get(&r.commit) {
            for f in &r.manifest.files {
                queries.push(format!("{}:{}", removal.parent, f.dest_path));
                queries.push(format!("{}:{}", r.commit, f.dest_path));
            }
            queries.push(format!("{}:{MANIFEST_DIR}/{}", removal.parent, r.name));
        }
    }
    let mut answers = git.lookup(&queries)?.into_iter();
    let notices = normalized_disk_text(&git.root.join(NOTICES_FILE)).unwrap_or_default();
    let noticed: BTreeSet<&str> = notices.lines().map(str::trim).collect();
    let mut to_compare: Vec<(usize, String)> = Vec::new();
    let mut sound = vec![true; records.len()];
    for (index, record) in records.iter().enumerate() {
        let found = violations.len();
        let Record {
            ctx,
            manifest,
            commit,
            name,
            ..
        } = record;
        if live.iter().any(|other| other.id == manifest.id) {
            violations.push(format!(
                "{ctx}: {} is also a live manifest; a harvest is live or retired, never both",
                manifest.id
            ));
        }
        for file in &manifest.files {
            let in_head = answers
                .next()
                .flatten()
                .is_some_and(|(_, kind)| kind == "blob");
            if in_head || fs::symlink_metadata(git.root.join(&file.dest_path)).is_ok() {
                violations.push(format!(
                    "{ctx}: {} is still in the tree; a harvest cannot be retired while its code is here",
                    file.dest_path
                ));
            }
        }
        if noticed.contains(manifest.id.as_str()) {
            violations.push(format!(
                "{ctx}: {NOTICES_FILE} still carries the notice block of {}",
                manifest.id
            ));
        }
        match removals.get(commit) {
            Some(Ok(removal)) => {
                let mut before_any = false;
                let mut still: Option<&str> = None;
                for file in &manifest.files {
                    let before = answers.next().flatten().is_some_and(|(_, kind)| kind == "blob");
                    let after = answers.next().flatten().is_some_and(|(_, kind)| kind == "blob");
                    before_any |= before;
                    if after && still.is_none() {
                        still = Some(&file.dest_path);
                    }
                }
                let manifest_before = answers.next().flatten();
                if let Some(path) = still {
                    violations.push(format!(
                        "{ctx}: {commit} did not remove the last file: {path} is still in its tree"
                    ));
                }
                if !before_any {
                    violations.push(format!(
                        "{ctx}: none of its files was in the tree before {commit}, so that commit removed nothing of it"
                    ));
                }
                if removal.date != record.removed_on {
                    violations.push(format!(
                        "{ctx}: removed_on {} is not the date of {commit} ({})",
                        record.removed_on, removal.date
                    ));
                }
                match manifest_before {
                    Some((oid, kind)) if kind == "blob" => to_compare.push((index, oid)),
                    _ => violations.push(format!(
                        "{ctx}: {MANIFEST_DIR}/{name} did not exist before {commit}, so the record cannot be checked against the manifest it retires"
                    )),
                }
            }
            Some(Err(why)) => violations.push(format!("{ctx}: removed_by_commit {commit} {why}")),
            None => violations.push(format!(
                "{ctx}: removed_by_commit {commit} must be an ordinary commit with exactly one parent"
            )),
        }
        sound[index] = violations.len() == found;
    }
    let oids: BTreeSet<String> = to_compare.iter().map(|(_, oid)| oid.clone()).collect();
    let blobs = git.read_blobs(&oids)?;
    for (index, oid) in to_compare {
        let Some(record) = records.get(index) else {
            continue;
        };
        let same = blobs
            .get(&oid)
            .and_then(|bytes| json::parse(&lossy(bytes)).ok())
            .is_some_and(|before| same_json(&before, &record.kept));
        if !same {
            violations.push(format!(
                "{}: differs from {MANIFEST_DIR}/{} as it stood before {}; a retired record keeps its manifest unchanged",
                record.ctx, record.name, record.commit
            ));
            sound[index] = false;
        }
    }
    Ok(records
        .into_iter()
        .zip(sound)
        .filter_map(|(record, sound)| sound.then_some(record.manifest))
        .collect())
}

// ---------------------------------------------------------------- corrections and citations

fn exact_object<'a>(value: &'a Json, fields: &[&str]) -> Result<&'a [(String, Json)]> {
    let Json::Object(object) = value else {
        return Err("correction must be an object".into());
    };
    if object.len() != fields.len() || fields.iter().any(|f| !object.iter().any(|(k, _)| k == f)) {
        return Err("correction has missing or unbound fields".into());
    }
    Ok(object)
}

fn field<'a>(value: &'a Json, name: &str) -> Result<&'a str> {
    value
        .get(name)
        .and_then(Json::as_str)
        .filter(|text| !text.is_empty())
        .ok_or_else(|| format!("correction requires {name}"))
}

fn generated_metadata(path: &str) -> bool {
    path == NOTICES_FILE
        || path == APACHE_TEXT_PATH
        || [".agents/harvest/", ".agents/tasks/"].iter().any(|prefix| {
            path.strip_prefix(prefix)
                .is_some_and(|name| !name.contains('/') && name.ends_with(".json"))
        })
}

enum Origin<'a> {
    Manifest(&'a Manifest),
    Repository(&'a str),
}

fn verify_files(git: &Git, commit: &str, row: &Json, origin: &Origin<'_>) -> Result<()> {
    let parents = git.text(&["rev-list", "--parents", "-n", "1", commit])?;
    if parents.split_whitespace().count() != 2 {
        return Err("correction requires a commit with exactly one parent".into());
    }
    let raw = git.checked(
        &[
            "diff-tree",
            "--no-commit-id",
            "--name-only",
            "--diff-filter=A",
            "-r",
            "-z",
            commit,
            "--",
        ],
        None,
    )?;
    let mut added = BTreeSet::new();
    for path in raw.split(|b| *b == 0).filter(|p| !p.is_empty()) {
        let path = std::str::from_utf8(path)
            .map_err(|_| "correction source path is not UTF-8".to_owned())?;
        if !generated_metadata(path) {
            added.insert(path.to_owned());
        }
    }
    if added.is_empty() {
        return Err("correction has no imported source files".into());
    }
    let files = match row.get("files") {
        Some(Json::Array(files)) if !files.is_empty() && files.len() <= 4096 => files,
        _ => return Err("correction requires a bounded nonempty source-file list".into()),
    };
    let tree = git.list_tree(commit)?;
    let source_tree = match origin {
        Origin::Repository(source) => Some(git.list_tree(source)?),
        Origin::Manifest(_) => None,
    };
    let regular = |entry: &TreeEntry, blob: &str| {
        entry.kind == "blob"
            && matches!(entry.mode.as_str(), "100644" | "100755")
            && entry.oid == blob
    };
    let mut covered = BTreeSet::new();
    for file in files {
        match origin {
            Origin::Manifest(_) => exact_object(file, &["path", "commit_blob_sha1"])?,
            Origin::Repository(_) => exact_object(
                file,
                &[
                    "path",
                    "commit_blob_sha1",
                    "source_path",
                    "source_blob_sha1",
                ],
            )?,
        };
        let path = field(file, "path")?;
        if normalize_rel(path, true).as_deref() != Ok(path) || !covered.insert(path.to_owned()) {
            return Err("correction source path is invalid or duplicated".into());
        }
        let blob = field(file, "commit_blob_sha1")?;
        if !is_hex_oid(blob) || !tree.get(path).is_some_and(|e| regular(e, blob)) {
            return Err("correction source blob differs from the immutable commit".into());
        }
        match (origin, &source_tree) {
            (Origin::Manifest(manifest), _) => {
                if !manifest.files.iter().any(|source| source.dest_path == path) {
                    return Err(format!(
                        "replacement manifest does not cover imported source {path}"
                    ));
                }
            }
            (Origin::Repository(_), Some(source_tree)) => {
                let source = field(file, "source_path")?;
                let source_blob = field(file, "source_blob_sha1")?;
                if normalize_rel(source, true).as_deref() != Ok(source)
                    || !is_hex_oid(source_blob)
                    || !source_tree
                        .get(source)
                        .is_some_and(|e| regular(e, source_blob))
                {
                    return Err(format!(
                        "correction repository source for {path} differs from the earlier commit"
                    ));
                }
            }
            (Origin::Repository(_), None) => {
                return Err("correction repository source tree is missing".into());
            }
        }
    }
    if covered != added {
        return Err("correction must cover exactly all imported source paths".into());
    }
    Ok(())
}

fn load_corrections(
    git: &Git,
    live: &[Manifest],
    retired: &[Manifest],
) -> Result<BTreeMap<(String, String), String>> {
    let path = git.root.join(CORRECTIONS_PATH);
    if !path.exists() {
        return Ok(BTreeMap::new());
    }
    let metadata = fs::symlink_metadata(&path).map_err(|e| e.to_string())?;
    if !metadata.is_file()
        || metadata.file_type().is_symlink()
        || metadata.len() > 1024 * 1024
        || !path
            .canonicalize()
            .map_err(|e| e.to_string())?
            .starts_with(git.root.canonicalize().map_err(|e| e.to_string())?)
    {
        return Err("correction record is redirected, nonregular or oversized".into());
    }
    let value = json::parse(&lossy(&fs::read(&path).map_err(|e| e.to_string())?))
        .map_err(|e| format!("invalid harvest correction record: {e}"))?;
    if duplicate_key(&value) {
        return Err("duplicate harvest correction JSON field".into());
    }
    exact_object(&value, &["schema", "corrections"])?;
    if value.get("schema").and_then(Json::as_str) != Some(CORRECTIONS_SCHEMA) {
        return Err("unsupported harvest correction schema".into());
    }
    let rows = match value.get("corrections") {
        Some(Json::Array(rows)) if !rows.is_empty() && rows.len() <= 128 => rows,
        _ => return Err("correction record requires a bounded nonempty list".into()),
    };
    let mut corrections = BTreeMap::new();
    for row in rows {
        let in_repository = row.get("source_commit").is_some();
        let origin_field = if in_repository {
            "source_commit"
        } else {
            "manifest_id"
        };
        exact_object(
            row,
            &["commit", "old_label", origin_field, "files", "reason"],
        )?;
        let commit = field(row, "commit")?;
        let old = field(row, "old_label")?;
        let replacement = field(row, origin_field)?;
        let reason = field(row, "reason")?;
        if !is_hex_oid(commit)
            || old
                .chars()
                .any(|c| c.is_whitespace() || c.is_control() || c == ',')
            || reason.trim().is_empty()
        {
            return Err("invalid correction commit, exact label or reason".into());
        }
        let origin = if in_repository {
            let earlier = is_hex_oid(replacement)
                && replacement != commit
                && git.commits(&[replacement.to_owned()])?.first() == Some(&true)
                && git
                    .output(&["merge-base", "--is-ancestor", replacement, commit], None)?
                    .status
                    .success();
            if !earlier {
                return Err(
                    "correction repository source must be a strictly earlier commit".into(),
                );
            }
            Origin::Repository(replacement)
        } else {
            Origin::Manifest(
                live.iter()
                    .chain(retired)
                    .find(|manifest| manifest.id == replacement)
                    .ok_or("correction replacement manifest does not exist or is invalid")?,
            )
        };
        if live
            .iter()
            .chain(retired)
            .any(|manifest| manifest.id == old)
        {
            return Err("correction cannot replace an existing valid manifest label".into());
        }
        if git.commits(&[commit.to_owned()])?.first() != Some(&true)
            || !git.in_head_history(commit)?
        {
            return Err("correction commit is not in this checkout's history".into());
        }
        let trailers = git.harvest_trailers(&format!("{commit}^!"))?;
        if trailers.len() != 1
            || trailers[0].0 != commit
            || trailers[0]
                .1
                .iter()
                .filter(|label| label.as_str() == old)
                .count()
                != 1
        {
            return Err(
                "correction does not match the immutable commit's exact Harvest label".into(),
            );
        }
        verify_files(git, commit, row, &origin)?;
        if corrections
            .insert((commit.to_owned(), old.to_owned()), replacement.to_owned())
            .is_some()
        {
            return Err("duplicate harvest correction".into());
        }
    }
    Ok(corrections)
}

#[derive(Default)]
struct Citations {
    commits: usize,
    live: usize,
    retired: usize,
    corrected: usize,
    refused: usize,
}

fn audit_history(
    git: &Git,
    live: &[Manifest],
    retired: &[Manifest],
    range: Option<&str>,
    violations: &mut Vec<String>,
) -> Result<Citations> {
    let corrections = load_corrections(git, live, retired)?;
    let mut citations = Citations::default();
    let Some(range) = range else {
        return Ok(citations);
    };
    let known: BTreeSet<String> = json_files(&git.root, MANIFEST_DIR)?
        .into_iter()
        .map(|(name, _)| name.trim_end_matches(".json").to_owned())
        .collect();
    for (commit, ids) in git.harvest_trailers(range)? {
        citations.commits += 1;
        for id in &ids {
            if known.contains(id) {
                citations.live += 1;
            } else if retired.iter().any(|record| &record.id == id) {
                citations.retired += 1;
            } else if corrections.contains_key(&(commit.clone(), id.clone())) {
                citations.corrected += 1;
            } else {
                citations.refused += 1;
                violations.push(format!(
                    "commit {} has `Harvest: {id}` but {MANIFEST_DIR}/{id}.json does not exist and no valid {RETIRED_DIR}/{id}.json stands for it",
                    &commit[..commit.len().min(12)]
                ));
            }
        }
    }
    Ok(citations)
}

// ---------------------------------------------------------------- the audit

/// Audit the harvest ledger of the repository holding `cwd`; `range` limits the citations read
/// from history (`HEAD` for all of it), and without it no citation is read.
pub fn audit(cwd: &Path, range: Option<&str>) -> Result<Report> {
    let git = Git::open(cwd)?;
    let root = git.root.clone();
    let (manifests, mut violations) = load_manifests(&root);
    let files = audit_manifests(&git, &manifests, &mut violations)?;
    if violations.is_empty() {
        let expected = render_notices(&manifests);
        match normalized_disk_text(&root.join(NOTICES_FILE)) {
            Some(text) if text == expected => {}
            None if manifests.is_empty() => {}
            Some(_) => violations.push(format!(
                "{NOTICES_FILE} is stale; run `local-harvest notices` (Apache-2.0 4(d))"
            )),
            None => violations.push(format!(
                "{NOTICES_FILE} is missing; run `local-harvest notices` (Apache-2.0 4(d))"
            )),
        }
    }
    if manifests.iter().any(|m| m.license.contains("Apache-2.0")) {
        let text = normalized_disk_text(&root.join(APACHE_TEXT_PATH)).unwrap_or_default();
        if classify_license_text(&text) != Some("Apache-2.0") {
            violations.push(format!(
                "{APACHE_TEXT_PATH} is missing or not the Apache License 2.0 text (Apache-2.0 4(a))"
            ));
        }
    }
    let head_exists = git.commits(&["HEAD".to_owned()])?.first() == Some(&true);
    let retired = audit_retired(&git, &manifests, head_exists, &mut violations)?;
    let cited = audit_history(&git, &manifests, &retired, range, &mut violations)?;
    let citations = cited.live + cited.retired + cited.corrected + cited.refused;
    let tally = format!(
        "{} retired record(s); {citations} citation(s): {} live, {} retired, {} corrected, {} refused",
        retired.len(),
        cited.live,
        cited.retired,
        cited.corrected,
        cited.refused
    );
    let ok = violations.is_empty();
    let text = if ok {
        format!(
            "harvest audit passed: {} manifest(s), {files} file(s), {} Harvest-trailer commit(s); {tally}",
            manifests.len(),
            cited.commits
        )
    } else {
        format!(
            "harvest audit failed with {} violation(s):\n  {}\n({tally})",
            violations.len(),
            violations.join("\n  ")
        )
    };
    Ok(Report { ok, text })
}

/// `audit [--range <A..B>]`, as the earlier `local-harvest audit` took it.
pub fn run_audit(cwd: &Path, args: &[String]) -> Result<Report> {
    let mut range = None;
    let mut rest = args.iter();
    while let Some(arg) = rest.next() {
        match arg.as_str() {
            "--range" => {
                range = Some(
                    rest.next()
                        .cloned()
                        .ok_or_else(|| "--range needs a value".to_owned())?,
                )
            }
            other => return Err(format!("unknown option {other} for audit")),
        }
    }
    audit(cwd, range.as_deref())
}

// SPDX-License-Identifier: Apache-2.0
//! `susi-tools speed [--repo <path>] [--cold] [--json]`: time a Cargo checkout's everyday
//! operations against the 10-second warm line, portably across Linux, macOS and Windows.
//!
//! Warm rows measure with caches filled:
//! - `cargo build --workspace --offline --locked`, nothing changed and after touching one file;
//! - `cargo nextest run --workspace --offline --locked` (and `--no-default-features` when the
//!   workspace defines features);
//! - `cargo clippy --workspace --all-targets --offline --locked -- -D warnings`;
//! - `cargo fmt --all --check`;
//! - `git status`, `git fetch`, and `susi-tools workflow check`.
//!
//! `--cold` also times `cargo build --workspace --offline --locked` after `cargo clean` in a
//! throwaway target directory (never the agent's own), reported apart from the warm rows.
//! The touched source file is restored byte for byte, also when any step fails.

use crate::git::Repo;
use crate::json::{self, Json};
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant, SystemTime};

/// Every warm operation an agent waits on must finish within this many seconds.
pub const LINE_SECONDS: f64 = 10.0;

const SCHEMA: &str = "susi-speed-report-v1";
const PROBE_SUFFIX: &[u8] = b"\n// susi-tools-speed-probe\n";

static TEMP_COUNTER: AtomicU64 = AtomicU64::new(0);

pub struct Config<'a> {
    pub cold: bool,
    pub agent: Option<&'a str>,
}

pub struct Row {
    pub operation: String,
    pub command: Vec<String>,
    pub seconds: f64,
    pub over_line: bool,
    pub ok: bool,
    pub exit_code: i32,
    pub detail: Option<String>,
}

pub struct Report {
    pub repo: String,
    pub commit: String,
    pub timestamp_utc: String,
    pub os: &'static str,
    pub arch: &'static str,
    pub cores: usize,
    pub line_seconds: f64,
    pub touched_file: Option<String>,
    pub has_features: bool,
    pub warm: Vec<Row>,
    pub cold: Vec<Row>,
}

impl Report {
    pub fn warm_ok_under_line(&self) -> usize {
        self.warm
            .iter()
            .filter(|row| row.ok && !row.over_line)
            .count()
    }

    pub fn warm_over_line(&self) -> usize {
        self.warm.iter().filter(|row| row.over_line).count()
    }

    pub fn warm_failed(&self) -> usize {
        self.warm.iter().filter(|row| !row.ok).count()
    }

    /// True when every warm operation passed within the 10-second line and any cold build passed.
    pub fn passed(&self) -> bool {
        self.warm.iter().all(|row| row.ok && !row.over_line) && self.cold.iter().all(|row| row.ok)
    }

    pub fn render_table(&self) -> String {
        let mut out = String::new();
        out.push_str(&format!(
            "speed report for {} at {} ({}, host: {}/{}, {} cores, line: {:.1}s)\n\n",
            self.repo,
            self.commit,
            self.timestamp_utc,
            self.os,
            self.arch,
            self.cores,
            self.line_seconds
        ));
        out.push_str(
            "The suites and the lints are timed side by side, as `susi-tools gates` runs them: each time is an upper bound.\n\n",
        );
        write_table(&mut out, &self.warm);
        let ok_under = self.warm_ok_under_line();
        let total = self.warm.len();
        let over = self.warm_over_line();
        let failed = self.warm_failed();
        out.push('\n');
        if over == 0 && failed == 0 {
            out.push_str(&format!(
                "{ok_under} of {total} warm operations under the {:.1}s line\n",
                self.line_seconds
            ));
        } else {
            out.push_str(&format!(
                "{ok_under} of {total} warm operations under the {:.1}s line ({over} over, {failed} failed)\n",
                self.line_seconds
            ));
        }
        if !self.cold.is_empty() {
            out.push_str("\nCold (reported apart; throwaway target directory):\n");
            write_table(&mut out, &self.cold);
        }
        for row in self.warm.iter().chain(&self.cold) {
            if !row.ok {
                if let Some(detail) = &row.detail {
                    out.push_str(&format!(
                        "\nfailed: {} (exit {}):\n{detail}\n",
                        row.operation, row.exit_code
                    ));
                }
            }
        }
        out
    }

    pub fn render_json(&self) -> String {
        let row_json = |row: &Row| {
            let mut fields = vec![
                ("operation".into(), Json::Str(row.operation.clone())),
                (
                    "command".into(),
                    Json::Array(row.command.iter().cloned().map(Json::Str).collect()),
                ),
                (
                    "seconds".into(),
                    Json::Number(format!("{:.2}", row.seconds)),
                ),
                ("over_line".into(), Json::Bool(row.over_line)),
                (
                    "line".into(),
                    Json::Str(if row.over_line { "over" } else { "under" }.into()),
                ),
                ("ok".into(), Json::Bool(row.ok)),
                ("exit_code".into(), Json::Number(row.exit_code.to_string())),
            ];
            if let Some(detail) = &row.detail {
                fields.push(("stderr".into(), Json::Str(detail.clone())));
            }
            Json::Object(fields)
        };
        let doc = Json::Object(vec![
            ("schema".into(), Json::Str(SCHEMA.into())),
            ("repo".into(), Json::Str(self.repo.clone())),
            ("commit".into(), Json::Str(self.commit.clone())),
            (
                "timestamp_utc".into(),
                Json::Str(self.timestamp_utc.clone()),
            ),
            (
                "host".into(),
                Json::Object(vec![
                    ("os".into(), Json::Str(self.os.into())),
                    ("arch".into(), Json::Str(self.arch.into())),
                    ("cores".into(), Json::Number(self.cores.to_string())),
                ]),
            ),
            (
                "line_seconds".into(),
                Json::Number(format!("{:.1}", self.line_seconds)),
            ),
            (
                "touched_file".into(),
                match &self.touched_file {
                    Some(path) => Json::Str(path.clone()),
                    None => Json::Null,
                },
            ),
            ("has_features".into(), Json::Bool(self.has_features)),
            (
                "warm".into(),
                Json::Array(self.warm.iter().map(row_json).collect()),
            ),
            (
                "cold".into(),
                Json::Array(self.cold.iter().map(row_json).collect()),
            ),
            (
                "warm_ok_under_line".into(),
                Json::Number(self.warm_ok_under_line().to_string()),
            ),
            (
                "warm_total".into(),
                Json::Number(self.warm.len().to_string()),
            ),
            (
                "warm_over_line".into(),
                Json::Number(self.warm_over_line().to_string()),
            ),
            (
                "warm_failed".into(),
                Json::Number(self.warm_failed().to_string()),
            ),
        ]);
        let mut text = doc.to_pretty();
        text.push('\n');
        text
    }
}

fn write_table(out: &mut String, rows: &[Row]) {
    let op_width = rows
        .iter()
        .map(|r| r.operation.len())
        .max()
        .unwrap_or(9)
        .max("Operation".len());
    out.push_str(&format!(
        "{:<op_width$}  {:>7}  {:<5}  {}\n",
        "Operation", "Seconds", "Line", "Status"
    ));
    out.push_str(&"-".repeat(op_width + 26));
    out.push('\n');
    for row in rows {
        let line = if row.over_line { "over" } else { "under" };
        let status = if row.ok {
            "ok".to_owned()
        } else {
            format!("failed (exit {})", row.exit_code)
        };
        out.push_str(&format!(
            "{:<op_width$}  {:>7.2}  {:<5}  {status}\n",
            row.operation, row.seconds, line
        ));
    }
}

/// Time a Cargo checkout's everyday operations and return the measured report.
pub fn run(repo: &Repo, cfg: &Config<'_>) -> Result<Report, String> {
    let work = repo.work_tree()?.to_path_buf();
    if !work.join("Cargo.toml").is_file() {
        return Err(format!("not a Cargo checkout: {}", work.display()));
    }
    // Capture our own executable path before any `cargo build` in this run can replace it
    // (on Linux, `/proc/self/exe` gains a ` (deleted)` suffix once overwritten).
    let self_exe = std::env::current_exe().ok();
    let commit = repo
        .try_read(&["rev-parse", "--short=12", "HEAD"])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unborn".to_owned());
    let timestamp_utc = crate::time::utc_iso(crate::time::now_unix());
    let cores = std::thread::available_parallelism().map_or(1, usize::from);

    let info = inspect_workspace(&work);
    let touched_rel = info
        .touch_file
        .as_deref()
        .map(|path| relative_slash(&work, path));

    let mut warm = Vec::new();

    // 1. Warmup build, then time `cargo build` with nothing changed.
    let build_args = ["build", "--workspace", "--offline", "--locked"];
    let _ = run_cargo(&work, &build_args, None);
    warm.push(measure_cargo(
        "cargo build --workspace --offline --locked (nothing changed)",
        &work,
        &build_args,
        None,
    ));

    // 2. Touch one source file, time `cargo build`, and always restore byte for byte; then the
    // suites and the lints run side by side, as `susi-tools gates` runs them (the CEO, 2026-11-24:
    // the 10-second line is non-negotiable; one after another they took 17.7 s). Each time is so
    // measured under the others' load: an upper bound.
    if let (Some(touch_path), Some(rel)) = (&info.touch_file, &touched_rel) {
        let row = {
            let guard = TouchGuard::touch(touch_path)?;
            let measured = measure_cargo(
                &format!("cargo build --workspace --offline --locked (touched {rel})"),
                &work,
                &build_args,
                None,
            );
            guard.restore()?;
            measured
        };
        warm.push(row);
    }
    let base_target = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("target"));
    let customer_target = base_target.join("no-default-features");
    let featured = info.has_features.then(|| info.featured.clone());
    let (suite, customer, rest) = std::thread::scope(|scope| {
        let suite = scope.spawn(|| {
            let warmup = [
                "nextest",
                "run",
                "--workspace",
                "--offline",
                "--locked",
                "--no-run",
            ];
            let _ = run_cargo(&work, &warmup, None);
            // As the gates run it: with the repository's nextest profile `gates`, if it has one.
            let mut args = vec!["nextest", "run", "--workspace", "--offline", "--locked"];
            if crate::gates::nextest_profile(&work, crate::gates::GATES_PROFILE) {
                args.extend(["--profile", crate::gates::GATES_PROFILE]);
            }
            vec![measure_cargo(
                &format!("cargo {}", args.join(" ")),
                &work,
                &args,
                None,
            )]
        });
        let customer = scope.spawn(|| {
            let Some(packages) = &featured else {
                return Vec::new();
            };
            // Only the packages with features change; built in their own target folder.
            let mut args: Vec<String> = [
                "nextest",
                "run",
                "--offline",
                "--locked",
                "--no-default-features",
            ]
            .iter()
            .map(|a| (*a).to_owned())
            .collect();
            if packages.is_empty() {
                args.push("--workspace".to_owned());
            }
            for package in packages {
                args.extend(["-p".to_owned(), package.clone()]);
            }
            let profile = crate::gates::GATES_NO_DEFAULT_PROFILE;
            if crate::gates::nextest_profile(&work, profile) {
                args.extend(["--profile".to_owned(), profile.to_owned()]);
            }
            let refs: Vec<&str> = args.iter().map(String::as_str).collect();
            let mut warmup = refs.clone();
            warmup.push("--no-run");
            let _ = run_cargo(&work, &warmup, Some(&customer_target));
            vec![measure_cargo(
                &format!("cargo {}", refs.join(" ")),
                &work,
                &refs,
                Some(&customer_target),
            )]
        });
        let rest = scope.spawn(|| {
            let clippy_args = [
                "clippy",
                "--workspace",
                "--all-targets",
                "--offline",
                "--locked",
                "--",
                "-D",
                "warnings",
            ];
            vec![
                measure_cargo(
                    "cargo clippy --workspace --all-targets --offline --locked -- -D warnings",
                    &work,
                    &clippy_args,
                    None,
                ),
                measure_cargo(
                    "cargo fmt --all --check",
                    &work,
                    &["fmt", "--all", "--check"],
                    None,
                ),
                measure_git("git status", &work, &["status"]),
                measure_git("git fetch", &work, &["fetch"]),
                measure_workflow_check(&work, self_exe.as_deref(), cfg.agent),
            ]
        });
        (
            suite.join().unwrap_or_default(),
            customer.join().unwrap_or_default(),
            rest.join().unwrap_or_default(),
        )
    });
    warm.extend(suite);
    warm.extend(customer);
    warm.extend(rest);

    // 8. Optional cold build into a throwaway target directory (never the agent's own).
    let mut cold = Vec::new();
    if cfg.cold {
        let temp = TempTargetDir::create()?;
        let _ = run_cargo(&work, &["clean"], Some(temp.path()));
        cold.push(measure_cargo(
            "cargo build --workspace --offline --locked (cold, after cargo clean)",
            &work,
            &build_args,
            Some(temp.path()),
        ));
    }

    Ok(Report {
        repo: work.display().to_string(),
        commit,
        timestamp_utc,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        cores,
        line_seconds: LINE_SECONDS,
        touched_file: touched_rel,
        has_features: info.has_features,
        warm,
        cold,
    })
}

struct WorkspaceInfo {
    has_features: bool,
    /// The packages that declare features, by name; empty when only a manifest scan saw them.
    featured: Vec<String>,
    touch_file: Option<PathBuf>,
}

/// The packages whose build changes with `--no-default-features`: `None` when none declares a
/// feature, `Some` of their names otherwise (empty when they could not be named, and the whole
/// workspace stands in for them).
pub fn featured_packages(work: &Path) -> Option<Vec<String>> {
    let info = inspect_workspace(work);
    info.has_features.then_some(info.featured)
}

/// Inspect the workspace with `cargo metadata --no-deps`, falling back to simple file scanning
/// when metadata cannot run (for example, a broken manifest the timed build will then report).
fn inspect_workspace(work: &Path) -> WorkspaceInfo {
    if let Some(info) = inspect_from_metadata(work) {
        return info;
    }
    WorkspaceInfo {
        has_features: manifest_mentions_features(&work.join("Cargo.toml")),
        featured: Vec::new(),
        touch_file: fallback_touch_file(work),
    }
}

fn inspect_from_metadata(work: &Path) -> Option<WorkspaceInfo> {
    let out = Command::new("cargo")
        .current_dir(work)
        .args([
            "metadata",
            "--no-deps",
            "--format-version",
            "1",
            "--offline",
            "--locked",
        ])
        .output()
        .ok()?;
    if !out.status.success() {
        return None;
    }
    let text = String::from_utf8(out.stdout).ok()?;
    let meta = json::parse(&text).ok()?;
    let packages = match meta.get("packages") {
        Some(Json::Array(items)) => items,
        _ => return None,
    };
    let featured: Vec<String> = packages
        .iter()
        .filter(|pkg| {
            matches!(
                pkg.get("features"),
                Some(Json::Object(entries)) if !entries.is_empty()
            )
        })
        .map(|pkg| pkg.text("name"))
        .collect();
    let has_features = !featured.is_empty();
    let root_manifest = canonical(&work.join("Cargo.toml"));
    let root_work = canonical(work);
    let mut best: Option<(u8, PathBuf)> = None;
    for pkg in packages {
        let is_root = pkg
            .get("manifest_path")
            .and_then(Json::as_str)
            .is_some_and(|p| canonical(Path::new(p)) == root_manifest);
        let pkg_name = pkg.text("name");
        let Some(Json::Array(targets)) = pkg.get("targets") else {
            continue;
        };
        for target in targets {
            let Some(src) = target.get("src_path").and_then(Json::as_str) else {
                continue;
            };
            let path = PathBuf::from(src);
            if path.extension().and_then(|e| e.to_str()) != Some("rs") || !path.is_file() {
                continue;
            }
            if !canonical(&path).starts_with(&root_work) {
                continue;
            }
            let kinds = target.str_list("kind");
            let is_bin = kinds.iter().any(|k| k == "bin");
            let is_lib = kinds.iter().any(|k| k == "lib");
            if !is_bin && !is_lib {
                continue;
            }
            let same_name = target.text("name") == pkg_name;
            let rank = match (is_root, is_bin && same_name, is_lib, is_bin) {
                (true, true, _, _) => 0,
                (true, _, true, _) => 1,
                (true, _, _, true) => 2,
                (false, true, _, _) => 3,
                (false, _, true, _) => 4,
                _ => 5,
            };
            if best.as_ref().is_none_or(|(best_rank, _)| rank < *best_rank) {
                best = Some((rank, path));
            }
        }
    }
    Some(WorkspaceInfo {
        has_features,
        featured,
        touch_file: best.map(|(_, p)| p).or_else(|| fallback_touch_file(work)),
    })
}

fn manifest_mentions_features(manifest: &Path) -> bool {
    let Ok(text) = std::fs::read_to_string(manifest) else {
        return false;
    };
    let mut in_features = false;
    for line in text.lines() {
        let clean = line.split('#').next().unwrap_or("").trim();
        if clean.is_empty() {
            continue;
        }
        if clean.starts_with('[') {
            in_features = clean == "[features]";
            continue;
        }
        if in_features && clean.contains('=') {
            return true;
        }
    }
    false
}

fn fallback_touch_file(work: &Path) -> Option<PathBuf> {
    for rel in ["src/main.rs", "src/lib.rs"] {
        let candidate = work.join(rel);
        if candidate.is_file() {
            return Some(candidate);
        }
    }
    for dir in ["src", "crates"] {
        let root = work.join(dir);
        if let Some(found) = first_rs_file(&root) {
            return Some(found);
        }
    }
    None
}

fn first_rs_file(dir: &Path) -> Option<PathBuf> {
    let entries = std::fs::read_dir(dir).ok()?;
    let mut paths: Vec<PathBuf> = entries.flatten().map(|e| e.path()).collect();
    paths.sort();
    for path in paths {
        if path.is_file() && path.extension().and_then(|e| e.to_str()) == Some("rs") {
            return Some(path);
        }
        if path.is_dir() {
            if let Some(found) = first_rs_file(&path) {
                return Some(found);
            }
        }
    }
    None
}

fn canonical(path: &Path) -> PathBuf {
    path.canonicalize().unwrap_or_else(|_| path.to_path_buf())
}

fn relative_slash(base: &Path, path: &Path) -> String {
    let rel = path.strip_prefix(base).ok().or_else(|| {
        let cb = canonical(base);
        let cp = canonical(path);
        cp.strip_prefix(&cb)
            .ok()
            .map(Path::to_path_buf)
            .map(|_| path)
    });
    let chosen = match rel {
        Some(r) if r != path => r.to_path_buf(),
        _ => canonical(path)
            .strip_prefix(canonical(base))
            .map(Path::to_path_buf)
            .unwrap_or_else(|_| path.to_path_buf()),
    };
    chosen.to_string_lossy().replace('\\', "/")
}

/// Temporarily appends a harmless Rust comment to a source file so Cargo is guaranteed to
/// recompile it, and restores the exact original bytes on `restore()` or `Drop`.
struct TouchGuard {
    path: PathBuf,
    original: Vec<u8>,
    restored: bool,
}

impl TouchGuard {
    fn touch(path: &Path) -> Result<TouchGuard, String> {
        let original = std::fs::read(path).map_err(|e| {
            format!(
                "cannot read {} to time a one-file rebuild: {e}",
                path.display()
            )
        })?;
        let before_mtime = std::fs::metadata(path).and_then(|m| m.modified()).ok();
        let mut modified = Vec::with_capacity(original.len() + PROBE_SUFFIX.len());
        modified.extend_from_slice(&original);
        modified.extend_from_slice(PROBE_SUFFIX);

        let guard = TouchGuard {
            path: path.to_path_buf(),
            original,
            restored: false,
        };
        std::fs::write(&guard.path, &modified).map_err(|e| {
            format!(
                "cannot touch {} to time a one-file rebuild: {e}",
                guard.path.display()
            )
        })?;
        // Ensure the modification timestamp advances past the previous build even on coarse
        // filesystem clocks, without writing a future timestamp.
        let after_mtime = std::fs::metadata(&guard.path)
            .and_then(|m| m.modified())
            .ok();
        if let (Some(before), Some(after)) = (before_mtime, after_mtime) {
            if after <= before {
                std::thread::sleep(Duration::from_millis(15));
                if let Ok(file) = std::fs::OpenOptions::new().write(true).open(&guard.path) {
                    let next = SystemTime::now().max(before + Duration::from_millis(10));
                    let _ = file.set_modified(next);
                }
            }
        }
        Ok(guard)
    }

    fn restore(mut self) -> Result<(), String> {
        let result = std::fs::write(&self.path, &self.original).map_err(|e| {
            format!(
                "cannot restore {} after timing one-file rebuild: {e}",
                self.path.display()
            )
        });
        self.restored = true;
        result
    }
}

impl Drop for TouchGuard {
    fn drop(&mut self) {
        if !self.restored {
            let _ = std::fs::write(&self.path, &self.original);
            self.restored = true;
        }
    }
}

/// A throwaway `CARGO_TARGET_DIR` for `--cold`, deleted on drop so the agent's own target
/// directory is never cleaned or touched.
struct TempTargetDir {
    path: PathBuf,
}

impl TempTargetDir {
    fn create() -> Result<TempTargetDir, String> {
        let seq = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "susi-tools-speed-cold-{}-{seq}",
            std::process::id()
        ));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).map_err(|e| {
            format!(
                "cannot create throwaway target directory {}: {e}",
                path.display()
            )
        })?;
        Ok(TempTargetDir { path })
    }

    fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for TempTargetDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

fn run_cargo(
    work: &Path,
    args: &[&str],
    target_dir: Option<&Path>,
) -> Result<(i32, String), String> {
    let mut cmd = Command::new("cargo");
    cmd.current_dir(work).args(args);
    if let Some(dir) = target_dir {
        cmd.env("CARGO_TARGET_DIR", dir)
            .env_remove("CARGO_BUILD_TARGET_DIR");
    }
    exec_capture(cmd, "cargo")
}

fn measure_cargo(operation: &str, work: &Path, args: &[&str], target_dir: Option<&Path>) -> Row {
    let mut command = vec!["cargo".to_owned()];
    command.extend(args.iter().map(|s| (*s).to_owned()));
    let started = Instant::now();
    let result = run_cargo(work, args, target_dir);
    finish_row(operation, command, started.elapsed(), result)
}

fn measure_git(operation: &str, work: &Path, args: &[&str]) -> Row {
    let mut command = vec!["git".to_owned()];
    command.extend(args.iter().map(|s| (*s).to_owned()));
    let mut cmd = Command::new(crate::git_program::path());
    for (name, _) in std::env::vars_os() {
        let upper = name.to_string_lossy().to_ascii_uppercase();
        if upper.starts_with("GIT_")
            && upper != "GIT_CONFIG_GLOBAL"
            && upper != "GIT_CONFIG_NOSYSTEM"
            && upper != "GIT_SSH_COMMAND"
        {
            cmd.env_remove(&name);
        }
    }
    cmd.current_dir(work)
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .args(args);
    let started = Instant::now();
    let result = exec_capture(cmd, "git");
    finish_row(operation, command, started.elapsed(), result)
}

fn measure_workflow_check(work: &Path, self_exe: Option<&Path>, agent: Option<&str>) -> Row {
    let program = self_exe
        .filter(|path| path.is_file())
        .map(Path::to_path_buf)
        .unwrap_or_else(|| PathBuf::from("susi-tools"));
    let mut cmd = Command::new(&program);
    cmd.current_dir(work)
        .args(["workflow", "check", "--repo"])
        .arg(work);
    let mut command = vec![
        "susi-tools".to_owned(),
        "workflow".to_owned(),
        "check".to_owned(),
    ];
    if let Some(agent) = agent {
        cmd.args(["--agent", agent]);
        command.push("--agent".to_owned());
        command.push(agent.to_owned());
    }
    let started = Instant::now();
    let result = exec_capture(cmd, "susi-tools");
    finish_row(
        "susi-tools workflow check",
        command,
        started.elapsed(),
        result,
    )
}

fn exec_capture(mut cmd: Command, name: &str) -> Result<(i32, String), String> {
    let out = cmd
        .output()
        .map_err(|e| format!("cannot run {name}: {e}"))?;
    let code = out.status.code().unwrap_or(-1);
    let stderr = String::from_utf8_lossy(&out.stderr).trim().to_owned();
    let detail = if stderr.is_empty() {
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    } else {
        stderr
    };
    Ok((code, detail))
}

fn finish_row(
    operation: &str,
    command: Vec<String>,
    elapsed: Duration,
    result: Result<(i32, String), String>,
) -> Row {
    let seconds = (elapsed.as_secs_f64() * 100.0).round() / 100.0;
    let over_line = seconds > LINE_SECONDS;
    match result {
        Ok((0, _)) => Row {
            operation: operation.to_owned(),
            command,
            seconds,
            over_line,
            ok: true,
            exit_code: 0,
            detail: None,
        },
        Ok((code, detail)) => Row {
            operation: operation.to_owned(),
            command,
            seconds,
            over_line,
            ok: false,
            exit_code: code,
            detail: (!detail.is_empty()).then(|| truncate_detail(&detail, 600)),
        },
        Err(err) => Row {
            operation: operation.to_owned(),
            command,
            seconds,
            over_line,
            ok: false,
            exit_code: -1,
            detail: Some(truncate_detail(&err, 600)),
        },
    }
}

fn truncate_detail(text: &str, max_chars: usize) -> String {
    let mut out: String = text.chars().take(max_chars).collect();
    if text.chars().count() > max_chars {
        out.push_str("...");
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn touch_guard_restores_exact_bytes_even_on_drop() {
        let dir =
            std::env::temp_dir().join(format!("susi-tools-touch-guard-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("sample.rs");
        let original = b"fn main() {\r\n    // exact bytes, no trailing newline\r\n}";
        std::fs::write(&file, original).unwrap();
        {
            let _guard = TouchGuard::touch(&file).unwrap();
            let touched = std::fs::read(&file).unwrap();
            assert_ne!(touched.as_slice(), original);
            assert!(touched.starts_with(original));
            // Drop without calling restore() simulates an early return or panic.
        }
        assert_eq!(std::fs::read(&file).unwrap().as_slice(), original);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn row_marks_under_and_over_the_ten_second_line() {
        let under = finish_row(
            "fast",
            vec!["cargo".into()],
            Duration::from_millis(10_000),
            Ok((0, String::new())),
        );
        assert!(!under.over_line);
        assert!(under.ok);
        let over = finish_row(
            "slow",
            vec!["cargo".into()],
            Duration::from_millis(10_050),
            Ok((0, String::new())),
        );
        assert!(over.over_line);
        assert!(over.ok);
    }

    #[test]
    fn report_renders_valid_json_and_table_with_cold_apart() {
        let report = Report {
            repo: "/tmp/example".into(),
            commit: "0123456789ab".into(),
            timestamp_utc: "2026-11-23T14:00:00Z".into(),
            os: "linux",
            arch: "x86_64",
            cores: 8,
            line_seconds: LINE_SECONDS,
            touched_file: Some("src/main.rs".into()),
            has_features: false,
            warm: vec![
                finish_row(
                    "git status",
                    vec!["git".into(), "status".into()],
                    Duration::from_millis(12),
                    Ok((0, String::new())),
                ),
                finish_row(
                    "slow step",
                    vec!["cargo".into(), "build".into()],
                    Duration::from_millis(12_340),
                    Ok((0, String::new())),
                ),
            ],
            cold: vec![finish_row(
                "cargo build (cold)",
                vec!["cargo".into(), "build".into()],
                Duration::from_millis(15_000),
                Ok((0, String::new())),
            )],
        };
        assert!(!report.passed());
        let table = report.render_table();
        assert!(table.contains("under"));
        assert!(table.contains("over"));
        assert!(table.contains("Cold (reported apart"));
        let parsed = json::parse(&report.render_json()).unwrap();
        assert_eq!(parsed.text("schema"), SCHEMA);
        assert_eq!(
            parsed.get("warm_ok_under_line").and_then(Json::as_i64),
            Some(1)
        );
        assert_eq!(parsed.get("warm_over_line").and_then(Json::as_i64), Some(1));
        assert_eq!(parsed.get("warm_failed").and_then(Json::as_i64), Some(0));
    }
}

//! `susi-tools gates`: every gate an agent runs before a ship, side by side, inside the 10-second
//! line (the CEO, 2026-11-24: "a non negotiable mandate would be 10s. if anything goes above that,
//! it would be treated high priority and fixed immediately"). A run over the line is refused
//! (non-zero exit), same as a failed gate (T-CURSOR-1). Run one after another, Susi's gates
//! took 14.3 s; side by side, 6.6 s.
//!
//! **The 10-second wall (phase 1)** — three lanes, each in its own build folder (Cargo lets one
//! build at a time use a folder, so steps sharing one across lanes wait on each other):
//! - the whole suite (`cargo nextest run --workspace`), with the repository's nextest profile
//!   `gates` when `.config/nextest.toml` names one;
//! - when packages have features, their suite then native Clippy with `--no-default-features` in
//!   `<target>/no-default-features` (Clippy reuses the suite build; a separate
//!   `no-default-features-lints` folder cost **52 s** cold under load, T-CURSOR-3);
//! - host `cargo fmt --check` and host Clippy.
//!
//! **Product-target Clippy (phase 2, apart)** — `cargo clippy --target …` for every `--target`
//! named, host and customer builds side by side, with `SUSI_CROSS_CHECK=1`. A cold rebuild of
//! five triples took **38–40.5 s** on CachyOS (T-CURSOR-2): unfit for the 10-second wall. It
//! still must pass, and its wall is printed, but it does not count toward the line (the mandate's
//! measured gate set is phase 1). Keep `CARGO_TARGET_DIR` warm so phase 2 stays a second or two.
//!
//! **The walks (phase 2, apart)** — when `.config/nextest.toml` names the profile `gates-walks`,
//! the whole suite with it runs beside product-target Clippy: the repository's end-to-end walks
//! (its delivery's dry runs, its hosts' scripts, an IDE's toolchain), which its `gates` profile
//! leaves out. They must pass; their wall is printed apart (the CEO, 2026-11-26: trim the suites
//! so cloud gates fit 4 cores, with the walks apart; T-CLOUDCLAUDE-6).
//!
//! **A workspace apart (phase 2)** — a folder below the root whose `Cargo.toml` has a
//! `[workspace]` table gets its own format, Clippy and tests when a change touches it, or touches
//! a package it depends on by path, followed through the packages it names (T-CLAUDE-454).
//! Its members also run Clippy for every requested target with `SUSI_CROSS_CHECK=1`.
//!
//! **Formatting** checks the packages a change touches since its merge base with `origin/main`
//! (`cargo fmt --check -p …`): rustfmt formats file by file and `main` is formatted, so an
//! untouched package cannot have changed. All of them when no base is known or a root file that
//! steers rustfmt changed (`rustfmt.toml`, `rust-toolchain`); none when no Rust or manifest did.

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::speed::{featured_packages, LINE_SECONDS};

mod copies;

/// One gate's result.
pub struct Gate {
    pub name: String,
    pub command: String,
    pub seconds: f64,
    pub ok: bool,
    /// The end of its output when it failed.
    pub tail: String,
}

/// Every gate, and how long the 10-second wall and the cross phase took.
pub struct Outcome {
    pub gates: Vec<Gate>,
    /// Phase 1 wall clock (suites + native lints): the 10-second line.
    pub seconds: f64,
    /// Phase 2 wall clock for cross Clippy, walks and nested workspaces. `None` when all are absent.
    pub cross_seconds: Option<f64>,
    copies: String,
}

impl Outcome {
    /// Every functional gate succeeded (tests, fmt, Clippy, cross Clippy). The wall-clock line
    /// is separate: [`Self::within_line`].
    pub fn gates_ok(&self) -> bool {
        self.gates.iter().all(|g| g.ok)
    }

    /// Phase-1 wall clock at or under the 10-second line.
    pub fn within_line(&self) -> bool {
        self.seconds <= LINE_SECONDS
    }

    /// Ship and land only when every gate passed *and* phase 1 stayed inside the line (CEO,
    /// 2026-11-24: the 10-second mandate is non-negotiable; a loud print alone did not stop a
    /// ship, so over-line is a refuse). Cross Clippy time is apart (T-CURSOR-2).
    pub fn passed(&self) -> bool {
        self.gates_ok() && self.within_line()
    }

    /// Whether this run is measured once more before it is refused ([`run_warm`]): every gate
    /// passed and only the wall clock was over the line. A first run on a tip just checked out,
    /// or after an edit, also compiles, beside whatever else the host runs then (a release's
    /// journeys, another agent's gates, the lander building a landing); the line is for the
    /// gates warm (Susi's `AGENTS.md`, "Speed"), which the second run measures, and work still
    /// over the line warm is refused. On 2026-11-25 every commit the lander refused for speed
    /// alone landed unchanged later, and on 2026-11-26 it refused three at 23.5 to 27.6 s while
    /// its own release ran (T-CLAUDE-384). The same day an agent's own gates read 9.8 and 10.5 s
    /// while the lander compiled a landing beside them, and 2.6 s at the next run, so the command
    /// `gates` measures again as the lander does (T-CLAUDE-432).
    pub fn measure_again(&self) -> bool {
        self.gates_ok() && !self.within_line()
    }

    /// Failures for land/Board when `passed` is false: each failed gate, or the speed line.
    pub fn failures(&self) -> Vec<(String, String)> {
        let mut out: Vec<(String, String)> = self
            .gates
            .iter()
            .filter(|g| !g.ok)
            .map(|g| (g.name.clone(), g.tail.clone()))
            .collect();
        if self.gates_ok() && !self.within_line() {
            out.push((
                "speed".to_owned(),
                self.over_line(load_average().as_deref()),
            ));
        }
        out
    }

    /// Why a run over the line was refused, with each gate's own time and the machine's load
    /// when it ended: a reader on the Board otherwise cannot tell which gate is the long pole,
    /// or a slow commit from a busy host (Board 8704, 2026-11-25: every landing refused at 11 to
    /// 12 s, and the answer said only the total).
    fn over_line(&self, load: Option<&str>) -> String {
        let mut text = format!(
            "OVER the {LINE_SECONDS:.0}-second line in {:.1} s; fix it before any other work",
            self.seconds
        );
        let mut gates: Vec<&Gate> = self.gates.iter().collect();
        gates.sort_by(|a, b| b.seconds.total_cmp(&a.seconds));
        for gate in gates {
            text.push_str(&format!("\n  {:>5.1} s  {}", gate.seconds, gate.name));
        }
        if let Some(load) = load {
            text.push_str(&format!("\nload average at the end: {load}"));
        }
        text
    }

    pub fn render(&self, work: &Path) -> String {
        let mut out = format!(
            "gates for {}: {} gate(s) side by side in {:.1} s (line {LINE_SECONDS:.1} s)",
            work.display(),
            self.gates.len(),
            self.seconds
        );
        if let Some(cross) = self.cross_seconds {
            out.push_str(&format!(
                "; cross and walks {cross:.1} s (apart from the {LINE_SECONDS:.0}-second line)"
            ));
        }
        out.push('\n');
        for gate in &self.gates {
            out.push_str(&format!(
                "  {:<6} {:>5.1} s  {}: {}\n",
                if gate.ok { "ok" } else { "FAILED" },
                gate.seconds,
                gate.name,
                gate.command
            ));
        }
        for gate in self.gates.iter().filter(|g| !g.ok) {
            out.push_str(&format!("\n== {} failed:\n{}\n", gate.name, gate.tail));
        }
        let verdict = if self.gates_ok() {
            "every gate passed"
        } else {
            "a gate failed"
        };
        let line = if self.within_line() {
            format!(
                "{verdict}, in {:.1} s, under the {LINE_SECONDS:.0}-second line\n",
                self.seconds
            )
        } else {
            format!(
                "{verdict}, in {:.1} s: OVER the {LINE_SECONDS:.0}-second line, a defect of the highest priority; fix it before any other work (refused)\n",
                self.seconds
            )
        };
        out.push_str(&line);
        out.push_str(&self.copies);
        out
    }
}

/// The machine's load averages over 1, 5 and 15 minutes, where the system says them (Linux).
fn load_average() -> Option<String> {
    let text = std::fs::read_to_string("/proc/loadavg").ok()?;
    let three: Vec<&str> = text.split_whitespace().take(3).collect();
    (three.len() == 3).then(|| three.join(" "))
}

/// One gate to run: its name, cargo's arguments, the target folder and whether it is Susi's
/// cross-target check.
struct Step {
    name: &'static str,
    args: Vec<String>,
    target: Option<PathBuf>,
    cross: bool,
}

fn step(name: &'static str, args: &[&str], target: Option<&Path>, cross: bool) -> Step {
    Step {
        name,
        args: args.iter().map(|a| (*a).to_owned()).collect(),
        target: target.map(Path::to_path_buf),
        cross,
    }
}

/// Runs the gates, and once more when every gate passed and only the wall clock was over the
/// line ([`Outcome::measure_again`]), so what is judged is the gates warm. Returns the run to
/// judge and, when it measured twice, how long the first run took. One rule for the command
/// `gates` and for `land`.
pub fn run_warm(work: &Path, targets: &[String]) -> Result<(Outcome, Option<f64>), String> {
    let first = run(work, targets)?;
    if !first.measure_again() {
        return Ok((first, None));
    }
    let before = first.seconds;
    Ok((run(work, targets)?, Some(before)))
}

/// Runs every gate of the Cargo checkout at `work`. Phase 1 times the suites and native lints
/// against the 10-second line. Phase 2 times cross Clippy, walks and nested workspaces separately.
pub fn run(work: &Path, targets: &[String]) -> Result<Outcome, String> {
    if !work.join("Cargo.toml").is_file() {
        return Err(format!("not a Cargo checkout: {}", work.display()));
    }
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("target"));
    let featured = featured_packages(work);
    let Plan {
        mut line_lanes,
        mut cross_steps,
    } = plan(
        &base,
        featured.as_deref(),
        targets,
        Profiles {
            whole: nextest_profile(work, GATES_PROFILE).then_some(GATES_PROFILE),
            no_default: nextest_profile(work, GATES_NO_DEFAULT_PROFILE)
                .then_some(GATES_NO_DEFAULT_PROFILE),
        },
    );
    if let Some(format) = line_lanes[2].iter_mut().find(|s| s.name == "format") {
        format.args = format_args(touched_packages(work).as_deref());
    }
    if nextest_profile(work, GATES_WALKS_PROFILE) {
        cross_steps.push(walks_step());
    }
    // A workspace of its own inside the checkout is gated by itself when a change touches it, or
    // touches a package it depends on by path (Susi Wall links every brick, T-CLAUDE-454).
    for apart in touched_files(work)
        .map(|files| {
            let mut all = workspaces_apart(work, &files);
            all.extend(workspaces_depending(
                work,
                &files,
                &tracked_workspaces_apart(work),
            ));
            all.sort();
            all.dedup();
            all
        })
        .unwrap_or_default()
    {
        cross_steps.extend(apart_steps(&base, &apart, targets));
    }

    let line_started = Instant::now();
    let mut gates = run_lanes(work, &line_lanes);
    let seconds = line_started.elapsed().as_secs_f64();

    let cross_seconds = if cross_steps.is_empty() {
        None
    } else {
        let cross_started = Instant::now();
        // Each cross step in its own thread: host and customer use different folders.
        let cross_gates = std::thread::scope(|scope| {
            let handles: Vec<_> = cross_steps
                .iter()
                .map(|s| scope.spawn(move || run_step(work, s)))
                .collect();
            handles
                .into_iter()
                .map(|h| {
                    h.join().unwrap_or_else(|_| Gate {
                        name: "cross".to_owned(),
                        command: String::new(),
                        seconds: 0.0,
                        ok: false,
                        tail: "cross step panicked".to_owned(),
                    })
                })
                .collect::<Vec<Gate>>()
        });
        gates.extend(cross_gates);
        Some(cross_started.elapsed().as_secs_f64())
    };

    let copies = copies::report(work);
    let outcome = Outcome {
        gates,
        seconds,
        cross_seconds,
        copies,
    };
    record_tree_receipt(work, outcome.passed());
    Ok(outcome)
}

/// Local ref namespace storing trees whose `susi-tools gates` passed inside the 10-second line.
pub const GATED_PREFIX: &str = "refs/susi-tools/gated/";

/// Records (when `passed` is true) or revokes (when `passed` is false) the passing-gates receipt
/// for `work`'s clean index tree (`refs/susi-tools/gated/<tree_oid>`). When tracked files have
/// unstaged edits, `gates` ran on bytes the index does not hold, so the tree is not stamped.
pub fn record_tree_receipt(work: &Path, passed: bool) {
    let Ok(repo) = crate::git::Repo::open(work) else {
        return;
    };
    let unstaged_clean = repo.read(&["diff", "--quiet"]).is_ok();
    let Some(tree) = repo
        .try_read(&["write-tree"])
        .map(|s| s.trim().to_owned())
        .filter(|s| !s.is_empty())
    else {
        return;
    };
    let refname = format!("{GATED_PREFIX}{tree}");
    if passed && unstaged_clean {
        let _ = repo.try_read(&["update-ref", &refname, &tree]);
    } else {
        let _ = repo.try_read(&["update-ref", "-d", &refname]);
    }
}

/// Whether `tree` has a passing under-10s `susi-tools gates` receipt in `repo`.
pub fn has_passed_tree(repo: &crate::git::Repo, tree: &str) -> bool {
    !tree.is_empty()
        && repo
            .try_read(&[
                "rev-parse",
                "-q",
                "--verify",
                &format!("{GATED_PREFIX}{tree}"),
            ])
            .is_some()
}

fn run_lanes(work: &Path, lanes: &[Vec<Step>]) -> Vec<Gate> {
    std::thread::scope(|scope| {
        let handles: Vec<_> = lanes
            .iter()
            .map(|lane| {
                scope.spawn(move || {
                    lane.iter()
                        .map(|s| run_step(work, s))
                        .collect::<Vec<Gate>>()
                })
            })
            .collect();
        handles
            .into_iter()
            .flat_map(|h| h.join().unwrap_or_default())
            .collect::<Vec<Gate>>()
    })
}

/// The nextest profile the whole suite runs with in the gates, when the repository names it.
pub const GATES_PROFILE: &str = "gates";
/// The nextest profile of the walks, run apart from the line when the repository names it.
pub const GATES_WALKS_PROFILE: &str = "gates-walks";

/// The walks: the whole suite with the profile `gates-walks`, in phase 2. A profile that matches
/// nothing passes, as an empty walk proves nothing wrong.
fn walks_step() -> Step {
    let mut args = whole_suite_args(Some(GATES_WALKS_PROFILE));
    args.push("--no-tests=pass".to_owned());
    Step {
        name: "walks",
        args,
        target: None,
        cross: false,
    }
}

/// `cargo fmt`'s arguments for `packages`: every package when `None`, those named otherwise,
/// and nothing to check (`cargo fmt --version`) when the list is empty.
fn format_args(packages: Option<&[String]>) -> Vec<String> {
    match packages {
        None => owned(["fmt", "--all", "--check"]),
        Some([]) => owned(["fmt", "--version"]),
        Some(packages) => {
            let mut args = owned(["fmt", "--check"]);
            for package in packages {
                args.extend(["-p".to_owned(), package.clone()]);
            }
            args
        }
    }
}

/// The files a change touches since its merge base with `origin/main`, staged, unstaged and
/// untracked; `None` when there is no such base.
fn touched_files(work: &Path) -> Option<Vec<String>> {
    let repo = crate::git::Repo::open(work).ok()?;
    let base = repo
        .try_read(&["merge-base", "HEAD", "refs/remotes/origin/main"])?
        .trim()
        .to_owned();
    if base.is_empty() {
        return None;
    }
    let mut files: Vec<String> = repo
        .try_read(&["diff", "--name-only", "--no-renames", &base])?
        .lines()
        .chain(
            repo.try_read(&["ls-files", "--others", "--exclude-standard"])
                .unwrap_or_default()
                .lines(),
        )
        .filter(|l| !l.is_empty())
        .map(str::to_owned)
        .collect();
    files.sort();
    files.dedup();
    Some(files)
}

/// The packages whose Rust sources or manifests `files` (paths from the checkout's root) touch,
/// each the package of the nearest `Cargo.toml` above the file; `None` (check every package) when
/// a file that steers rustfmt for all of them changed.
fn packages_of(work: &Path, files: &[String]) -> Option<Vec<String>> {
    const EVERYWHERE: [&str; 4] = [
        "rustfmt.toml",
        ".rustfmt.toml",
        "rust-toolchain",
        "rust-toolchain.toml",
    ];
    let mut packages = Vec::new();
    for file in files {
        let path = Path::new(file);
        let name = path.file_name().and_then(|n| n.to_str()).unwrap_or("");
        if EVERYWHERE.contains(&name) {
            return None;
        }
        let rust = path.extension().is_some_and(|e| e == "rs") || name == "Cargo.toml";
        if !rust {
            continue;
        }
        // A file of a workspace apart is no package of this one: its own steps check it.
        if workspace_apart(work, file).is_some() {
            continue;
        }
        let mut dir = work.join(path);
        while dir.pop() {
            let manifest = dir.join("Cargo.toml");
            if let Some(package) = package_name(&manifest) {
                packages.push(package);
                break;
            }
            if dir == work {
                break;
            }
        }
    }
    packages.sort();
    packages.dedup();
    Some(packages)
}

/// The workspace of its own that `file` (a path from the checkout's root) belongs to, when it is
/// not the checkout's: the nearest folder above it, below the root, whose `Cargo.toml` has a
/// `[workspace]` table. Such a workspace is built apart (a toolkit the main one must not pay
/// for, say), so `cargo` run at the root knows none of its packages.
fn workspace_apart(work: &Path, file: &str) -> Option<String> {
    let mut dir = Path::new(file).parent()?;
    while !dir.as_os_str().is_empty() {
        let manifest = std::fs::read_to_string(work.join(dir).join("Cargo.toml"));
        if manifest.is_ok_and(|text| text.lines().any(|line| line.trim() == "[workspace]")) {
            return Some(dir.to_string_lossy().replace('\\', "/"));
        }
        dir = dir.parent()?;
    }
    None
}

/// The workspaces apart that `files` touch, each once.
fn workspaces_apart(work: &Path, files: &[String]) -> Vec<String> {
    let mut found: Vec<String> = files
        .iter()
        .filter_map(|file| workspace_apart(work, file))
        .collect();
    found.sort();
    found.dedup();
    found
}

/// Every workspace apart the checkout tracks: each `Cargo.toml` below the root with a
/// `[workspace]` table, by its folder.
fn tracked_workspaces_apart(work: &Path) -> Vec<String> {
    let Some(listed) = crate::git::Repo::open(work)
        .ok()
        .and_then(|repo| repo.try_read(&["ls-files", "*Cargo.toml"]))
    else {
        return Vec::new();
    };
    let mut found: Vec<String> = listed
        .lines()
        .filter(|l| l.contains('/'))
        .filter_map(|manifest| workspace_apart(work, manifest))
        .filter(|dir| work.join(dir).join("Cargo.toml").is_file())
        .collect();
    found.sort();
    found.dedup();
    found
}

/// The workspaces apart among `candidates` that depend by path, directly or through the packages
/// they name, on a package `files` touch: a change to a brick is checked in the workspace that
/// links it too, though no file of that workspace changed.
fn workspaces_depending(work: &Path, files: &[String], candidates: &[String]) -> Vec<String> {
    candidates
        .iter()
        .filter(|dir| {
            let deps = path_closure(work, dir);
            files.iter().any(|file| {
                !file.starts_with(&format!("{dir}/"))
                    && deps.iter().any(|dep| file.starts_with(&format!("{dep}/")))
            })
        })
        .cloned()
        .collect()
}

/// The package folders (from the checkout's root) the package in `dir` depends on by path, and
/// those they depend on, each once: inline `path = "..."` entries, dependencies inherited from a
/// workspace (`{ workspace = true }`) read in that workspace's manifest, and path patches.
fn path_closure(work: &Path, dir: &str) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut queue = vec![dir.to_owned()];
    while let Some(next) = queue.pop() {
        let Ok(text) = std::fs::read_to_string(work.join(&next).join("Cargo.toml")) else {
            continue;
        };
        let inherited = workspace_paths(work, &next);
        for line in text.lines() {
            let line = line.trim();
            let Some((name, value)) = line.split_once('=') else {
                continue;
            };
            let value = value.trim();
            if !value.starts_with('{') {
                continue;
            }
            let found = inline_path(value)
                .and_then(|path| normal_join(&next, path))
                .or_else(|| {
                    value
                        .contains("workspace = true")
                        .then(|| inherited.iter().find(|(n, _)| n == name.trim()))
                        .flatten()
                        .map(|(_, path)| path.clone())
                });
            if let Some(found) = found {
                if found != dir && !seen.contains(&found) {
                    seen.push(found.clone());
                    queue.push(found);
                }
            }
        }
    }
    seen.sort();
    seen
}

/// The `path` of an inline table (`{ path = "x", ... }`).
fn inline_path(value: &str) -> Option<&str> {
    let rest = &value[value.find("path")? + 4..];
    let rest = rest
        .trim_start()
        .strip_prefix('=')?
        .trim_start()
        .strip_prefix('"')?;
    rest.split('"').next()
}

/// The path dependencies the workspace `dir` inherits from declare, by name, as folders from the
/// checkout's root: the nearest manifest at or above it with a `[workspace]` table.
fn workspace_paths(work: &Path, dir: &str) -> Vec<(String, String)> {
    let mut at = Path::new(dir).to_path_buf();
    loop {
        let rel = at.to_string_lossy().replace('\\', "/");
        let text = std::fs::read_to_string(work.join(&at).join("Cargo.toml")).unwrap_or_default();
        if text.lines().any(|l| {
            let l = l.trim();
            l == "[workspace]" || l.starts_with("[workspace.")
        }) {
            return text
                .lines()
                .filter_map(|line| {
                    let (name, value) = line.trim().split_once('=')?;
                    let path =
                        inline_path(value.trim()).filter(|_| value.trim().starts_with('{'))?;
                    Some((name.trim().to_owned(), normal_join(&rel, path)?))
                })
                .collect();
        }
        if !at.pop() {
            return Vec::new();
        }
    }
}

/// `path` taken from the folder `from` (both from the checkout's root), `..` resolved; `None` when
/// it leaves the checkout.
fn normal_join(from: &str, path: &str) -> Option<String> {
    let mut parts: Vec<&str> = from
        .split('/')
        .filter(|p| !p.is_empty() && *p != ".")
        .collect();
    for part in path.split('/') {
        match part {
            "" | "." => {}
            ".." => {
                parts.pop()?;
            }
            other => parts.push(other),
        }
    }
    Some(parts.join("/"))
}

/// Required phase-2 checks for the workspace in `dir`. Cross Clippy uses a separate build folder
/// so it can run alongside the host checks.
fn apart_steps(base: &Path, dir: &str, targets: &[String]) -> Vec<Step> {
    let manifest = format!("{dir}/Cargo.toml");
    let target = Some(base.join("apart").join(dir));
    let with = |args: &[&str]| -> Vec<String> {
        args.iter()
            .map(|arg| {
                if *arg == "<manifest>" {
                    manifest.clone()
                } else {
                    (*arg).to_owned()
                }
            })
            .collect()
    };
    let mut steps = vec![
        Step {
            name: "format, a workspace apart",
            args: with(&["fmt", "--check", "--manifest-path", "<manifest>"]),
            target: None,
            cross: false,
        },
        Step {
            name: "clippy, a workspace apart",
            args: with(&[
                "clippy",
                "--manifest-path",
                "<manifest>",
                "--all-targets",
                "--offline",
                "--locked",
                "--",
                "-D",
                "warnings",
            ]),
            target: target.clone(),
            cross: false,
        },
        Step {
            name: "tests, a workspace apart",
            args: with(&[
                "test",
                "--offline",
                "--locked",
                "--manifest-path",
                "<manifest>",
            ]),
            target,
            cross: false,
        },
    ];
    if !targets.is_empty() {
        let mut flags = vec!["--manifest-path", manifest.as_str()];
        flags.extend(target_flags(targets));
        steps.push(Step {
            name: "clippy, every product target, a workspace apart",
            args: host_clippy_args(&flags),
            target: Some(base.join("apart-cross").join(dir)),
            cross: true,
        });
    }
    steps
}

/// The `[package]` name of the manifest at `path`, if it names one.
fn package_name(path: &Path) -> Option<String> {
    let text = std::fs::read_to_string(path).ok()?;
    let mut in_package = false;
    for line in text.lines() {
        let line = line.trim();
        if line.starts_with('[') {
            in_package = line == "[package]";
            continue;
        }
        if in_package {
            if let Some(value) = line.strip_prefix("name") {
                let value = value.trim_start().strip_prefix('=')?.trim();
                return Some(value.trim_matches('"').to_owned());
            }
        }
    }
    None
}

/// The packages `cargo fmt --check` looks at: those a change touches, or `None` for all.
fn touched_packages(work: &Path) -> Option<Vec<String>> {
    packages_of(work, &touched_files(work)?)
}
/// The nextest profile the suite without default features runs with, when the repository names it.
pub const GATES_NO_DEFAULT_PROFILE: &str = "gates-no-default-features";

/// Whether the repository's nextest settings (`.config/nextest.toml`) name the profile `name`.
pub fn nextest_profile(work: &Path, name: &str) -> bool {
    std::fs::read_to_string(work.join(".config/nextest.toml")).is_ok_and(|text| {
        text.lines()
            .any(|l| l.trim() == format!("[profile.{name}]"))
    })
}

/// Whole-suite nextest args (`--workspace`), with an optional profile. Shared with `speed`.
pub fn whole_suite_args(profile: Option<&str>) -> Vec<String> {
    let mut args = owned(["nextest", "run", "--workspace", "--offline", "--locked"]);
    if let Some(profile) = profile {
        args.extend(owned(["--profile", profile]));
    }
    args
}

/// Customer-suite nextest args (`--no-default-features`), one `-p` per package (or `--workspace`
/// when the list is empty). Shared with `speed`.
pub fn customer_suite_args(packages: &[String], profile: Option<&str>) -> Vec<String> {
    let mut args = owned([
        "nextest",
        "run",
        "--offline",
        "--locked",
        "--no-default-features",
    ]);
    if packages.is_empty() {
        args.push("--workspace".to_owned());
    }
    for package in packages {
        args.extend(["-p".to_owned(), package.clone()]);
    }
    if let Some(profile) = profile {
        args.extend(owned(["--profile", profile]));
    }
    args
}

fn owned<const N: usize>(args: [&str; N]) -> Vec<String> {
    args.iter().map(|a| (*a).to_owned()).collect()
}

fn target_flags(targets: &[String]) -> impl Iterator<Item = &str> {
    targets
        .iter()
        .flat_map(|target| ["--target", target.as_str()])
}

fn host_clippy_args(extra: &[&str]) -> Vec<String> {
    let mut args = owned([
        "clippy",
        "--workspace",
        "--all-targets",
        "--offline",
        "--locked",
    ]);
    args.extend(extra.iter().map(|a| (*a).to_owned()));
    args.extend(owned(["--", "-D", "warnings"]));
    args
}

fn customer_clippy_args(packages: Option<&[String]>, extra: &[&str]) -> Vec<String> {
    let mut args = vec!["clippy".to_owned()];
    match packages {
        Some(packages) if !packages.is_empty() => {
            for package in packages {
                args.extend(["-p".to_owned(), package.clone()]);
            }
        }
        _ => args.push("--workspace".to_owned()),
    }
    args.extend(owned([
        "--all-targets",
        "--offline",
        "--locked",
        "--no-default-features",
    ]));
    args.extend(extra.iter().map(|a| (*a).to_owned()));
    args.extend(owned(["--", "-D", "warnings"]));
    args
}

/// The nextest profiles of the two suites, each when the repository names it.
#[derive(Clone, Copy, Default)]
struct Profiles<'a> {
    whole: Option<&'a str>,
    no_default: Option<&'a str>,
}

struct Plan {
    /// Whole suite; customer suite then native Clippy; host fmt+clippy.
    line_lanes: [Vec<Step>; 3],
    cross_steps: Vec<Step>,
}

/// Phase-1 lanes (10-second line) and phase-2 cross steps (apart).
fn plan(
    base: &Path,
    featured: Option<&[String]>,
    targets: &[String],
    profiles: Profiles<'_>,
) -> Plan {
    let customer = base.join("no-default-features");
    let cross_refs: Vec<_> = target_flags(targets).collect();

    let tests = vec![Step {
        name: "tests",
        args: whole_suite_args(profiles.whole),
        target: None,
        cross: false,
    }];
    let mut customer_lane = Vec::new();
    let mut lints = vec![step("format", &["fmt", "--all", "--check"], None, false)];
    lints.push(Step {
        name: "clippy",
        args: host_clippy_args(&[]),
        target: None,
        cross: false,
    });
    if let Some(packages) = featured {
        // Suite then native Clippy in one folder: Clippy reuses the suite's build (T-CURSOR-3).
        // Product-target Clippy stays phase 2 so it cannot sum into the 10-second wall.
        customer_lane.push(Step {
            name: "tests, no default features",
            args: customer_suite_args(packages, profiles.no_default),
            target: Some(customer.clone()),
            cross: false,
        });
        customer_lane.push(Step {
            name: "clippy, no default features",
            args: customer_clippy_args(Some(packages), &[]),
            target: Some(customer.clone()),
            cross: false,
        });
    }

    let mut cross_steps = Vec::new();
    if !targets.is_empty() {
        cross_steps.push(Step {
            name: "clippy, every product target",
            args: host_clippy_args(&cross_refs),
            target: None,
            cross: true,
        });
        if featured.is_some() {
            cross_steps.push(Step {
                name: "clippy, every product target, no default features",
                args: customer_clippy_args(featured, &cross_refs),
                target: Some(customer.clone()),
                cross: true,
            });
        }
    }

    Plan {
        line_lanes: [tests, customer_lane, lints],
        cross_steps,
    }
}

fn run_step(work: &Path, step: &Step) -> Gate {
    let mut cmd = Command::new("cargo");
    cmd.args(&step.args)
        .current_dir(work)
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    if let Some(target) = &step.target {
        cmd.env("CARGO_TARGET_DIR", target);
    }
    if step.cross {
        cmd.env("SUSI_CROSS_CHECK", "1");
    }
    let nextest = step.args.first().map(String::as_str) == Some("nextest");
    let threads = if nextest {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        test_threads(cores, std::env::var_os("NEXTEST_TEST_THREADS").as_deref())
    } else {
        None
    };
    if let Some(threads) = &threads {
        cmd.env("NEXTEST_TEST_THREADS", threads);
    }
    let spawn = if nextest {
        double_spawn(std::env::var_os("NEXTEST_DOUBLE_SPAWN").as_deref())
    } else {
        None
    };
    if let Some(spawn) = spawn {
        cmd.env("NEXTEST_DOUBLE_SPAWN", spawn);
    }
    let mut command = format!("cargo {}", step.args.join(" "));
    if let Some(target) = &step.target {
        command = format!("CARGO_TARGET_DIR={} {command}", target.display());
    }
    if let Some(threads) = &threads {
        command = format!("NEXTEST_TEST_THREADS={threads} {command}");
    }
    if let Some(spawn) = spawn {
        command = format!("NEXTEST_DOUBLE_SPAWN={spawn} {command}");
    }
    if step.cross {
        command = format!("SUSI_CROSS_CHECK=1 {command}");
    }
    let started = Instant::now();
    let (ok, tail) = match cmd.output() {
        Ok(out) => {
            let text = format!(
                "{}{}",
                String::from_utf8_lossy(&out.stdout),
                String::from_utf8_lossy(&out.stderr)
            );
            let lines: Vec<&str> = text.lines().collect();
            (
                out.status.success(),
                lines[lines.len().saturating_sub(25)..].join("\n"),
            )
        }
        Err(e) => (false, format!("cargo does not start: {e}")),
    };
    Gate {
        name: step.name.to_owned(),
        command,
        seconds: started.elapsed().as_secs_f64(),
        ok,
        tail,
    }
}

/// The test threads a suite runs with on a machine of `cores`, unless `named` (the caller's
/// `NEXTEST_TEST_THREADS`) says: twice the cores. Susi's tests wait on the programs they start
/// (Git, its own program, stand-in agents) more than they compute, so as many threads as cores, as
/// nextest gives, left those cores idle: on a cloud VM's 4 cores Susi's gates took 24.3 s with 4
/// test threads, 18.4 s with 8 and 17.9 s with 16 (Susi's docs/speed.md, "The gates on a cloud
/// VM's 4 cores"; T-CLOUDCLAUDE-1).
fn test_threads(cores: usize, named: Option<&std::ffi::OsStr>) -> Option<String> {
    match named {
        Some(_) => None,
        None => Some(cores.max(1).saturating_mul(2).to_string()),
    }
}

/// nextest's double spawn, unless `named` (the caller's `NEXTEST_DOUBLE_SPAWN`) says: off. nextest
/// starts itself again before each test so that Ctrl-Z in a terminal stops the test too; the gates
/// run no terminal. On a cloud VM's 4 cores it was about 1.5 of the 21 s of processor time of
/// Susi's customer suite (352 starts of nextest's 32 MB program), and Susi's phase 1 took 8.1 to
/// 8.4 s without it, 9.4 to 10.0 s with it (Susi's docs/speed.md, "The walks run apart from the
/// line"; T-CLOUDCLAUDE-7).
fn double_spawn(named: Option<&std::ffi::OsStr>) -> Option<&'static str> {
    match named {
        Some(_) => None,
        None => Some("0"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ok_outcome(seconds: f64, cross_seconds: Option<f64>) -> Outcome {
        Outcome {
            gates: vec![Gate {
                name: "tests".into(),
                command: "cargo nextest".into(),
                seconds: 1.0,
                ok: true,
                tail: String::new(),
            }],
            seconds,
            cross_seconds,
            copies: String::new(),
        }
    }

    #[test]
    fn only_a_run_over_the_line_with_every_gate_passing_is_measured_again() {
        assert!(ok_outcome(23.5, None).measure_again(), "slow, all passing");
        assert!(!ok_outcome(4.0, None).measure_again(), "inside the line");
        assert!(!ok_outcome(10.0, None).measure_again(), "at the line");
        let mut failing = ok_outcome(23.5, None);
        failing.gates[0].ok = false;
        assert!(
            !failing.measure_again(),
            "a failing gate is refused at once"
        );
        // Product-target Clippy's own phase is apart from the line, so it asks for nothing.
        assert!(!ok_outcome(4.0, Some(45.0)).measure_again());
    }

    /// The walks run apart, with their profile, and a profile matching nothing passes; fmt looks
    /// at the packages a change touches, all of them when rustfmt's own settings change.
    #[test]
    fn the_walks_run_apart_and_fmt_checks_the_packages_a_change_touches() {
        let walks = walks_step();
        assert_eq!(walks.name, "walks");
        assert!(!walks.cross);
        assert!(walks.args.ends_with(&[
            "--profile".to_owned(),
            "gates-walks".to_owned(),
            "--no-tests=pass".to_owned()
        ]));

        assert_eq!(format_args(None), ["fmt", "--all", "--check"]);
        assert_eq!(format_args(Some(&[])), ["fmt", "--version"]);
        assert_eq!(
            format_args(Some(&["a".to_owned(), "b-c".to_owned()])),
            ["fmt", "--check", "-p", "a", "-p", "b-c"]
        );

        let dir = std::env::temp_dir().join(format!("susi-tools-fmt-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in [
            ("Cargo.toml", "[workspace]\nmembers = [\"crates/a\"]\n\n[package]\nname = \"root\"\n"),
            ("crates/a/Cargo.toml", "[package]\nname = \"pkg-a\"\nversion = \"0.1.0\"\n\n[dependencies]\nname = \"x\"\n"),
            ("crates/a/src/lib.rs", ""),
            ("crates/a/src/deep/m.rs", ""),
            ("src/main.rs", ""),
        ] {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let files = |list: &[&str]| list.iter().map(|f| (*f).to_owned()).collect::<Vec<_>>();
        assert_eq!(
            packages_of(
                &dir,
                &files(&["crates/a/src/deep/m.rs", "docs/x.md", "src/main.rs"])
            ),
            Some(vec!["pkg-a".to_owned(), "root".to_owned()])
        );
        assert_eq!(
            packages_of(&dir, &files(&["crates/a/Cargo.toml"])),
            Some(vec!["pkg-a".to_owned()])
        );
        assert_eq!(
            packages_of(&dir, &files(&["docs/x.md", "README.md"])),
            Some(vec![])
        );
        assert_eq!(
            packages_of(&dir, &files(&["src/main.rs", "rustfmt.toml"])),
            None
        );
        assert_eq!(
            package_name(&dir.join("crates/a/Cargo.toml")).as_deref(),
            Some("pkg-a")
        );

        // A workspace of its own inside the checkout is not the main one's package: its files
        // are left out of the main format check and gated by its own steps, in phase 2.
        for (path, text) in [
            (
                "apart/Cargo.toml",
                "[workspace]\n\n[package]\nname = \"native\"\n",
            ),
            ("apart/src/main.rs", ""),
            ("apart/src/views/one.rs", ""),
        ] {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        let touched = files(&["apart/src/views/one.rs", "apart/Cargo.toml", "src/main.rs"]);
        assert_eq!(packages_of(&dir, &touched), Some(vec!["root".to_owned()]));
        assert_eq!(workspaces_apart(&dir, &touched), ["apart"]);
        assert_eq!(workspace_apart(&dir, "crates/a/src/lib.rs"), None);
        assert_eq!(
            workspace_apart(&dir, "src/main.rs"),
            None,
            "the root is the main workspace"
        );
        let steps = apart_steps(Path::new("/t"), "apart", &[]);
        assert_eq!(
            lane_names(&steps),
            [
                "format, a workspace apart",
                "clippy, a workspace apart",
                "tests, a workspace apart"
            ]
        );
        assert_eq!(
            steps[2].args,
            [
                "test",
                "--offline",
                "--locked",
                "--manifest-path",
                "apart/Cargo.toml"
            ]
        );
        assert_eq!(
            steps[1].args[..3],
            ["clippy", "--manifest-path", "apart/Cargo.toml"]
        );
        assert_eq!(steps[0].target, None);
        assert_eq!(
            steps[2].target.as_deref(),
            Some(Path::new("/t/apart/apart"))
        );
        assert!(steps.iter().all(|s| !s.cross));
        std::fs::remove_dir_all(&dir).unwrap();
    }

    #[test]
    fn apart_cross_clippy_checks_the_whole_workspace_for_each_requested_target() {
        let base = Path::new("/t");
        let dir = "nested/ui";
        let targets = [
            "aarch64-apple-ios".to_owned(),
            "aarch64-linux-android".to_owned(),
        ];
        let host = apart_steps(base, dir, &[]);
        let steps = apart_steps(base, dir, &targets);
        assert_eq!(steps.len(), host.len() + 1);
        for (actual, expected) in steps.iter().zip(&host) {
            assert_eq!(actual.name, expected.name);
            assert_eq!(actual.args, expected.args);
            assert_eq!(actual.target, expected.target);
            assert!(!actual.cross);
        }
        let cross = steps.last().unwrap();
        assert!(cross.cross);
        assert_eq!(cross.target, Some(base.join("apart-cross").join(dir)));
        assert_eq!(
            cross.args,
            [
                "clippy",
                "--workspace",
                "--all-targets",
                "--offline",
                "--locked",
                "--manifest-path",
                "nested/ui/Cargo.toml",
                "--target",
                "aarch64-apple-ios",
                "--target",
                "aarch64-linux-android",
                "--",
                "-D",
                "warnings",
            ]
        );
    }

    /// A workspace apart that links a brick by path is gated when the brick changes, through
    /// the packages it names and what they inherit from the main workspace; a change to docs
    /// alone gates nothing apart (T-CLAUDE-454: Susi Wall broke on main from a brick's change).
    #[test]
    fn a_workspace_apart_is_gated_when_a_brick_it_links_by_path_changes() {
        let dir = std::env::temp_dir().join(format!("susi-tools-apart-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        for (path, text) in [
            (
                "Cargo.toml",
                "[workspace]\nmembers = [\"crates/*\"]\n\n[workspace.dependencies]\n\
                 brick = { path = \"crates/brick\" }\nleaf = { path = \"crates/leaf\" }\n",
            ),
            (
                "crates/catalogue/Cargo.toml",
                "[package]\nname = \"catalogue\"\n\n[dependencies]\nbrick = { workspace = true }\n",
            ),
            (
                "crates/brick/Cargo.toml",
                "[package]\nname = \"brick\"\n\n[dependencies]\nleaf = { workspace = true, optional = true }\n",
            ),
            ("crates/leaf/Cargo.toml", "[package]\nname = \"leaf\"\n"),
            ("crates/alone/Cargo.toml", "[package]\nname = \"alone\"\n"),
            (
                "desktop/Cargo.toml",
                "[workspace]\n\n[package]\nname = \"wall\"\n\n[[bin]]\nname = \"wall\"\n\
                 path = \"src/main.rs\"\n\n[dependencies]\n\
                 catalogue = { path = \"../crates/catalogue\" }\n\n[patch.crates-io]\n\
                 idna = { path = \"../vendor/idna\" }\n",
            ),
        ] {
            let p = dir.join(path);
            std::fs::create_dir_all(p.parent().unwrap()).unwrap();
            std::fs::write(p, text).unwrap();
        }
        assert_eq!(
            path_closure(&dir, "desktop"),
            [
                "crates/brick",
                "crates/catalogue",
                "crates/leaf",
                "vendor/idna"
            ]
        );
        let apart = ["desktop".to_owned()];
        let gated = |list: &[&str]| {
            let files: Vec<String> = list.iter().map(|f| (*f).to_owned()).collect();
            workspaces_depending(&dir, &files, &apart)
        };
        assert_eq!(gated(&["crates/leaf/src/lib.rs"]), ["desktop"]);
        assert_eq!(
            gated(&["docs/desktop.md", "crates/catalogue/src/lib.rs"]),
            ["desktop"]
        );
        assert!(gated(&["docs/desktop.md", "README.md"]).is_empty());
        assert!(gated(&["crates/alone/src/lib.rs"]).is_empty());
        assert!(
            gated(&["desktop/src/main.rs"]).is_empty(),
            "its own files gate it already"
        );
        assert_eq!(normal_join("desktop", "../../x"), None);
        std::fs::remove_dir_all(&dir).unwrap();
    }

    fn lane_names(lane: &[Step]) -> Vec<&str> {
        lane.iter().map(|s| s.name).collect()
    }

    #[test]
    fn each_suite_gets_twice_the_cores_unless_told() {
        assert_eq!(test_threads(4, None).as_deref(), Some("8"));
        assert_eq!(test_threads(28, None).as_deref(), Some("56"));
        assert_eq!(test_threads(0, None).as_deref(), Some("2"));
        assert_eq!(test_threads(4, Some(std::ffi::OsStr::new("3"))), None);
    }

    #[test]
    fn each_suite_runs_without_nextests_double_spawn_unless_told() {
        assert_eq!(double_spawn(None), Some("0"));
        assert_eq!(double_spawn(Some(std::ffi::OsStr::new("1"))), None);
    }

    /// Customer native Clippy runs after the customer suite in the same folder (T-CURSOR-3).
    /// Product-target Clippy is phase 2 (apart): cold five-triple rebuilds took 38–40.5 s.
    #[test]
    fn lanes_run_customer_native_clippy_after_its_tests() {
        let base = Path::new("/target");
        let customer = base.join("no-default-features");
        let featured = vec!["susi-commercial".to_owned()];
        let targets = vec!["x86_64-pc-windows-msvc".to_owned()];
        let Plan {
            line_lanes: [tests, customer_lane, lints],
            cross_steps,
        } = plan(base, Some(&featured), &targets, Profiles::default());
        assert_eq!(
            lane_names(&customer_lane),
            ["tests, no default features", "clippy, no default features"]
        );
        assert!(customer_lane
            .iter()
            .all(|s| s.target.as_deref() == Some(customer.as_path())));
        assert_eq!(lane_names(&lints), ["format", "clippy"]);
        assert!(tests.iter().all(|s| s.target.is_none()));
        assert_eq!(
            lane_names(&cross_steps),
            [
                "clippy, every product target",
                "clippy, every product target, no default features"
            ]
        );
        assert!(cross_steps.iter().all(|s| s.cross));
        assert_eq!(cross_steps[1].target.as_deref(), Some(customer.as_path()));

        let Plan {
            line_lanes: [_, customer_lane, lints],
            cross_steps,
        } = plan(base, None, &targets, Profiles::default());
        assert!(customer_lane.is_empty());
        assert_eq!(lane_names(&lints), ["format", "clippy"]);
        assert_eq!(lane_names(&cross_steps), ["clippy, every product target"]);
    }

    /// Product-target Clippy does not count toward the 10-second line (T-CURSOR-2): cold rebuilds
    /// of five triples took 38–40.5 s; the measured gate set is phase 1 only.
    #[test]
    fn product_target_clippy_is_apart_from_the_ten_second_line() {
        let under = ok_outcome(6.0, Some(40.5));
        assert!(under.gates_ok() && under.within_line() && under.passed());
        let rendered = under.render(Path::new("."));
        assert!(rendered.contains("under the 10-second line"));
        assert!(rendered.contains("cross and walks 40.5 s (apart from the 10-second line)"));
    }

    /// Over the 10-second line refuses even when every functional gate passed (T-CURSOR-1): a loud
    /// print alone used to exit 0 and let a ship through.
    #[test]
    fn gates_refuse_a_run_over_the_ten_second_line() {
        let under = ok_outcome(9.9, None);
        assert!(under.gates_ok() && under.within_line() && under.passed());
        assert!(under.failures().is_empty());
        assert!(under
            .render(Path::new("."))
            .contains("under the 10-second line"));

        let over = ok_outcome(11.0, Some(2.0));
        assert!(over.gates_ok() && !over.within_line() && !over.passed());
        let fails = over.failures();
        assert_eq!(fails.len(), 1);
        assert_eq!(fails[0].0, "speed");
        assert!(fails[0].1.contains("OVER the 10-second line"));
        let rendered = over.render(Path::new("."));
        let mut over = over;
        over.gates.push(Gate {
            name: "tests, no default features".into(),
            command: "cargo nextest".into(),
            seconds: 9.0,
            ok: true,
            tail: String::new(),
        });
        let said = over.over_line(Some("3.05 4.00 2.64"));
        let slowest = said
            .find("s  tests, no default features")
            .unwrap_or(usize::MAX);
        assert!(
            slowest < said.find("s  tests\n").unwrap_or(0),
            "the long pole first: {said}"
        );
        assert!(
            said.ends_with("load average at the end: 3.05 4.00 2.64"),
            "{said}"
        );
        for gate in &over.gates {
            assert!(said.contains(&format!("s  {}", gate.name)), "{said}");
        }
        assert!(rendered.contains("OVER the 10-second line"));
        assert!(rendered.contains("(refused)"));
    }

    /// A repository's nextest profile `gates` is the whole suite's, and only its, and
    /// `gates-no-default-features` the other suite's; without them the suites run as they always
    /// did.
    #[test]
    fn each_suite_takes_the_repository_s_profile_for_it() {
        let base = Path::new("/target");
        let featured = vec!["susi-commercial".to_owned()];
        let Plan {
            line_lanes: [tests, customer_lane, _],
            ..
        } = plan(
            base,
            Some(&featured),
            &[],
            Profiles {
                whole: Some("gates"),
                no_default: None,
            },
        );
        assert_eq!(tests[0].args, whole_suite_args(Some("gates")));
        assert!(customer_lane
            .iter()
            .all(|s| !s.args.iter().any(|a| a == "--profile")));
        let Plan {
            line_lanes: [tests, customer_lane, _],
            ..
        } = plan(
            base,
            Some(&featured),
            &[],
            Profiles {
                whole: None,
                no_default: Some("gates-no-default-features"),
            },
        );
        assert_eq!(tests[0].args, whole_suite_args(None));
        assert_eq!(
            customer_lane[0].args,
            customer_suite_args(&featured, Some("gates-no-default-features"))
        );
        assert!(customer_lane[1..]
            .iter()
            .all(|s| !s.args.iter().any(|a| a == "--profile")));

        let dir =
            std::env::temp_dir().join(format!("susi-tools-gates-profile-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(dir.join(".config")).unwrap();
        assert!(!nextest_profile(&dir, GATES_PROFILE));
        std::fs::write(
            dir.join(".config/nextest.toml"),
            "[[profile.default.overrides]]\nfilter = \"test(a)\"\n[profile.gatesish]\n",
        )
        .unwrap();
        assert!(!nextest_profile(&dir, GATES_PROFILE));
        std::fs::write(
            dir.join(".config/nextest.toml"),
            "[profile.gates]\ndefault-filter = \"not test(a)\"\n",
        )
        .unwrap();
        assert!(nextest_profile(&dir, GATES_PROFILE));
        assert!(!nextest_profile(&dir, GATES_NO_DEFAULT_PROFILE));
        std::fs::remove_dir_all(&dir).unwrap();
    }
}

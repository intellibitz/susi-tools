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
    /// Phase 2 wall clock (product-target Clippy), reported apart; `None` when no `--target`.
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

    /// Every gate passed and only the clock went over the line: the run to measure again, warm.
    /// The line is warm time (AGENTS.md, "Speed"), and the first run after a new tip also
    /// compiles what it changed. A failed gate is never run again.
    pub fn only_over_line(&self) -> bool {
        self.gates_ok() && !self.within_line()
    }

    /// Ship and land only when every gate passed *and* phase 1 stayed inside the line (CEO,
    /// 2026-11-24: the 10-second mandate is non-negotiable; a loud print alone did not stop a
    /// ship, so over-line is a refuse). Cross Clippy time is apart (T-CURSOR-2).
    pub fn passed(&self) -> bool {
        self.gates_ok() && self.within_line()
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
                format!(
                    "OVER the {LINE_SECONDS:.0}-second line in {:.1} s; fix it before any other work",
                    self.seconds
                ),
            ));
        }
        out
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
                "; cross {cross:.1} s (apart from the {LINE_SECONDS:.0}-second line)"
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

/// Runs every gate of the Cargo checkout at `work`. Phase 1 (10-second line) is the suites and
/// native lints; phase 2 is product-target Clippy when `targets` is non-empty (apart).
pub fn run(work: &Path, targets: &[String]) -> Result<Outcome, String> {
    if !work.join("Cargo.toml").is_file() {
        return Err(format!("not a Cargo checkout: {}", work.display()));
    }
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("target"));
    let featured = featured_packages(work);
    let Plan {
        line_lanes,
        cross_steps,
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
    let cross: Vec<String> = targets
        .iter()
        .flat_map(|t| ["--target".to_owned(), t.clone()])
        .collect();
    let cross_refs: Vec<&str> = cross.iter().map(String::as_str).collect();

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
    let threads = if step.args.first().map(String::as_str) == Some("nextest") {
        let cores = std::thread::available_parallelism().map_or(1, usize::from);
        test_threads(cores, std::env::var_os("NEXTEST_TEST_THREADS").as_deref())
    } else {
        None
    };
    if let Some(threads) = &threads {
        cmd.env("NEXTEST_TEST_THREADS", threads);
    }
    let mut command = format!("cargo {}", step.args.join(" "));
    if let Some(target) = &step.target {
        command = format!("CARGO_TARGET_DIR={} {command}", target.display());
    }
    if let Some(threads) = &threads {
        command = format!("NEXTEST_TEST_THREADS={threads} {command}");
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
    fn only_a_run_whose_gates_all_passed_is_measured_again() {
        assert!(ok_outcome(23.9, None).only_over_line());
        assert!(!ok_outcome(4.0, None).only_over_line());
        let mut failed = ok_outcome(23.9, None);
        failed.gates[0].ok = false;
        assert!(!failed.only_over_line());
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
        assert!(rendered.contains("cross 40.5 s (apart from the 10-second line)"));
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

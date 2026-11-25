//! `susi-tools gates`: every gate an agent runs before a ship, side by side, inside the 10-second
//! line (the CEO, 2026-11-24: "a non negotiable mandate would be 10s. if anything goes above that,
//! it would be treated high priority and fixed immediately"). A run over the line is refused
//! (non-zero exit), same as a failed gate (T-CURSOR-1). Run one after another, Susi's gates
//! took 14.3 s; side by side, 6.6 s.
//!
//! Three lanes run at once, each in its own build folder (Cargo lets one build at a time use a
//! folder, so steps sharing one across lanes waited on each other):
//! - the whole suite (`cargo nextest run --workspace`), with the repository's nextest profile
//!   `gates` when `.config/nextest.toml` names one (`gates_profile`): its `default-filter` can
//!   leave out the tests the customer build's suite runs again, so each runs once (Susi's took
//!   15 s on a cloud VM's 4 cores, 9.4 s so);
//! - when packages have features, the suite of those packages with `--no-default-features` (the
//!   program a customer is sold, in Susi), with the profile `gates-no-default-features` when the
//!   repository names one (Susi's leaves out its own repository checks, which the whole suite
//!   runs), then their Clippy and their Clippy for every `--target`
//!   named. Only they change with it; every other package compiles the same and is already tested
//!   in the first lane. It builds in its own target folder, `<target>/no-default-features`, so the
//!   two builds never overwrite each other's binaries (they did, and a test then ran the wrong
//!   program); its lints were in the third lane until Susi's lints lane took 10.3 s after a change
//!   with a new test, over the line, beside a suite of 9.4 s;
//! - the lints: `cargo fmt --check`, Clippy, and Clippy for every `--target` named, with
//!   `SUSI_CROSS_CHECK=1` (Susi's switch for its Windows runtime's native-only build script);
//!   without default features, only for the packages that have features, as their tests (after
//!   an edit to a deep crate, Susi's gates took 9.3 s; so, 8.4).

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

/// Every gate, and how long they took together.
pub struct Outcome {
    pub gates: Vec<Gate>,
    pub seconds: f64,
    copies: String,
}

impl Outcome {
    /// Every functional gate succeeded (tests, fmt, Clippy). The wall-clock line is separate:
    /// [`Self::within_line`].
    pub fn gates_ok(&self) -> bool {
        self.gates.iter().all(|g| g.ok)
    }

    /// Wall clock at or under the 10-second line.
    pub fn within_line(&self) -> bool {
        self.seconds <= LINE_SECONDS
    }

    /// Ship and land only when every gate passed *and* the run stayed inside the line (CEO,
    /// 2026-11-24: the 10-second mandate is non-negotiable; a loud print alone did not stop a
    /// ship, so over-line is a refuse).
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
            "gates for {}: {} gate(s) side by side in {:.1} s (line {LINE_SECONDS:.1} s)\n",
            work.display(),
            self.gates.len(),
            self.seconds
        );
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
        if !self.within_line() {
            out.push_str(&format!(
                "{verdict}, in {:.1} s: OVER the {LINE_SECONDS:.0}-second line, a defect of the highest priority; fix it before any other work (refused)\n",
                self.seconds
            ));
        } else {
            out.push_str(&format!(
                "{verdict}, in {:.1} s, under the {LINE_SECONDS:.0}-second line\n",
                self.seconds
            ));
        }
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

/// Runs every gate of the Cargo checkout at `work`, three lanes at once; `targets` are the
/// product targets Clippy also checks.
pub fn run(work: &Path, targets: &[String]) -> Result<Outcome, String> {
    if !work.join("Cargo.toml").is_file() {
        return Err(format!("not a Cargo checkout: {}", work.display()));
    }
    let base = std::env::var_os("CARGO_TARGET_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|| work.join("target"));
    let featured = featured_packages(work);
    let lanes = lanes(
        &base,
        featured.as_deref(),
        targets,
        Profiles {
            whole: nextest_profile(work, GATES_PROFILE).then_some(GATES_PROFILE),
            no_default: nextest_profile(work, GATES_NO_DEFAULT_PROFILE)
                .then_some(GATES_NO_DEFAULT_PROFILE),
        },
    );
    let started = Instant::now();
    let gates: Vec<Gate> = std::thread::scope(|scope| {
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
    });
    let copies = copies::report(work);
    Ok(Outcome {
        gates,
        seconds: started.elapsed().as_secs_f64(),
        copies,
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

/// The nextest profiles of the two suites, each when the repository names it.
#[derive(Clone, Copy, Default)]
struct Profiles<'a> {
    whole: Option<&'a str>,
    no_default: Option<&'a str>,
}

/// The three lanes of steps, run side by side, each keeping to its own build folder, since Cargo
/// lets one build at a time use a folder and steps sharing one across lanes wait on each other:
/// the whole suite; the customer build's suite and lints, in `<base>/no-default-features`, its
/// suite first; and the lints of the first build. `featured` is the packages that have features
/// (`Some(&[])` when they could not be named, `None` for none); `profiles` the nextest profiles
/// of the two suites, if any.
fn lanes(
    base: &Path,
    featured: Option<&[String]>,
    targets: &[String],
    profiles: Profiles<'_>,
) -> [Vec<Step>; 3] {
    let customer = base.join("no-default-features");
    let clippy = [
        "clippy",
        "--workspace",
        "--all-targets",
        "--offline",
        "--locked",
    ];
    let with = |extra: &[&str], tail: &[&str]| -> Vec<String> {
        clippy
            .iter()
            .chain(extra)
            .chain(tail)
            .map(|a| (*a).to_owned())
            .collect()
    };
    let deny = ["--", "-D", "warnings"];
    // Without default features only the packages that have features change, so their Clippy is
    // all that differs from the first build's: `-p` each, as their tests are (`--workspace` when
    // they could not be named).
    let customer_clippy = |extra: &[&str]| -> Vec<String> {
        let mut args = vec!["clippy".to_owned()];
        match featured {
            Some(packages) if !packages.is_empty() => {
                for package in packages {
                    args.extend(["-p".to_owned(), package.clone()]);
                }
            }
            _ => args.push("--workspace".to_owned()),
        }
        args.extend(
            [
                "--all-targets",
                "--offline",
                "--locked",
                "--no-default-features",
            ]
            .iter()
            .chain(extra)
            .chain(&deny)
            .map(|a| (*a).to_owned()),
        );
        args
    };
    let cross: Vec<String> = targets
        .iter()
        .flat_map(|t| ["--target".to_owned(), t.clone()])
        .collect();
    let cross_refs: Vec<&str> = cross.iter().map(String::as_str).collect();

    let mut whole = vec!["nextest", "run", "--workspace", "--offline", "--locked"];
    if let Some(profile) = profiles.whole {
        whole.extend(["--profile", profile]);
    }
    let tests = vec![step("tests", &whole, None, false)];
    let mut customer_tests = Vec::new();
    let mut lints = vec![step("format", &["fmt", "--all", "--check"], None, false)];
    lints.push(Step {
        name: "clippy",
        args: with(&[], &deny),
        target: None,
        cross: false,
    });
    if let Some(packages) = featured {
        let mut args = vec![
            "nextest",
            "run",
            "--offline",
            "--locked",
            "--no-default-features",
        ];
        if packages.is_empty() {
            args.push("--workspace");
        }
        let mut owned: Vec<String> = args.iter().map(|a| (*a).to_owned()).collect();
        for package in packages {
            owned.extend(["-p".to_owned(), package.clone()]);
        }
        if let Some(profile) = profiles.no_default {
            owned.extend(["--profile".to_owned(), profile.to_owned()]);
        }
        customer_tests.push(Step {
            name: "tests, no default features",
            args: owned,
            target: Some(customer.clone()),
            cross: false,
        });
        customer_tests.push(Step {
            name: "clippy, no default features",
            args: customer_clippy(&[]),
            target: Some(customer.clone()),
            cross: false,
        });
    }
    if !targets.is_empty() {
        lints.push(Step {
            name: "clippy, every product target",
            args: with(&cross_refs, &deny),
            target: None,
            cross: true,
        });
        if featured.is_some() {
            customer_tests.push(Step {
                name: "clippy, every product target, no default features",
                args: customer_clippy(&cross_refs),
                target: Some(customer.clone()),
                cross: true,
            });
        }
    }

    [tests, customer_tests, lints]
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
            let tail = lines[lines.len().saturating_sub(25)..].join("\n");
            (out.status.success(), tail)
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

    #[test]
    fn each_suite_gets_twice_the_cores_unless_told() {
        assert_eq!(test_threads(4, None).as_deref(), Some("8"));
        assert_eq!(test_threads(28, None).as_deref(), Some("56"));
        assert_eq!(test_threads(0, None).as_deref(), Some("2"));
        assert_eq!(test_threads(4, Some(std::ffi::OsStr::new("3"))), None);
    }

    /// Cargo lets one build at a time use a target folder (its "file lock on build directory"),
    /// so two steps sharing one in different lanes wait on each other, and the lanes' lengths
    /// add up. In Susi, after a change with a new test, the lints lane took 10.3 s, over the
    /// 10-second line, while the suite took 9.4 s, because it also held the customer build's two
    /// lints, which build in the customer's folder. So every step building in that folder runs in
    /// the customer's lane, after its tests, and no step of another lane builds there.
    #[test]
    fn the_customer_folder_s_steps_run_in_its_own_lane() {
        let base = Path::new("/target");
        let customer = base.join("no-default-features");
        let featured = vec!["susi-commercial".to_owned()];
        let targets = vec!["x86_64-pc-windows-msvc".to_owned()];
        let [tests, customer_lane, lints] =
            lanes(base, Some(&featured), &targets, Profiles::default());
        let names = |lane: &[Step]| lane.iter().map(|s| s.name).collect::<Vec<_>>();
        assert_eq!(
            names(&customer_lane),
            [
                "tests, no default features",
                "clippy, no default features",
                "clippy, every product target, no default features"
            ]
        );
        assert!(customer_lane
            .iter()
            .all(|s| s.target.as_deref() == Some(customer.as_path())));
        for lane in [&tests, &lints] {
            assert!(
                lane.iter().all(|s| s.target.is_none()),
                "{:?} builds in the customer's folder",
                names(lane)
            );
        }
        assert_eq!(
            names(&lints),
            ["format", "clippy", "clippy, every product target"]
        );
        // Without packages that have features, there is no customer lane.
        let [_, customer_lane, lints] = lanes(base, None, &targets, Profiles::default());
        assert!(customer_lane.is_empty());
        assert_eq!(
            names(&lints),
            ["format", "clippy", "clippy, every product target"]
        );
    }

    /// Over the 10-second line refuses even when every functional gate passed (T-CURSOR-1): a loud
    /// print alone used to exit 0 and let a ship through.
    #[test]
    fn gates_refuse_a_run_over_the_ten_second_line() {
        let under = Outcome {
            gates: vec![Gate {
                name: "tests".into(),
                command: "cargo nextest".into(),
                seconds: 1.0,
                ok: true,
                tail: String::new(),
            }],
            seconds: 9.9,
            copies: String::new(),
        };
        assert!(under.gates_ok());
        assert!(under.within_line());
        assert!(under.passed());
        assert!(under.failures().is_empty());
        assert!(under
            .render(Path::new("."))
            .contains("under the 10-second line"));

        let over = Outcome {
            gates: vec![Gate {
                name: "tests".into(),
                command: "cargo nextest".into(),
                seconds: 11.0,
                ok: true,
                tail: String::new(),
            }],
            seconds: 11.0,
            copies: String::new(),
        };
        assert!(over.gates_ok());
        assert!(!over.within_line());
        assert!(!over.passed());
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
        let whole = Profiles {
            whole: Some("gates"),
            no_default: None,
        };
        let [tests, customer_lane, _] = lanes(base, Some(&featured), &[], whole);
        assert_eq!(
            tests[0].args,
            [
                "nextest",
                "run",
                "--workspace",
                "--offline",
                "--locked",
                "--profile",
                "gates"
            ]
        );
        assert!(customer_lane
            .iter()
            .all(|s| !s.args.iter().any(|a| a == "--profile")));
        let both = Profiles {
            whole: None,
            no_default: Some("gates-no-default-features"),
        };
        let [tests, customer_lane, _] = lanes(base, Some(&featured), &[], both);
        assert!(!tests[0].args.iter().any(|a| a == "--profile"));
        assert!(customer_lane[0].args.ends_with(&[
            "--profile".to_owned(),
            "gates-no-default-features".to_owned()
        ]));
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

//! `susi-tools gates`: every gate an agent runs before a ship, side by side, inside the 10-second
//! line (the CEO, 2026-11-24: "a non negotiable mandate would be 10s. if anything goes above that,
//! it would be treated high priority and fixed immediately"). Run one after another, Susi's gates
//! took 14.3 s; side by side, 6.6 s.
//!
//! Three lanes run at once:
//! - the whole suite (`cargo nextest run --workspace`);
//! - when packages have features, the suite of those packages with `--no-default-features` (the
//!   program a customer is sold, in Susi). Only they change with it; every other package compiles
//!   the same and is already tested in the first lane. It builds in its own target folder,
//!   `<target>/no-default-features`, so the two builds never overwrite each other's binaries (they
//!   did, and a test then ran the wrong program);
//! - the lints: `cargo fmt --check`, Clippy for both builds, and Clippy for every `--target` named,
//!   for both builds, with `SUSI_CROSS_CHECK=1` (Susi's switch for its Windows runtime's
//!   native-only build script); without default features, only for the packages that have
//!   features, as their tests (after an edit to a deep crate, Susi's gates took 9.3 s; so, 8.4).

use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::Instant;

use crate::speed::{featured_packages, LINE_SECONDS};

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
}

impl Outcome {
    pub fn passed(&self) -> bool {
        self.gates.iter().all(|g| g.ok)
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
        let verdict = if self.passed() {
            "every gate passed"
        } else {
            "a gate failed"
        };
        if self.seconds > LINE_SECONDS {
            out.push_str(&format!(
                "{verdict}, in {:.1} s: OVER the {LINE_SECONDS:.0}-second line, a defect of the highest priority; fix it before any other work\n",
                self.seconds
            ));
        } else {
            out.push_str(&format!(
                "{verdict}, in {:.1} s, under the {LINE_SECONDS:.0}-second line\n",
                self.seconds
            ));
        }
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
    let customer = base.join("no-default-features");
    let featured = featured_packages(work);
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
        match &featured {
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

    let tests = vec![step(
        "tests",
        &["nextest", "run", "--workspace", "--offline", "--locked"],
        None,
        false,
    )];
    let mut customer_tests = Vec::new();
    let mut lints = vec![step("format", &["fmt", "--all", "--check"], None, false)];
    lints.push(Step {
        name: "clippy",
        args: with(&[], &deny),
        target: None,
        cross: false,
    });
    if let Some(packages) = &featured {
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
        customer_tests.push(Step {
            name: "tests, no default features",
            args: owned,
            target: Some(customer.clone()),
            cross: false,
        });
        lints.push(Step {
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
            lints.push(Step {
                name: "clippy, every product target, no default features",
                args: customer_clippy(&cross_refs),
                target: Some(customer.clone()),
                cross: true,
            });
        }
    }

    let started = Instant::now();
    let lanes = [tests, customer_tests, lints];
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
    Ok(Outcome {
        gates,
        seconds: started.elapsed().as_secs_f64(),
    })
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
    let mut command = format!("cargo {}", step.args.join(" "));
    if let Some(target) = &step.target {
        command = format!("CARGO_TARGET_DIR={} {command}", target.display());
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

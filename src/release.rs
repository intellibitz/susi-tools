// SPDX-License-Identifier: Apache-2.0
//! Susi Core releases: `release check`, `release qualify` and `release publish`.
//!
//! A Core release is an annotated tag `vX.Y.Z` on a commit of `main` and two committed records:
//! `releases/<tag>.json`, which the installer reads to rebuild the tag, and
//! `releases/qualification/<tag>.json`, every gate that passed on the exact tree. Their formats
//! are Core's (`tests/release_records.rs` pins them). `qualify` runs the gates on the commit,
//! tags it and commits the records on top; nothing leaves the machine until `publish` pushes the
//! records' commit to `main` and the tag in one atomic push, so the tag and its records are
//! published together or not at all.

use crate::git::{PushOutcome, Repo, Update};
use crate::json::{quote, Json};
use crate::sha256;
use std::io::{Read, Write};
use std::net::TcpStream;
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::time::{Duration, Instant};

pub const LINUX: &str = "x86_64-unknown-linux-gnu";
const WINDOWS: &str = "x86_64-pc-windows-msvc";
const ASSET: &str = "susi-linux-x86_64";
const PROFILE: &str = "dist";
/// Thin optimisation for the reference build, as earlier releases recorded; the installer
/// rebuilds the tag with these same settings.
const BUILD_ENV: [(&str, &str); 2] = [
    ("CARGO_PROFILE_DIST_LTO", "false"),
    ("CARGO_PROFILE_DIST_CODEGEN_UNITS", "16"),
];
const BUILD: [&str; 10] = [
    "cargo",
    "build",
    "--offline",
    "--locked",
    "--profile",
    PROFILE,
    "-p",
    "susi",
    "--bin",
    "susi",
];

/// What a release of this checkout would be, once every check before the gates holds.
pub struct Plan {
    pub root: PathBuf,
    pub version: String,
    pub tag: String,
    pub commit: String,
    pub tree: String,
    pub channel: String,
}

/// One gate: a command run in the checkout, its output kept in a log whose digest is recorded.
pub struct Gate {
    pub name: &'static str,
    pub argv: Vec<String>,
    pub env: Vec<(String, String)>,
}

pub struct Passed {
    gate: Gate,
    seconds: u64,
    log_sha256: String,
}

fn words(argv: &[&str]) -> Vec<String> {
    argv.iter().map(|w| (*w).to_owned()).collect()
}

/// The gates, in order. The smoke run of the built program follows them.
pub fn gates() -> Vec<Gate> {
    let plain = |name, argv: &[&str]| Gate {
        name,
        argv: words(argv),
        env: Vec::new(),
    };
    vec![
        plain("format", &["cargo", "fmt", "--all", "--check"]),
        plain(
            "clippy",
            &[
                "cargo",
                "clippy",
                "--workspace",
                "--all-targets",
                "--offline",
                "--locked",
                "--",
                "-D",
                "warnings",
            ],
        ),
        plain(
            "licenses",
            &[
                "cargo",
                "deny",
                "--offline",
                "--locked",
                "check",
                "licenses",
            ],
        ),
        plain(
            "tests",
            &[
                "cargo",
                "nextest",
                "run",
                "--workspace",
                "--offline",
                "--locked",
            ],
        ),
        plain(
            "doctests",
            &[
                "cargo",
                "test",
                "--doc",
                "--workspace",
                "--offline",
                "--locked",
            ],
        ),
        Gate {
            name: "build",
            argv: words(&BUILD),
            env: BUILD_ENV
                .iter()
                .map(|(k, v)| ((*k).to_owned(), (*v).to_owned()))
                .collect(),
        },
    ]
}

fn read(path: &Path) -> Result<String, String> {
    std::fs::read_to_string(path).map_err(|e| format!("cannot read {}: {e}", path.display()))
}

/// The `version` of the `[package]` table.
pub fn package_version(manifest: &str) -> Option<String> {
    let package = manifest.split("[package]").nth(1)?;
    let package = package.split("\n[").next()?;
    package.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("version")?.trim_start();
        let rest = rest.strip_prefix('=')?.trim();
        Some(rest.trim_matches('"').to_owned())
    })
}

/// The `channel` of `rust-toolchain.toml`.
pub fn toolchain_channel(text: &str) -> Option<String> {
    text.lines().find_map(|line| {
        let rest = line.trim().strip_prefix("channel")?.trim_start();
        let rest = rest.strip_prefix('=')?.trim();
        Some(rest.trim_matches('"').to_owned())
    })
}

/// Every check that comes before the gates. Reads, and fetches `main`; changes nothing.
pub fn plan(repo: &Repo, remote: &str, tag: Option<&str>) -> Result<Plan, String> {
    let root = repo.work_tree()?.to_path_buf();
    let role = std::fs::read_to_string(root.join(".susi-repo-role")).unwrap_or_default();
    if role.trim() != "public-core" {
        return Err(
            "this is not a Susi Core checkout (its .susi-repo-role is not public-core)".into(),
        );
    }
    if !(cfg!(target_os = "linux") && cfg!(target_arch = "x86_64")) {
        return Err(
            "Susi Core releases are qualified only on a native x86_64 Linux host; \
             from Windows, run this inside WSL2"
                .into(),
        );
    }
    let version = package_version(&read(&root.join("Cargo.toml"))?)
        .ok_or("Cargo.toml has no [package] version")?;
    let expected = format!("v{version}");
    let tag = tag.map_or_else(|| expected.clone(), str::to_owned);
    if tag != expected {
        return Err(format!(
            "{tag} is not the workspace version: Cargo.toml says {version}, so this release is {expected}"
        ));
    }
    if !repo.read(&["status", "--porcelain"])?.trim().is_empty() {
        return Err("the checkout has changes: a release is cut from a clean checkout".into());
    }
    let local = format!("refs/tags/{tag}");
    if repo
        .try_read(&["rev-parse", "--verify", "--quiet", &local])
        .is_some()
    {
        return Err(format!(
            "{tag} already exists in this checkout: each version is cut once"
        ));
    }
    if !repo
        .read(&["ls-remote", "--tags", remote, &local])?
        .trim()
        .is_empty()
    {
        return Err(format!(
            "{tag} is already published on {remote}: each version is cut once; release the next one"
        ));
    }
    for rel in [
        format!("releases/{tag}.json"),
        format!("releases/qualification/{tag}.json"),
    ] {
        if root.join(&rel).exists() {
            return Err(format!("{rel} already exists"));
        }
    }
    let main = crate::tasks::fetch_main(repo, remote)?;
    let commit = repo.read(&["rev-parse", "HEAD"])?.trim().to_owned();
    let tip = repo.read(&["rev-parse", &main])?.trim().to_owned();
    if commit != tip {
        return Err(format!(
            "HEAD {} is not {remote}'s main {}: a release is cut from main itself",
            &commit[..12],
            &tip[..12]
        ));
    }
    let tree = repo.read(&["rev-parse", "HEAD^{tree}"])?.trim().to_owned();
    let channel = toolchain_channel(&read(&root.join("rust-toolchain.toml"))?)
        .ok_or("rust-toolchain.toml names no channel")?;
    Ok(Plan {
        root,
        version,
        tag,
        commit,
        tree,
        channel,
    })
}

fn target_dir(root: &Path) -> PathBuf {
    std::env::var_os("CARGO_TARGET_DIR").map_or_else(|| root.join("target"), PathBuf::from)
}

fn run_gate(root: &Path, logs: &Path, gate: Gate) -> Result<Passed, String> {
    let log_path = logs.join(format!("{}.log", gate.name));
    let log = std::fs::File::create(&log_path)
        .map_err(|e| format!("cannot write {}: {e}", log_path.display()))?;
    let errors = log
        .try_clone()
        .map_err(|e| format!("cannot write {}: {e}", log_path.display()))?;
    println!("gate {}: {}", gate.name, gate.argv.join(" "));
    let started = Instant::now();
    let status = Command::new(&gate.argv[0])
        .args(&gate.argv[1..])
        .envs(gate.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
        .current_dir(root)
        .stdin(Stdio::null())
        .stdout(log)
        .stderr(errors)
        .status()
        .map_err(|e| format!("gate {}: cannot run {}: {e}", gate.name, gate.argv[0]))?;
    let seconds = started.elapsed().as_secs();
    if !status.success() {
        return Err(format!(
            "gate {} failed (exit {}) after {seconds} s; its log is {}. Nothing was written",
            gate.name,
            status.code().unwrap_or(-1),
            log_path.display()
        ));
    }
    let (log_sha256, _) = sha256::of_file(&log_path)?;
    println!("gate {}: passed in {seconds} s", gate.name);
    Ok(Passed {
        gate,
        seconds,
        log_sha256,
    })
}

/// `(passed, failed, skipped)` from cargo-nextest's summary line.
pub fn nextest_counts(log: &str) -> Option<(u64, u64, u64)> {
    let line = log.lines().rev().find(|l| l.contains("tests run:"))?;
    let after = line.split("tests run:").nth(1)?;
    let count = |word: &str| {
        after
            .split(',')
            .find_map(|part| part.trim().strip_suffix(word)?.trim().parse::<u64>().ok())
            .unwrap_or(0)
    };
    Some((count("passed"), count("failed"), count("skipped")))
}

fn http(port: u16, request: &str) -> Result<(u16, String), String> {
    let mut stream = TcpStream::connect(("127.0.0.1", port)).map_err(|e| e.to_string())?;
    let _ = stream.set_read_timeout(Some(Duration::from_secs(5)));
    stream
        .write_all(request.as_bytes())
        .map_err(|e| e.to_string())?;
    let mut reply = String::new();
    stream
        .read_to_string(&mut reply)
        .map_err(|e| e.to_string())?;
    let code = reply
        .split_whitespace()
        .nth(1)
        .and_then(|c| c.parse().ok())
        .ok_or_else(|| format!("no HTTP status in {reply:?}"))?;
    Ok((code, reply))
}

fn post_mcp(port: u16, token: Option<&str>, body: &str) -> Result<(u16, String), String> {
    let auth = token.map_or_else(String::new, |t| format!("Authorization: Bearer {t}\r\n"));
    http(
        port,
        &format!(
            "POST /mcp HTTP/1.1\r\nHost: 127.0.0.1\r\n{auth}Content-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
            body.len()
        ),
    )
}

/// Stops the daemon a smoke run started, however the run ends.
struct Daemon<'a> {
    program: &'a Path,
    env: &'a [(String, String)],
}

impl Drop for Daemon<'_> {
    fn drop(&mut self) {
        let _ = Command::new(self.program)
            .arg("stop")
            .envs(self.env.iter().map(|(k, v)| (k.as_str(), v.as_str())))
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status();
    }
}

/// Run the built program the way its users do, from a private home: `--version`, an MCP
/// session over standard input and output, and the daemon's health, token check and stop.
/// Returns what each check saw.
pub fn smoke(program: &Path, version: &str, scratch: &Path) -> Result<Vec<String>, String> {
    let home = scratch.join("home");
    let served = scratch.join("served");
    for dir in [&home, &served] {
        std::fs::create_dir_all(dir)
            .map_err(|e| format!("cannot create {}: {e}", dir.display()))?;
    }
    std::fs::write(served.join("notes.txt"), "release smoke\n")
        .map_err(|e| format!("cannot write the smoke folder: {e}"))?;
    let offset = 30_000 + std::process::id() % 20_000;
    let env: Vec<(String, String)> = vec![
        ("HOME".into(), home.display().to_string()),
        ("SUSI_HOME".into(), home.join(".susi").display().to_string()),
        ("SUSI_PORT_OFFSET".into(), offset.to_string()),
    ];
    let susi = |args: &[&str]| {
        let mut c = Command::new(program);
        c.args(args)
            .envs(env.iter().map(|(k, v)| (k.as_str(), v.as_str())));
        c
    };
    let mut seen = Vec::new();

    let out = susi(&["--version"])
        .output()
        .map_err(|e| format!("smoke: cannot run {}: {e}", program.display()))?;
    let answer = String::from_utf8_lossy(&out.stdout).trim().to_owned();
    if answer != format!("susi {version}") {
        return Err(format!(
            "smoke: --version answered {answer:?}, not \"susi {version}\""
        ));
    }
    seen.push(format!("--version answered {answer}"));

    let mut child = susi(&["mcp", "serve", "--root"])
        .arg(&served)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| format!("smoke: cannot start mcp serve: {e}"))?;
    {
        let mut stdin = child.stdin.take().ok_or("smoke: no stdin for mcp serve")?;
        let session = concat!(
            r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-06-18"}}"#,
            "\n",
            r#"{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"read_file","arguments":{"path":"notes.txt"}}}"#,
            "\n"
        );
        stdin
            .write_all(session.as_bytes())
            .map_err(|e| format!("smoke: cannot talk to mcp serve: {e}"))?;
    }
    let out = child
        .wait_with_output()
        .map_err(|e| format!("smoke: mcp serve: {e}"))?;
    let replies = String::from_utf8_lossy(&out.stdout);
    if !replies.contains("\"serverInfo\"") || !replies.contains("release smoke") {
        return Err(format!(
            "smoke: mcp serve did not initialize and read the file: {replies}"
        ));
    }
    seen.push("mcp serve answered initialize and read_file over standard input".into());

    let started = susi(&["start", "--root"])
        .arg(&served)
        .output()
        .map_err(|e| format!("smoke: cannot run start: {e}"))?;
    let _daemon = Daemon { program, env: &env };
    if !started.status.success() {
        return Err(format!(
            "smoke: start failed: {}",
            String::from_utf8_lossy(&started.stderr).trim()
        ));
    }
    let port = u16::try_from(9090 + offset).map_err(|_| "smoke: port out of range")?;
    let health = (0..50)
        .find_map(|_| {
            let reply = http(
                port,
                "GET /health HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\n\r\n",
            )
            .ok()
            .filter(|(code, _)| *code == 200);
            if reply.is_none() {
                std::thread::sleep(Duration::from_millis(100));
            }
            reply
        })
        .ok_or_else(|| format!("smoke: the daemon never answered /health on port {port}"))?;
    if !health.1.contains(&format!("\"version\":\"{version}\"")) {
        return Err(format!(
            "smoke: /health did not report {version}: {}",
            health.1
        ));
    }
    seen.push(format!("the daemon answered /health with {version}"));
    let token = read(&home.join(".susi").join("api_token"))?;
    let list = r#"{"jsonrpc":"2.0","id":3,"method":"tools/list"}"#;
    let (code, body) = post_mcp(port, Some(token.trim()), list)?;
    if code != 200 || !body.contains("read_file") {
        return Err(format!(
            "smoke: /mcp with the token answered {code}: {body}"
        ));
    }
    let (code, _) = post_mcp(port, None, list)?;
    if code != 401 {
        return Err(format!(
            "smoke: /mcp without the token answered {code}, not 401"
        ));
    }
    seen.push("/mcp listed the tools with the token and refused without it (401)".into());
    let stopped = susi(&["stop"])
        .output()
        .map_err(|e| format!("smoke: cannot run stop: {e}"))?;
    if !stopped.status.success() {
        return Err("smoke: stop failed".into());
    }
    seen.push("stop ended the daemon".into());
    Ok(seen)
}

/// `releases/<tag>.json`, in the line layout the installer reads (one field per line, the build
/// environment on one line, each asset an object on one line).
// A record states this many separate facts; a struct built for this one call would only rename them.
#[allow(clippy::too_many_arguments)]
pub fn release_record(
    plan: &Plan,
    tagged: &str,
    asset_sha256: &str,
    tests: (u64, u64, u64),
    gate_names: &[&str],
    qualification_sha256: &str,
    tool_version: &str,
) -> String {
    let env = BUILD_ENV
        .iter()
        .map(|(k, v)| quote(&format!("{k}={v}")))
        .collect::<Vec<_>>()
        .join(", ");
    let gates = gate_names
        .iter()
        .map(|g| quote(g))
        .collect::<Vec<_>>()
        .join(", ");
    let (passed, failed, skipped) = tests;
    let v = &plan.version;
    let tag = &plan.tag;
    let commit = &plan.commit;
    let short = &commit[..12];
    let tree = &plan.tree[..12];
    let lines = [
        "{".to_owned(),
        r#"  "schema": "susi-release-record/v1","#.to_owned(),
        r#"  "product": "susi","#.to_owned(),
        format!(r#"  "version": "{v}","#),
        format!(r#"  "tag": "{tag}","#),
        format!(r#"  "commit": "{commit}","#),
        format!(r#"  "tagged": "{tagged}","#),
        r#"  "build": {"#.to_owned(),
        format!(r#"    "toolchain": "{}","#, plan.channel),
        format!(r#"    "host": "{LINUX}","#),
        format!(
            "    \"source\": {},",
            quote(&format!(
                "the build gate of susi-tools release qualify on the qualified tree {tree} (commit {short}, which the tag names)"
            ))
        ),
        format!(r#"    "profile": "{PROFILE}","#),
        r#"    "features": "","#.to_owned(),
        format!(r#"    "env": [{env}],"#),
        format!("    \"command\": {}", quote(&BUILD.join(" "))),
        "  },".to_owned(),
        r#"  "reproducible": false,"#.to_owned(),
        r#"  "assets": ["#.to_owned(),
        format!(
            r#"    {{ "file": "{ASSET}", "target": "{LINUX}", "package": "susi", "bin": "susi", "sha256": "{asset_sha256}" }}"#
        ),
        "  ],".to_owned(),
        r#"  "missing_targets": ["#.to_owned(),
        format!(
            r#"    {{ "target": "{WINDOWS}", "reason": "Susi Core releases target x86_64 Linux only (since 2026-11-18); Windows is not a Core target" }}"#
        ),
        "  ],".to_owned(),
        r#"  "checks": {"#.to_owned(),
        format!(r#"    "qualification_record": "releases/qualification/{tag}.json","#),
        format!(r#"    "release_qualify": {{ "gates": [{gates}, "smoke"], "all_exit_0": true }},"#),
        format!(
            r#"    "workspace_tests": {{ "passed": {passed}, "failed": {failed}, "skipped": {skipped} }},"#
        ),
        r#"    "locally_qualified": true,"#.to_owned(),
        format!(
            "    \"qualification_note\": {}",
            quote(&format!(
                "qualified on {LINUX} with susi-tools {tool_version} release qualify {tag} on commit {commit} (tree {}); every gate exited 0, {passed} tests passed and {failed} failed ({skipped} skipped), and the smoke run of the built program passed",
                plan.tree
            ))
        ),
        "  },".to_owned(),
        r#"  "provenance": ["#.to_owned(),
        format!(
            "    {},",
            quote(&format!(
                "commit and tagged time: the annotated tag {tag} on {short}; once published: git cat-file -p {tag}"
            ))
        ),
        format!(
            "    {}",
            quote(&format!(
                "sha256: the build gate's {PROFILE} binary, recorded in releases/qualification/{tag}.json (sha256 {qualification_sha256}) with each gate's log digest"
            ))
        ),
        "  ]".to_owned(),
        "}".to_owned(),
    ];
    let mut out = lines.join("\n");
    out.push('\n');
    out
}

fn text(s: &str) -> Json {
    Json::Str(s.to_owned())
}

fn number(n: u64) -> Json {
    Json::Number(n.to_string())
}

/// `releases/qualification/<tag>.json`: every gate, its command, time and log digest.
// A record states this many separate facts; a struct built for this one call would only rename them.
#[allow(clippy::too_many_arguments)]
pub fn qualification_record(
    plan: &Plan,
    rustc: &str,
    lock_sha256: &str,
    passed: &[Passed],
    tests: (u64, u64, u64),
    asset: (&str, u64, &str),
    smoke: &[String],
    started: i64,
    finished: i64,
    tool_version: &str,
) -> String {
    let gates = passed
        .iter()
        .map(|p| {
            Json::Object(vec![
                ("name".into(), text(p.gate.name)),
                (
                    "argv".into(),
                    Json::Array(p.gate.argv.iter().map(|a| text(a)).collect()),
                ),
                (
                    "env".into(),
                    Json::Array(
                        p.gate
                            .env
                            .iter()
                            .map(|(k, v)| text(&format!("{k}={v}")))
                            .collect(),
                    ),
                ),
                ("exit_code".into(), number(0)),
                ("duration_s".into(), number(p.seconds)),
                ("log_sha256".into(), text(&p.log_sha256)),
            ])
        })
        .collect();
    let (sha, bytes, answer) = asset;
    let record = Json::Object(vec![
        ("schema".into(), text("susi-release-qualification/v1")),
        ("product".into(), text("susi")),
        ("version".into(), text(&plan.version)),
        ("tag".into(), text(&plan.tag)),
        ("source".into(), text("snapshot")),
        ("qualified_commit".into(), text(&plan.commit)),
        ("qualified_tree".into(), text(&plan.tree)),
        ("cargo_lock_sha256".into(), text(lock_sha256)),
        (
            "toolchain".into(),
            Json::Object(vec![
                ("channel".into(), text(&plan.channel)),
                ("rustc".into(), text(rustc)),
            ]),
        ),
        ("host".into(), text(LINUX)),
        ("gates".into(), Json::Array(gates)),
        (
            "tests".into(),
            Json::Object(vec![
                ("passed".into(), number(tests.0)),
                ("failed".into(), number(tests.1)),
                ("skipped".into(), number(tests.2)),
            ]),
        ),
        (
            "assets".into(),
            Json::Array(vec![Json::Object(vec![
                ("file".into(), text("susi")),
                ("bytes".into(), number(bytes)),
                ("sha256".into(), text(sha)),
                ("version_output".into(), text(answer)),
            ])]),
        ),
        (
            "smoke".into(),
            Json::Array(smoke.iter().map(|s| text(s)).collect()),
        ),
        ("owed_platforms".into(), text("")),
        ("started_unix".into(), Json::Number(started.to_string())),
        ("finished_unix".into(), Json::Number(finished.to_string())),
        (
            "provenance".into(),
            text(&format!(
                "written by susi-tools {tool_version} release qualify from the gate logs; each gate's log digest is recorded"
            )),
        ),
    ]);
    let mut out = record.to_pretty();
    out.push('\n');
    out
}

/// What `qualify` made: the tag (not yet published) and the commit carrying its records.
pub struct Qualified {
    pub tag: String,
    pub commit: String,
    pub records_commit: String,
    pub tests: (u64, u64, u64),
    pub seconds: u64,
}

/// Run every gate and the smoke run on `main`'s commit, then tag it and commit its records.
pub fn qualify(
    repo: &Repo,
    remote: &str,
    tag: Option<&str>,
    task: &str,
    sign: bool,
) -> Result<Qualified, String> {
    let plan = plan(repo, remote, tag)?;
    let started = crate::time::now_unix();
    let clock = Instant::now();
    let work = target_dir(&plan.root)
        .join("release-qualification")
        .join(&plan.tag);
    let _ = std::fs::remove_dir_all(&work);
    std::fs::create_dir_all(&work).map_err(|e| format!("cannot create {}: {e}", work.display()))?;
    let mut passed = Vec::new();
    for gate in gates() {
        passed.push(run_gate(&plan.root, &work, gate)?);
    }
    let tests_log = read(&work.join("tests.log"))?;
    let tests = nextest_counts(&tests_log).ok_or("the tests gate printed no nextest summary")?;
    if tests.1 != 0 || tests.0 == 0 {
        return Err(format!(
            "the tests gate reports {} passed and {} failed: a release needs executed tests and no failure",
            tests.0, tests.1
        ));
    }
    let program = target_dir(&plan.root).join(PROFILE).join("susi");
    println!("smoke: {}", program.display());
    let smoke = smoke(&program, &plan.version, &work.join("smoke"))?;
    for line in &smoke {
        println!("smoke: {line}");
    }
    let (asset_sha, bytes) = sha256::of_file(&program)?;
    let (lock_sha, _) = sha256::of_file(&plan.root.join("Cargo.lock"))?;
    let rustc = Command::new("rustc")
        .arg("--version")
        .current_dir(&plan.root)
        .output()
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
        .unwrap_or_default();
    let tool = env!("CARGO_PKG_VERSION");
    let qualification = qualification_record(
        &plan,
        &rustc,
        &lock_sha,
        &passed,
        tests,
        (&asset_sha, bytes, &format!("susi {}", plan.version)),
        &smoke,
        started,
        crate::time::now_unix(),
        tool,
    );

    let message = format!(
        "Susi Core {}\n\nQualified on {LINUX} by susi-tools release qualify: every gate and the smoke run passed on commit {}.",
        plan.version, plan.commit
    );
    let flag = if sign { "-s" } else { "-a" };
    repo.run_visible(&["tag", flag, "-m", &message, &plan.tag, &plan.commit])?;
    let undo_tag = || {
        let _ = repo.read(&["tag", "-d", &plan.tag]);
    };
    let tagged = match repo.read(&[
        "for-each-ref",
        "--format=%(taggerdate:iso-strict)",
        &format!("refs/tags/{}", plan.tag),
    ]) {
        Ok(t) if !t.trim().is_empty() => t.trim().to_owned(),
        _ => {
            undo_tag();
            return Err(format!("{} has no tagger date", plan.tag));
        }
    };
    let names: Vec<&str> = passed.iter().map(|p| p.gate.name).collect();
    let record = release_record(
        &plan,
        &tagged,
        &asset_sha,
        tests,
        &names,
        &sha256::of(qualification.as_bytes()),
        tool,
    );
    let record_rel = format!("releases/{}.json", plan.tag);
    let qualification_rel = format!("releases/qualification/{}.json", plan.tag);
    let write = |rel: &str, body: &str| {
        let path = plan.root.join(rel);
        std::fs::write(&path, body).map_err(|e| format!("cannot write {}: {e}", path.display()))
    };
    let committed = write(&qualification_rel, &qualification)
        .and_then(|()| write(&record_rel, &record))
        .and_then(|()| repo.read(&["add", "--", &record_rel, &qualification_rel]).map(|_| ()))
        .and_then(|()| {
            repo.run_visible(&[
                "commit",
                "-m",
                &format!("release: {}", plan.tag),
                "-m",
                &format!(
                    "The release and qualification records of {}, which names {}: every gate and the smoke run passed on that exact commit ({} tests passed, {} failed).",
                    plan.tag,
                    &plan.commit[..12],
                    tests.0,
                    tests.1
                ),
                "-m",
                &format!("Task: {task}"),
            ])
        });
    if let Err(why) = committed {
        undo_tag();
        let _ = repo.read(&["reset", "-q", "--", &record_rel, &qualification_rel]);
        let _ = std::fs::remove_file(plan.root.join(&record_rel));
        let _ = std::fs::remove_file(plan.root.join(&qualification_rel));
        return Err(format!(
            "{why}; the tag and the records were removed again, nothing was published"
        ));
    }
    let records_commit = repo.read(&["rev-parse", "HEAD"])?.trim().to_owned();
    Ok(Qualified {
        tag: plan.tag,
        commit: plan.commit,
        records_commit,
        tests,
        seconds: clock.elapsed().as_secs(),
    })
}

/// Push the records' commit to `main` and the tag, in one atomic push guarded by `main`'s
/// current value and the tag's absence. Returns `(main before, main after, tag commit)`.
pub fn publish(repo: &Repo, remote: &str, tag: &str) -> Result<(String, String, String), String> {
    let root = repo.work_tree()?.to_path_buf();
    if !repo.read(&["status", "--porcelain"])?.trim().is_empty() {
        return Err("the checkout has changes: publish only committed work".into());
    }
    let tag_ref = format!("refs/tags/{tag}");
    let tag_object = repo
        .try_read(&["rev-parse", "--verify", "--quiet", &tag_ref])
        .map(|s| s.trim().to_owned())
        .ok_or_else(|| format!("{tag} does not exist here: run release qualify first"))?;
    let tagged = repo
        .read(&["rev-parse", &format!("{tag}^{{commit}}")])?
        .trim()
        .to_owned();
    let record = read(&root.join(format!("releases/{tag}.json")))?;
    if !record.contains(&format!("\"commit\": \"{tagged}\"")) {
        return Err(format!(
            "releases/{tag}.json does not name the commit {tag} points at ({tagged})"
        ));
    }
    if !root
        .join(format!("releases/qualification/{tag}.json"))
        .is_file()
    {
        return Err(format!("releases/qualification/{tag}.json is missing"));
    }
    let head = repo.read(&["rev-parse", "HEAD"])?.trim().to_owned();
    if repo
        .try_read(&["merge-base", "--is-ancestor", &tagged, &head])
        .is_none()
    {
        return Err(format!("HEAD does not contain {tag}'s commit"));
    }
    if !repo
        .read(&["ls-remote", "--tags", remote, &tag_ref])?
        .trim()
        .is_empty()
    {
        return Err(format!(
            "{tag} is already published on {remote}: each version is published once"
        ));
    }
    let main = crate::tasks::fetch_main(repo, remote)?;
    let tip = repo.read(&["rev-parse", &main])?.trim().to_owned();
    if repo
        .try_read(&["merge-base", "--is-ancestor", &tip, &head])
        .is_none()
    {
        return Err(format!(
            "{remote}'s main ({}) has moved on: merge it, then publish again",
            &tip[..12]
        ));
    }
    let updates = [
        Update {
            refname: "refs/heads/main".into(),
            new: Some(head.clone()),
            expect: Some(tip.clone()),
        },
        Update {
            refname: tag_ref,
            new: Some(tag_object),
            expect: None,
        },
    ];
    match repo.push_atomic(remote, &updates)? {
        PushOutcome::Done => Ok((tip, head, tagged)),
        PushOutcome::Stale => Err(format!(
            "{remote} changed meanwhile ({tag} exists there, or main moved): nothing was published"
        )),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn plan() -> Plan {
        Plan {
            root: PathBuf::from("."),
            version: "0.21.72".into(),
            tag: "v0.21.72".into(),
            commit: "0123456789abcdef0123456789abcdef01234567".into(),
            tree: "89abcdef0123456789abcdef0123456789abcdef".into(),
            channel: "1.99.0".into(),
        }
    }

    #[test]
    fn versions_come_from_the_package_table_and_the_toolchain_file() {
        let manifest = "[workspace]\nmembers = []\n\n[package]\nname = \"susi\"\nversion = \"0.21.72\"\nedition = \"2021\"\n\n[dependencies]\nversion = \"9\"\n";
        assert_eq!(package_version(manifest).as_deref(), Some("0.21.72"));
        assert_eq!(package_version("[workspace]\n"), None);
        assert_eq!(
            toolchain_channel("[toolchain]\nchannel = \"1.99.0\"\n").as_deref(),
            Some("1.99.0")
        );
    }

    #[test]
    fn nextest_summaries_give_their_counts() {
        let log = "   Compiling x\n     Summary [  13.401s] 460 tests run: 460 passed, 1 skipped\n";
        assert_eq!(nextest_counts(log), Some((460, 0, 1)));
        let failing = "     Summary [   2.000s] 10 tests run: 8 passed, 2 failed, 3 skipped\n";
        assert_eq!(nextest_counts(failing), Some((8, 2, 3)));
        assert_eq!(nextest_counts("no summary here"), None);
    }

    #[test]
    fn the_release_record_keeps_the_line_layout_the_installer_reads() {
        let sha = "a".repeat(64);
        let record = release_record(
            &plan(),
            "2026-11-22T14:00:00+05:30",
            &sha,
            (390, 0, 14),
            &["format", "tests", "build"],
            &"b".repeat(64),
            "0.2.0",
        );
        let parsed = crate::json::parse(&record).unwrap();
        assert_eq!(parsed.text("tag"), "v0.21.72");
        assert_eq!(parsed.text("commit"), plan().commit);
        for field in [
            "version",
            "tag",
            "commit",
            "toolchain",
            "profile",
            "features",
        ] {
            let prefix = format!("\"{field}\": \"");
            assert_eq!(
                record
                    .lines()
                    .filter(|l| l.trim_start().starts_with(&prefix))
                    .count(),
                1,
                "{field} on one line"
            );
        }
        let env: Vec<&str> = record
            .lines()
            .filter(|l| l.trim_start().starts_with("\"env\": ["))
            .collect();
        assert_eq!(env.len(), 1);
        assert!(env[0].contains("CARGO_PROFILE_DIST_LTO=false"));
        let asset = record
            .lines()
            .find(|l| l.contains("\"file\": \"susi-linux-x86_64\""))
            .unwrap();
        assert!(asset.contains(&format!("\"sha256\": \"{sha}\"")));
        assert!(record.contains("\"target\": \"x86_64-pc-windows-msvc\", \"reason\": \""));
        assert!(record.ends_with("}\n") && !record.contains('\r'));
    }

    #[test]
    fn the_qualification_record_names_its_tag_and_every_gate() {
        let passed = vec![Passed {
            gate: Gate {
                name: "format",
                argv: words(&["cargo", "fmt", "--all", "--check"]),
                env: Vec::new(),
            },
            seconds: 2,
            log_sha256: "c".repeat(64),
        }];
        let text = qualification_record(
            &plan(),
            "rustc 1.99.0",
            &"d".repeat(64),
            &passed,
            (390, 0, 14),
            (&"e".repeat(64), 1234, "susi 0.21.72"),
            &["--version answered susi 0.21.72".into()],
            1,
            2,
            "0.2.0",
        );
        let parsed = crate::json::parse(&text).unwrap();
        assert_eq!(parsed.text("schema"), "susi-release-qualification/v1");
        assert_eq!(parsed.text("tag"), "v0.21.72");
        assert_eq!(parsed.text("source"), "snapshot");
        assert_eq!(parsed.text("qualified_commit"), plan().commit);
        assert!(text.contains("\"log_sha256\": \"cccc"));
    }

    #[test]
    fn the_gates_are_the_contract_checks_and_the_build_the_installer_repeats() {
        let names: Vec<&str> = gates().iter().map(|g| g.name).collect();
        assert_eq!(
            names,
            ["format", "clippy", "licenses", "tests", "doctests", "build"]
        );
        let build = gates().pop().unwrap();
        assert_eq!(build.argv.join(" "), BUILD.join(" "));
        assert!(build.argv.contains(&"--profile".to_owned()));
    }
}

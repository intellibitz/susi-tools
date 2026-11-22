// SPDX-License-Identifier: Apache-2.0
//! `susi-tools hook <stage>`, what the Git hooks of Susi and Susi Core run, driven end to end
//! against made-up repositories.

use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::{Command, Output, Stdio};

#[path = "../src/git_program.rs"]
mod git_program;

const ZERO: &str = "0000000000000000000000000000000000000000";

struct Fixture {
    root: PathBuf,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("susi-tools-githook-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        std::fs::write(
            root.join("gitconfig"),
            "[user]\n\tname = fixture\n\temail = fixture@example.test\n[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = main\n[maintenance]\n\tauto = false\n[gc]\n\tauto = 0\n",
        )
        .unwrap();
        let fx = Fixture { root };
        fx.git(
            &fx.root,
            &["init", "-q", "--template=", "--bare", "origin.git"],
        );
        fx.git(
            &fx.root,
            &["clone", "-q", "--template=", "origin.git", "work"],
        );
        fx.write("first.txt", "first\n");
        fx.git(&fx.work(), &["add", "-A"]);
        fx.git(&fx.work(), &["commit", "-q", "-m", "first"]);
        fx.git(&fx.work(), &["push", "-q", "origin", "HEAD:main"]);
        fx.git(&fx.work(), &["fetch", "-q", "origin"]);
        fx
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn write(&self, rel: &str, text: &str) {
        let path = self.work().join(rel);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, text).unwrap();
    }

    fn git_out(&self, dir: &Path, args: &[&str], input: Option<&[u8]>) -> String {
        let mut child = Command::new(git_program::path())
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(input.unwrap_or_default())
            .unwrap();
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        self.git_out(dir, args, None)
    }

    /// `hook <args>` in `dir` with a clean environment: no agent, no escape hatch, no isolation
    /// settings, the fixture as home; `env` adds to it.
    fn hook(&self, dir: &Path, args: &[&str], stdin: &str, env: &[(&str, &str)]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_susi-tools"));
        cmd.current_dir(dir)
            .arg("hook")
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.root)
            .env_remove("XDG_CONFIG_HOME")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        for var in [
            "SUSI_AGENT",
            "SUSI_HOME",
            "CARGO_TARGET_DIR",
            "CARGO_BUILD_TARGET_DIR",
            "TMPDIR",
            "TEMP",
            "TMP",
            "SUSI_ALLOW_FOREIGN_ROOT",
            "SUSI_ALLOW_STALE",
            "SUSI_ALLOW_NO_REUSE",
            "SUSI_ALLOW_PRIMARY",
            "SUSI_ALLOW_MAIN",
            "SUSI_ALLOW_SHARED_HOME",
        ] {
            cmd.env_remove(var);
        }
        for (name, value) in env {
            cmd.env(name, value);
        }
        let mut child = cmd.spawn().unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(stdin.as_bytes())
            .unwrap();
        child.wait_with_output().unwrap()
    }

    fn message(&self, text: &str) -> String {
        let path = self.root.join("MSG");
        std::fs::write(&path, text).unwrap();
        path.to_string_lossy().into_owned()
    }

    /// Record a claim on `task` the way `susi-tools` keeps the remote's claims.
    fn claim(&self, task: &str, agent: &str, scopes: &[&str]) {
        let scopes: Vec<String> = scopes.iter().map(|s| format!("\"{s}\"")).collect();
        let json = format!(
            "{{\"branch\":\"main\",\"scopes\":[{}],\"task\":\"{task}\",\"agent\":\"{agent}\",\"claimed_unix\":1,\"lease_until_unix\":9999999999}}",
            scopes.join(",")
        );
        let oid = self.git_out(
            &self.work(),
            &["hash-object", "-w", "--stdin"],
            Some(json.as_bytes()),
        );
        self.git(
            &self.work(),
            &[
                "update-ref",
                &format!("refs/susi-tools/remote/claims/{task}"),
                &oid,
            ],
        );
    }
}

fn err(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn every_stage_answers_help_and_an_unknown_stage_is_a_usage_error() {
    let fx = Fixture::new("help");
    let help = fx.hook(&fx.work(), &["--help"], "", &[]);
    assert!(help.status.success(), "{}", err(&help));
    assert!(String::from_utf8_lossy(&help.stdout).contains("pre-push"));
    let unknown = fx.hook(&fx.work(), &["pre-rebase"], "", &[]);
    assert_eq!(unknown.status.code(), Some(2), "{}", err(&unknown));
}

#[test]
fn pre_commit_refuses_secrets_new_large_blobs_debug_calls_and_unequal_pointers() {
    let fx = Fixture::new("pre-commit");
    let work = fx.work();
    let stage = |rel: &str, text: &str| {
        fx.git(&work, &["reset", "-q", "--hard"]);
        fx.git(&work, &["clean", "-qfd"]);
        fx.write(rel, text);
        fx.git(&work, &["add", "-A"]);
    };

    stage("notes.txt", "just words\n");
    let plain = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(plain.status.success(), "{}", err(&plain));

    stage("config.txt", &format!("token = ghp_{}\n", "a".repeat(36)));
    let secret = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(!secret.status.success());
    assert!(
        err(&secret).contains("Secrets never enter source"),
        "{}",
        err(&secret)
    );

    stage("licence.txt", &format!("ACME-LIC-v1.{}\n", "A".repeat(40)));
    let foreign = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(foreign.status.success(), "{}", err(&foreign));
    let own = fx.hook(
        &work,
        &["pre-commit", "--token-prefix", "ACME-LIC-v1"],
        "",
        &[],
    );
    assert!(
        !own.status.success(),
        "the repository's own licence token is a secret"
    );

    stage("vendor/up/key.txt", &format!("ghp_{}\n", "a".repeat(36)));
    let vendored = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(
        vendored.status.success(),
        "vendored code stays as received: {}",
        err(&vendored)
    );

    stage("big.bin", &"x".repeat(1_048_576));
    let big = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(!big.status.success());
    assert!(err(&big).contains("1 MiB"), "{}", err(&big));

    stage("src/lib.rs", "pub fn f(x: u8) -> u8 {\n    dbg!(x)\n}\n");
    let debug = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(!debug.status.success());
    assert!(err(&debug).contains("dbg!"), "{}", err(&debug));

    fx.git(&work, &["reset", "-q", "--hard"]);
    fx.write("CLAUDE.md", "Read AGENTS.md.\n");
    fx.write("GEMINI.md", "Read the contract.\n");
    fx.git(&work, &["add", "-A"]);
    let pointers = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(!pointers.status.success());
    assert!(
        err(&pointers).contains("byte-identical"),
        "{}",
        err(&pointers)
    );
}

#[test]
fn the_worktree_rule_applies_where_a_repository_asks_for_it() {
    let fx = Fixture::new("worktrees");
    let work = fx.work();
    fx.write("a.txt", "a\n");
    fx.git(&work, &["add", "-A"]);
    let anywhere = fx.hook(&work, &["pre-commit"], "", &[]);
    assert!(anywhere.status.success(), "{}", err(&anywhere));
    let primary = fx.hook(&work, &["pre-commit", "--worktrees"], "", &[]);
    assert!(!primary.status.success());
    assert!(
        err(&primary).contains("primary checkout"),
        "{}",
        err(&primary)
    );
    let merge = fx.hook(&work, &["pre-merge-commit", "--worktrees"], "", &[]);
    assert!(!merge.status.success());
    assert!(fx
        .hook(&work, &["pre-merge-commit"], "", &[])
        .status
        .success());
}

#[test]
fn commit_msg_wants_a_task_recorded_or_claimed_here_and_keeps_to_the_claim() {
    let fx = Fixture::new("commit-msg");
    let work = fx.work();
    fx.write("src/a.rs", "pub fn a() {}\n");
    fx.write("docs/b.md", "b\n");
    fx.git(&work, &["add", "src/a.rs"]);
    let run = |text: &str, env: &[(&str, &str)]| {
        let file = fx.message(text);
        fx.hook(&work, &["commit-msg", &file], "", env)
    };

    let none = run("feat: a\n", &[]);
    assert!(!none.status.success());
    assert!(err(&none).contains("Mandate 50"), "{}", err(&none));
    let wip = run("wip\n\nTask: LOCAL-x\n", &[]);
    assert!(!wip.status.success());
    assert!(run("feat: a\n\nTask: LOCAL-core-work\n", &[])
        .status
        .success());
    let unknown = run("feat: a\n\nTask: T-A-1\n", &[]);
    assert!(!unknown.status.success());
    assert!(
        err(&unknown).contains("neither an open task"),
        "{}",
        err(&unknown)
    );
    assert!(run(
        "chore: a comment line\n# Task: nothing\n\nTask: LOCAL-y\n",
        &[]
    )
    .status
    .success());

    // Recorded here: a small task owes no reuse decision.
    fx.write(
        ".agents/tasks/T-A-1.json",
        r#"{"id":"T-A-1","size":"s","created_unix":1795000000}"#,
    );
    assert!(run("feat: a\n\nTask: T-A-1\n", &[]).status.success());

    // Claimed here, from another repository's queue: the claim is yours and covers the change.
    fx.claim("T-B-2", "alpha", &["src"]);
    let mine = run("feat: a\n\nTask: T-B-2\n", &[("SUSI_AGENT", "Alpha")]);
    assert!(mine.status.success(), "{}", err(&mine));
    let theirs = run("feat: a\n\nTask: T-B-2\n", &[("SUSI_AGENT", "beta")]);
    assert!(!theirs.status.success());
    assert!(
        err(&theirs).contains("claimed by alpha"),
        "{}",
        err(&theirs)
    );
    fx.git(&work, &["add", "docs/b.md"]);
    let outside = run("feat: a\n\nTask: T-B-2\n", &[("SUSI_AGENT", "alpha")]);
    assert!(!outside.status.success());
    assert!(err(&outside).contains("docs/b.md"), "{}", err(&outside));

    // A commit of task records alone owes nothing.
    fx.git(&work, &["reset", "-q"]);
    fx.git(&work, &["add", ".agents/tasks/T-A-1.json"]);
    assert!(run("chore(tasks): add T-A-1\n", &[]).status.success());
}

#[test]
fn pre_push_lets_through_only_main_agent_refs_release_tags_and_records() {
    let fx = Fixture::new("pre-push");
    let work = fx.work();
    let first = fx.git(&work, &["rev-parse", "HEAD"]);
    fx.write("second.txt", "second\n");
    fx.git(&work, &["add", "-A"]);
    fx.git(&work, &["commit", "-q", "-m", "second"]);
    let second = fx.git(&work, &["rev-parse", "HEAD"]);
    let blob = fx.git_out(&work, &["hash-object", "-w", "--stdin"], Some(b"{}"));
    let push = |records: &[(&str, &str, &str)]| {
        let input: String = records
            .iter()
            .map(|(local, remote_ref, remote)| format!("x {local} {remote_ref} {remote}\n"))
            .collect();
        fx.hook(&work, &["pre-push", "origin"], &input, &[])
    };
    let ok = |records: &[(&str, &str, &str)]| {
        let out = push(records);
        assert!(out.status.success(), "{records:?}: {}", err(&out));
    };
    let refused = |records: &[(&str, &str, &str)], why: &str| {
        let out = push(records);
        assert!(!out.status.success(), "{records:?} passed");
        assert!(err(&out).contains(why), "{records:?}: {}", err(&out));
    };

    ok(&[(&second, "refs/heads/main", &first)]);
    refused(&[(&first, "refs/heads/main", &second)], "no force pushes");
    refused(&[(ZERO, "refs/heads/main", &first)], "never deleted");
    refused(&[(&second, "refs/heads/feature", ZERO)], "only main");
    ok(&[(&second, "refs/agents/alpha", &first)]);
    ok(&[(ZERO, "refs/agents/alpha", &first)]);
    refused(&[(&second, "refs/agents/a/b", ZERO)], "one segment");
    ok(&[(&second, "refs/tags/v1.2.3", ZERO)]);
    refused(&[(&second, "refs/tags/v1.2.3", &first)], "created once");
    refused(&[(&second, "refs/tags/snapshot-1", ZERO)], "release tag");
    ok(&[(&blob, "refs/claims/T-A-1", ZERO)]);
    refused(&[(&second, "refs/claims/T-A-1", ZERO)], "must name a blob");
    ok(&[(&blob, "refs/board/items/7", ZERO)]);
    refused(&[(&blob, "refs/board/items/7", &blob)], "append-only");
    refused(
        &[(&blob, "refs/board/items/07", ZERO)],
        "refs/board/items/<n>",
    );
    refused(
        &[(&second, "refs/inbox/a/b", ZERO)],
        "unsupported ref namespace",
    );
}

#[test]
fn post_checkout_lists_finished_worktrees_when_a_new_one_is_added() {
    let fx = Fixture::new("post-checkout");
    let work = fx.work();
    let done = fx.root.join("wt-done");
    fx.git(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "a/done",
            done.to_str().unwrap(),
        ],
    );
    let fresh = fx.root.join("wt-fresh");
    fx.git(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "a/fresh",
            fresh.to_str().unwrap(),
        ],
    );
    let head = fx.git(&fresh, &["rev-parse", "HEAD"]);
    let added = fx.hook(&fresh, &["post-checkout", ZERO, &head, "1"], "", &[]);
    assert!(added.status.success());
    assert!(err(&added).contains("wt-done"), "{}", err(&added));
    let switched = fx.hook(&fresh, &["post-checkout", &head, &head, "1"], "", &[]);
    assert!(switched.status.success());
    assert!(
        err(&switched).is_empty(),
        "only a new worktree is advised: {}",
        err(&switched)
    );
}

#[test]
fn isolation_refuses_a_shared_or_checkout_home_and_one_home_for_two_worktrees() {
    let fx = Fixture::new("isolation");
    let work = fx.work();
    fx.write("a.txt", "a\n");
    fx.git(&work, &["add", "-A"]);
    let unset = fx.hook(&work, &["pre-commit", "--isolation"], "", &[]);
    assert!(!unset.status.success());
    assert!(
        err(&unset).contains("SUSI_HOME is unset"),
        "{}",
        err(&unset)
    );
    let inside = work.join("state");
    let inside = fx.hook(
        &work,
        &["pre-commit", "--isolation"],
        "",
        &[("SUSI_HOME", inside.to_str().unwrap())],
    );
    assert!(!inside.status.success());
    assert!(
        err(&inside).contains("inside the checkout"),
        "{}",
        err(&inside)
    );
    let shared = fx.root.join(".susi");
    let shared = fx.hook(
        &work,
        &["pre-commit", "--isolation"],
        "",
        &[("SUSI_HOME", shared.to_str().unwrap())],
    );
    assert!(!shared.status.success());
    assert!(err(&shared).contains("shared default"), "{}", err(&shared));

    let home = fx.root.join("agent-home");
    std::fs::create_dir_all(&home).unwrap();
    let own = fx.hook(
        &work,
        &["pre-commit", "--isolation"],
        "",
        &[("SUSI_HOME", home.to_str().unwrap())],
    );
    assert!(own.status.success(), "{}", err(&own));
    assert!(home.join(".agent-isolation").is_file());

    let other = fx.root.join("wt-other");
    fx.git(
        &work,
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "a/other",
            other.to_str().unwrap(),
        ],
    );
    std::fs::write(other.join("b.txt"), "b\n").unwrap();
    fx.git(&other, &["add", "-A"]);
    let second = fx.hook(
        &other,
        &["pre-commit", "--isolation"],
        "",
        &[("SUSI_HOME", home.to_str().unwrap())],
    );
    assert!(!second.status.success());
    assert!(
        err(&second).contains("was claimed by the worktree"),
        "{}",
        err(&second)
    );
}

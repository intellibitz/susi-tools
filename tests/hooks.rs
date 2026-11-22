// SPDX-License-Identifier: Apache-2.0
//! The checks the Git hooks run (`workflow root | fresh | reuse | guard`), driven end to end
//! against made-up repositories.

use std::path::{Path, PathBuf};
use std::process::{Command, Output};

#[path = "../src/git_program.rs"]
mod git_program;

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
            std::env::temp_dir().join(format!("susi-tools-hooks-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // No background `git maintenance run`: more processes for nothing.
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
        fx.commit(&fx.work(), "first");
        fx.git(&fx.work(), &["push", "-q", "origin", "HEAD:main"]);
        fx
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn git(&self, dir: &Path, args: &[&str]) {
        let out = Command::new(git_program::path())
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn commit(&self, dir: &Path, name: &str) {
        std::fs::write(dir.join(format!("{name}.txt")), name).unwrap();
        self.git(dir, &["add", "-A"]);
        self.git(dir, &["commit", "-q", "-m", name]);
    }

    /// Run the program in `dir` as `agent` (none: no agent set).
    fn run(&self, dir: &Path, agent: Option<&str>, args: &[&str]) -> Output {
        let mut cmd = Command::new(env!("CARGO_BIN_EXE_susi-tools"));
        cmd.current_dir(dir)
            .args(args)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("HOME", &self.root)
            .env_remove("SUSI_AGENT")
            .env_remove("SUSI_TOOLS_LEGACY");
        for var in [
            "SUSI_ALLOW_FOREIGN_ROOT",
            "SUSI_ALLOW_STALE",
            "SUSI_ALLOW_NO_REUSE",
            "SUSI_ALLOW_PRIMARY",
            "SUSI_ALLOW_MAIN",
        ] {
            cmd.env_remove(var);
        }
        if let Some(agent) = agent {
            cmd.env("SUSI_AGENT", agent);
        }
        cmd.output().unwrap()
    }
}

fn err(out: &Output) -> String {
    String::from_utf8_lossy(&out.stderr).into_owned()
}

#[test]
fn every_check_the_hooks_probe_answers_help() {
    let fx = Fixture::new("help");
    for verb in ["root", "fresh", "reuse", "guard"] {
        let out = fx.run(&fx.work(), None, &["workflow", verb, "--help"]);
        assert!(out.status.success(), "{verb} --help: {}", err(&out));
    }
}

#[test]
fn the_guard_refuses_a_primary_checkout_and_main_and_passes_a_worktree_branch() {
    let fx = Fixture::new("guard");
    let primary = fx.run(&fx.work(), None, &["workflow", "guard", "commit"]);
    assert!(!primary.status.success());
    assert!(
        err(&primary).contains("primary checkout"),
        "{}",
        err(&primary)
    );

    let branch = fx.root.join("wt-branch");
    fx.git(
        &fx.work(),
        &[
            "worktree",
            "add",
            "-q",
            "-b",
            "a/b",
            branch.to_str().unwrap(),
        ],
    );
    let ok = fx.run(&branch, None, &["workflow", "guard", "commit"]);
    assert!(ok.status.success(), "{}", err(&ok));

    // A worktree on main: a merge there is the remedy and passes; a plain commit does not.
    fx.git(&fx.work(), &["switch", "-q", "-c", "elsewhere"]);
    let on_main = fx.root.join("wt-main");
    fx.git(
        &fx.work(),
        &["worktree", "add", "-q", on_main.to_str().unwrap(), "main"],
    );
    let commit = fx.run(&on_main, None, &["workflow", "guard", "commit"]);
    assert!(!commit.status.success());
    assert!(
        err(&commit).contains("no direct commits on main"),
        "{}",
        err(&commit)
    );
    let merge = fx.run(&on_main, None, &["workflow", "guard", "merge"]);
    assert!(merge.status.success(), "{}", err(&merge));
}

#[test]
fn the_root_check_refuses_only_another_agents_root_or_a_damaged_marker() {
    let fx = Fixture::new("root");
    let alpha = fx.root.join("agents/alpha");
    let made = fx.run(
        &fx.root,
        None,
        &[
            "workflow",
            "root",
            "init",
            "--agent",
            "alpha",
            alpha.to_str().unwrap(),
        ],
    );
    assert!(made.status.success(), "{}", err(&made));
    let marker = std::fs::read_to_string(alpha.join(".susi-agent-root")).unwrap();
    assert!(
        marker.contains("\"schema\":\"susi-agent-root-v1\""),
        "{marker}"
    );
    assert!(marker.contains("\"agent\":\"alpha\""), "{marker}");
    let again = fx.run(
        &fx.root,
        None,
        &[
            "workflow",
            "root",
            "init",
            "--agent",
            "alpha",
            alpha.to_str().unwrap(),
        ],
    );
    assert!(
        again.status.success(),
        "marking it again changes nothing: {}",
        err(&again)
    );
    let nested = alpha.join("inner");
    let inside = fx.run(
        &fx.root,
        None,
        &[
            "workflow",
            "root",
            "init",
            "--agent",
            "beta",
            nested.to_str().unwrap(),
        ],
    );
    assert!(!inside.status.success(), "a root inside another is refused");

    let dir = alpha.to_str().unwrap();
    let own = fx.run(&fx.root, Some("Alpha"), &["workflow", "root", "show", dir]);
    assert!(own.status.success(), "{}", err(&own));
    let foreign = fx.run(&fx.root, Some("beta"), &["workflow", "root", "show", dir]);
    assert!(!foreign.status.success());
    assert!(err(&foreign).contains("Mandate 77"), "{}", err(&foreign));
    let nobody = fx.run(&fx.root, None, &["workflow", "root", "show", dir]);
    assert!(nobody.status.success(), "{}", err(&nobody));
    let outside = fx.run(&fx.root, Some("beta"), &["workflow", "root", "show"]);
    assert!(outside.status.success(), "{}", err(&outside));

    std::fs::write(alpha.join(".susi-agent-root"), "not json").unwrap();
    let damaged = fx.run(&fx.root, Some("alpha"), &["workflow", "root", "show", dir]);
    assert!(!damaged.status.success(), "a damaged marker refuses");
}

#[test]
fn reuse_refuses_a_sized_task_that_states_no_reuse_decision() {
    let fx = Fixture::new("reuse");
    let tasks = fx.work().join(".agents/tasks");
    std::fs::create_dir_all(&tasks).unwrap();
    std::fs::write(
        tasks.join("T-A-1.json"),
        r#"{"id":"T-A-1","size":"m","created_unix":1795000000}"#,
    )
    .unwrap();
    let message = fx.root.join("msg");
    std::fs::write(&message, "feat: a thing\n\nTask: T-A-1\n").unwrap();
    let msg = message.to_str().unwrap();
    let owed = fx.run(
        &fx.work(),
        None,
        &["workflow", "reuse", "--message-file", msg],
    );
    assert!(!owed.status.success());
    assert!(err(&owed).contains("Mandate 78"), "{}", err(&owed));
    let by_id = fx.run(&fx.work(), None, &["workflow", "reuse", "T-A-1"]);
    assert!(!by_id.status.success());

    std::fs::write(
        tasks.join("T-A-1.json"),
        r#"{"id":"T-A-1","size":"m","created_unix":1795000000,"reuse":{"decision":"build","searched":"here","reason":"nothing fit"}}"#,
    )
    .unwrap();
    let stated = fx.run(
        &fx.work(),
        None,
        &["workflow", "reuse", "--message-file", msg],
    );
    assert!(stated.status.success(), "{}", err(&stated));

    std::fs::write(&message, "feat: another\n\nTask: T-B-9\n").unwrap();
    let elsewhere = fx.run(
        &fx.work(),
        None,
        &["workflow", "reuse", "--message-file", msg],
    );
    assert!(
        elsewhere.status.success(),
        "no record here: nothing to check"
    );
}

#[test]
fn freshness_refuses_only_far_behind_and_never_when_the_remote_is_silent() {
    let fx = Fixture::new("fresh");
    let other = fx.root.join("other");
    fx.git(
        &fx.root,
        &["clone", "-q", "--template=", "origin.git", "other"],
    );
    for n in ["a", "b", "c"] {
        fx.commit(&other, n);
    }
    fx.git(&other, &["push", "-q", "origin", "HEAD:main"]);

    fx.git(&fx.work(), &["config", "susi.maxBehind", "2"]);
    let behind = fx.run(&fx.work(), None, &["workflow", "fresh"]);
    assert!(!behind.status.success());
    assert!(
        err(&behind).contains("3 commits behind"),
        "{}",
        err(&behind)
    );

    fx.git(&fx.work(), &["config", "susi.maxBehind", "5"]);
    let within = fx.run(&fx.work(), None, &["workflow", "fresh"]);
    assert!(within.status.success(), "{}", err(&within));

    fx.git(&fx.work(), &["config", "susi.maxBehind", "0"]);
    fx.git(
        &fx.work(),
        &[
            "remote",
            "set-url",
            "origin",
            fx.root.join("gone.git").to_str().unwrap(),
        ],
    );
    let silent = fx.run(&fx.work(), None, &["workflow", "fresh"]);
    assert!(silent.status.success(), "{}", err(&silent));
}

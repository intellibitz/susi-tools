// SPDX-License-Identifier: Apache-2.0
//! The commands that write, against made-up repositories: an `origin` bare repository and
//! clones of it, one per agent. No real Board item, claim or task record is used.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "../src/git_program.rs"]
mod git_program;

struct Fx {
    root: PathBuf,
}

impl Drop for Fx {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

fn task_json(id: &str, deps: &[&str], accept: &[&str]) -> String {
    let deps: Vec<String> = deps.iter().map(|d| format!("\"{d}\"")).collect();
    let accept: Vec<String> = accept.iter().map(|a| format!("\"{a}\"")).collect();
    format!(
        r#"{{"id":"{id}","title":"task {id}","size":"s","deps":[{}],"accept":{{"cmd":[{}]}}}}"#,
        deps.join(","),
        accept.join(",")
    )
}

fn write_gitconfig(dir: &Path) {
    std::fs::write(
        dir.join("gitconfig"),
        // No background `git maintenance run` after commits: more processes for nothing.
        "[user]\n\tname = fixture\n\temail = fixture@example.test\n[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = main\n[maintenance]\n\tauto = false\n[gc]\n\tauto = 0\n",
    )
    .unwrap();
}

fn git_in(config: &Path, dir: &Path, args: &[&str]) {
    let out = Command::new(git_program::path())
        .current_dir(dir)
        .env("GIT_CONFIG_GLOBAL", config)
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

fn copy_dir(from: &Path, to: &Path) {
    std::fs::create_dir_all(to).unwrap();
    for entry in std::fs::read_dir(from).unwrap().flatten() {
        let target = to.join(entry.file_name());
        if entry.file_type().unwrap().is_dir() {
            copy_dir(&entry.path(), &target);
        } else {
            std::fs::copy(entry.path(), &target).unwrap();
        }
    }
}

/// The made-up repositories, built with Git once and then copied for each test: `origin.git`,
/// whose main holds open tasks T-A-1, T-A-2 (waits on T-A-1) and T-A-3, and the clones `alpha`
/// and `beta`. One template serves every test process and every run: cargo-nextest runs each
/// test in its own process, and a template of its own would cost each one about ten git
/// processes. The name carries a digest of this file, so a change here
/// builds a new template; a finished one has a `.complete` marker.
fn template() -> &'static Path {
    static TEMPLATE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    TEMPLATE.get_or_init(|| {
        use std::hash::{Hash, Hasher};
        let mut digest = std::collections::hash_map::DefaultHasher::new();
        include_str!("write.rs").hash(&mut digest);
        let name = format!("susi-tools-w-template-{:016x}", digest.finish());
        let ready = std::env::temp_dir().join(&name);
        if ready.join(".complete").is_file() {
            return ready;
        }
        let root = std::env::temp_dir().join(format!("{name}-building-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        write_gitconfig(&root);
        let config = root.join("gitconfig");
        git_in(
            &config,
            &root,
            &["init", "-q", "--template=", "--bare", "origin.git"],
        );
        git_in(&config, &root, &["init", "-q", "--template=", "seed"]);
        let tasks = root.join("seed/.agents/tasks");
        std::fs::create_dir_all(&tasks).unwrap();
        std::fs::write(
            tasks.join("T-A-1.json"),
            task_json("T-A-1", &[], &["cargo", "--version"]),
        )
        .unwrap();
        std::fs::write(
            tasks.join("T-A-2.json"),
            task_json("T-A-2", &["T-A-1"], &["cargo", "--version"]),
        )
        .unwrap();
        std::fs::write(
            tasks.join("T-A-3.json"),
            task_json("T-A-3", &[], &["cargo", "no-such-subcommand-zz"]),
        )
        .unwrap();
        std::fs::write(root.join("seed/README"), "seed\n").unwrap();
        let visions = root.join("seed/.agents/visions");
        std::fs::create_dir_all(&visions).unwrap();
        std::fs::write(
            visions.join("V-1.json"),
            r#"{"schema":"susi.vision/v1","id":"V-1","statement":"build what sells","set_by":"ceo"}"#,
        )
        .unwrap();
        std::fs::write(root.join("seed/.susi-repo-role"), "private-commercial\n").unwrap();
        let seed = root.join("seed");
        git_in(&config, &seed, &["add", "-A"]);
        git_in(&config, &seed, &["commit", "-q", "-m", "seed"]);
        let origin = root.join("origin.git");
        git_in(
            &config,
            &seed,
            &[
                "push",
                "-q",
                origin.to_str().unwrap(),
                "HEAD:refs/heads/main",
            ],
        );
        for agent in ["alpha", "beta"] {
            git_in(
                &config,
                &root,
                &["clone", "-q", "--template=", "origin.git", agent],
            );
        }
        std::fs::write(root.join(".complete"), "").unwrap();
        // Another test process may have finished the same template first: then use that one.
        if std::fs::rename(&root, &ready).is_err() {
            let _ = std::fs::remove_dir_all(&root);
            assert!(
                ready.join(".complete").is_file(),
                "no template at {}",
                ready.display()
            );
        }
        ready
    })
}

impl Fx {
    /// A private copy of the template; each clone's `origin` is pointed at this copy's origin.
    fn new(tag: &str) -> Fx {
        let root = std::env::temp_dir().join(format!("susi-tools-w-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let template = template();
        for part in ["origin.git", "alpha", "beta"] {
            copy_dir(&template.join(part), &root.join(part));
        }
        write_gitconfig(&root);
        let origin = root.join("origin.git").to_string_lossy().replace('\\', "/");
        for agent in ["alpha", "beta"] {
            let config = root.join(agent).join(".git/config");
            let text = std::fs::read_to_string(&config).unwrap();
            let pointed: String = text
                .lines()
                .map(|line| {
                    if line.trim_start().starts_with("url = ") {
                        format!("\turl = {origin}")
                    } else {
                        line.to_owned()
                    }
                })
                .collect::<Vec<_>>()
                .join("\n");
            std::fs::write(&config, pointed + "\n").unwrap();
        }
        Fx { root }
    }
    fn origin(&self) -> PathBuf {
        self.root.join("origin.git")
    }

    fn clone_of(&self, agent: &str) -> PathBuf {
        self.root.join(agent)
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
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
        String::from_utf8_lossy(&out.stdout).trim().to_owned()
    }

    fn try_git(&self, dir: &Path, args: &[&str]) -> Option<String> {
        let out = Command::new(git_program::path())
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args(args)
            .output()
            .unwrap();
        out.status
            .success()
            .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
    }

    fn tool(&self, agent: &str, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_susi-tools"))
            .current_dir(self.clone_of(agent))
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("SUSI_AGENT", agent)
            .env("COMPUTERNAME", "test-host")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }

    fn ok(&self, agent: &str, args: &[&str]) -> String {
        let (code, out, err) = self.tool(agent, args);
        assert_eq!(code, 0, "{args:?} failed: {err}{out}");
        out
    }

    fn refused(&self, agent: &str, args: &[&str], why: &str) {
        let (code, out, err) = self.tool(agent, args);
        assert_eq!(code, 1, "{args:?} should be refused: {out}{err}");
        assert!(err.contains(why), "{args:?}: expected {why:?} in {err:?}");
    }

    fn origin_ref(&self, name: &str) -> Option<String> {
        self.try_git(&self.origin(), &["rev-parse", "--verify", "--quiet", name])
    }

    fn origin_blob(&self, name: &str) -> String {
        self.git(&self.origin(), &["cat-file", "blob", name])
    }
}

#[test]
fn posts_take_the_next_number_and_name_the_item_before() {
    let fx = Fx::new("post");
    fx.ok(
        "alpha",
        &["board", "post", "--kind", "note", "--title", "one"],
    );
    fx.ok(
        "beta",
        &[
            "board",
            "post",
            "--kind",
            "request",
            "--title",
            "two",
            "--to",
            "alpha",
            "--re",
            "1",
            "--task",
            "T-A-1",
            "--body",
            "line\nline",
        ],
    );
    let first = fx.origin_ref("refs/board/items/1").unwrap();
    let second = fx.origin_blob("refs/board/items/2");
    assert!(
        second.contains(&format!("\"prev\": \"{first}\"")),
        "{second}"
    );
    let order: Vec<&str> = second
        .lines()
        .filter_map(|l| l.trim().strip_prefix('"').and_then(|l| l.split('"').next()))
        .collect();
    assert_eq!(
        order,
        [
            "schema",
            "n",
            "kind",
            "agent",
            "host",
            "posted_unix",
            "title",
            "body",
            "to",
            "re",
            "task",
            "prev"
        ]
    );
    assert!(
        second.contains("\"re\": 1,") && second.contains("\"body\": \"line\\nline\""),
        "{second}"
    );
    let raw = fx.git(&fx.origin(), &["cat-file", "-p", "refs/board/items/2"]);
    assert!(
        raw.starts_with("{\n  \"schema\": \"susi-board-item-v1\","),
        "{raw}"
    );
    let listed = fx.ok("alpha", &["board", "list"]);
    assert!(listed.contains("to alpha: re 1: two"), "{listed}");
    fx.refused(
        "alpha",
        &["board", "post", "--kind", "gossip", "--title", "x"],
        "kind must be one of",
    );
}

#[test]
fn racing_posters_never_share_a_number() {
    let fx = Fx::new("race");
    let children: Vec<_> = (0..6)
        .map(|i| {
            let agent = if i % 2 == 0 { "alpha" } else { "beta" };
            Command::new(env!("CARGO_BIN_EXE_susi-tools"))
                .current_dir(fx.clone_of(agent))
                .env("GIT_CONFIG_GLOBAL", fx.root.join("gitconfig"))
                .env("GIT_CONFIG_NOSYSTEM", "1")
                .env("SUSI_AGENT", agent)
                .args([
                    "board",
                    "post",
                    "--kind",
                    "note",
                    "--title",
                    &format!("racer {i}"),
                ])
                .output_async()
        })
        .collect();
    for child in children {
        let out = child.wait_with_output().unwrap();
        assert!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
    }
    let mut titles = Vec::new();
    for n in 1..=6 {
        let item = fx.origin_blob(&format!("refs/board/items/{n}"));
        if n > 1 {
            let prev = fx
                .origin_ref(&format!("refs/board/items/{}", n - 1))
                .unwrap();
            assert!(
                item.contains(&format!("\"prev\": \"{prev}\"")),
                "item {n} does not name item {}",
                n - 1
            );
        }
        titles.push(
            item.lines()
                .find(|l| l.contains("\"title\""))
                .unwrap()
                .to_owned(),
        );
    }
    titles.sort();
    titles.dedup();
    assert_eq!(titles.len(), 6, "a post was lost or duplicated");
    assert!(fx.origin_ref("refs/board/items/7").is_none());
}

trait Spawn {
    fn output_async(&mut self) -> std::process::Child;
}

impl Spawn for Command {
    fn output_async(&mut self) -> std::process::Child {
        self.stdout(std::process::Stdio::piped())
            .stderr(std::process::Stdio::piped())
            .spawn()
            .unwrap()
    }
}

#[test]
fn a_claim_writes_its_three_refs_and_says_so_on_the_board() {
    let fx = Fx::new("claim");
    let out = fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    assert!(out.contains("ALPHA claimed T-A-1"), "{out}");
    let claim = fx.origin_blob("refs/claims/T-A-1");
    assert!(
        claim.starts_with(
            r#"{"branch":"main","scopes":["src"],"task":"T-A-1","agent":"ALPHA","claimed_unix":"#
        ),
        "{claim}"
    );
    assert_eq!(
        fx.origin_ref("refs/claim-agents/ALPHA"),
        fx.origin_ref("refs/claims/T-A-1")
    );
    assert_eq!(
        fx.origin_ref("refs/claim-generation/current"),
        fx.origin_ref("refs/claims/T-A-1")
    );
    let item = fx.origin_blob("refs/board/items/1");
    assert!(
        item.contains("\"title\": \"Claimed T-A-1\"") && item.contains("\"agent\": \"ALPHA\""),
        "{item}"
    );
    assert!(item.contains("(scopes=[src]).\""), "{item}");
}

#[test]
fn another_agents_claim_or_scope_refuses_a_claim() {
    let fx = Fx::new("others");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    fx.refused(
        "beta",
        &["tasks", "claim", "T-A-1", "--scope", "docs"],
        "is held by ALPHA",
    );
    fx.refused(
        "beta",
        &["tasks", "claim", "T-A-3", "--scope", "src/lib.rs"],
        "overlaps src, claimed by ALPHA",
    );
    fx.ok("beta", &["tasks", "claim", "T-A-3", "--scope", "docs"]);
    let (_, list, _) = fx.tool("alpha", &["tasks", "list"]);
    assert!(
        list.contains("claimed by ALPHA") && list.contains("claimed by BETA"),
        "{list}"
    );
}

#[test]
fn tasks_list_reads_the_remotes_main_and_claims_not_a_stale_local_main() {
    let fx = Fx::new("liststale");
    let alpha = fx.clone_of("alpha");
    let head = fx.git(&alpha, &["rev-parse", "HEAD"]);
    // Beta records a new task, ships it and claims it; alpha's own main stays where it was.
    let beta = fx.clone_of("beta");
    std::fs::write(
        beta.join(".agents/tasks/T-A-4.json"),
        task_json("T-A-4", &[], &["cargo", "--version"]),
    )
    .unwrap();
    fx.git(&beta, &["add", "-A"]);
    fx.git(&beta, &["commit", "-q", "-m", "add T-A-4"]);
    fx.ok("beta", &["ship"]);
    fx.ok("beta", &["tasks", "claim", "T-A-4", "--scope", "docs"]);
    let list = fx.ok("alpha", &["tasks", "list"]);
    assert!(
        list.starts_with("4 open task(s) at origin's main"),
        "{list}"
    );
    assert!(
        list.lines()
            .any(|l| l.contains("T-A-4") && l.contains("claimed by BETA")),
        "{list}"
    );
    assert_eq!(
        fx.git(&alpha, &["rev-parse", "HEAD"]),
        head,
        "listing moved the checkout"
    );
}

#[test]
fn the_queue_and_the_one_claim_rule_refuse_a_claim() {
    let fx = Fx::new("rules");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    fx.refused(
        "alpha",
        &["tasks", "claim", "T-A-3", "--scope", "docs"],
        "already holds T-A-1",
    );
    fx.refused(
        "beta",
        &["tasks", "claim", "T-A-2", "--scope", "docs"],
        "waits on T-A-1",
    );
    fx.refused(
        "beta",
        &["tasks", "claim", "T-NOPE-1", "--scope", "docs"],
        "not an open task",
    );
    fx.refused(
        "beta",
        &["tasks", "claim", "T-A-3", "--scope", "../x"],
        "normalized",
    );
}

#[test]
fn claiming_again_extends_the_lease_and_keeps_when_it_was_claimed() {
    let fx = Fx::new("renew");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    let before = fx.origin_blob("refs/claims/T-A-1");
    let out = fx.ok(
        "alpha",
        &[
            "tasks", "claim", "T-A-1", "--scope", "src", "--scope", "docs", "--hours", "2",
        ],
    );
    assert!(out.contains("ALPHA claimed T-A-1 until unix"), "{out}");
    let after = fx.origin_blob("refs/claims/T-A-1");
    let field = |s: &str, k: &str| {
        s.split(&format!("\"{k}\":"))
            .nth(1)
            .unwrap()
            .split([',', '}'])
            .next()
            .unwrap()
            .parse::<i64>()
            .unwrap()
    };
    assert_eq!(
        field(&before, "claimed_unix"),
        field(&after, "claimed_unix")
    );
    assert!(field(&after, "lease_until_unix") >= now() + 7000);
    assert!(after.contains(r#""scopes":["src","docs"]"#), "{after}");
}

#[test]
fn release_frees_the_task_for_the_next_agent() {
    let fx = Fx::new("release");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    fx.refused(
        "beta",
        &["tasks", "release", "T-A-1"],
        "claimed by ALPHA, not by BETA",
    );
    fx.ok("alpha", &["tasks", "release", "T-A-1"]);
    assert!(fx.origin_ref("refs/claims/T-A-1").is_none());
    assert!(fx.origin_ref("refs/claim-agents/ALPHA").is_none());
    let released = fx.origin_blob("refs/board/items/2");
    assert!(
        released.contains("\"kind\": \"handoff\"")
            && released.contains("\"title\": \"Released T-A-1\""),
        "{released}"
    );
    fx.ok("beta", &["tasks", "claim", "T-A-1", "--scope", "src"]);
}

#[test]
fn a_number_hidden_behind_a_merge_is_still_used() {
    // A task added and closed on main, then main merged into a branch that never had its record
    // (as an agent's worktree branch merges main before shipping) and main moved to that merge:
    // Git's default history simplification follows the merge's first parent, which never had the
    // record, so a plain `git log -- <tasks>` misses the task and its number came back.
    let fx = Fx::new("merge-hidden");
    let alpha = fx.clone_of("alpha");
    fx.git(&alpha, &["switch", "-q", "-c", "side"]);
    std::fs::write(alpha.join("side.txt"), "side\n").unwrap();
    fx.git(&alpha, &["add", "side.txt"]);
    fx.git(&alpha, &["commit", "-q", "-m", "side work"]);
    fx.git(&alpha, &["switch", "-q", "main"]);
    std::fs::write(
        alpha.join(".agents/tasks/T-ALPHA-7.json"),
        task_json("T-ALPHA-7", &[], &["cargo", "test"]),
    )
    .unwrap();
    fx.git(&alpha, &["add", "-A"]);
    fx.git(&alpha, &["commit", "-q", "-m", "add 7"]);
    fx.git(&alpha, &["rm", "-q", ".agents/tasks/T-ALPHA-7.json"]);
    fx.git(&alpha, &["commit", "-q", "-m", "close 7"]);
    fx.git(&alpha, &["switch", "-q", "side"]);
    fx.git(&alpha, &["merge", "-q", "--no-edit", "main"]);
    fx.git(&alpha, &["switch", "-q", "main"]);
    fx.git(&alpha, &["merge", "-q", "--ff-only", "side"]);
    let out = fx.ok(
        "alpha",
        &[
            "tasks",
            "start",
            "After the merge",
            "--accept",
            "cargo test",
            "--size",
            "s",
            "--scope",
            "src",
            "--vision",
            "V-1",
        ],
    );
    assert!(out.contains("added T-ALPHA-8"), "{out}");
}

#[test]
fn a_started_task_takes_a_number_never_used_before() {
    let fx = Fx::new("add");
    let alpha = fx.clone_of("alpha");
    std::fs::write(
        alpha.join(".agents/tasks/T-ALPHA-5.json"),
        task_json("T-ALPHA-5", &[], &["cargo", "test"]),
    )
    .unwrap();
    fx.git(&alpha, &["add", "-A"]);
    fx.git(&alpha, &["commit", "-q", "-m", "add 5"]);
    fx.git(&alpha, &["rm", "-q", ".agents/tasks/T-ALPHA-5.json"]);
    fx.git(&alpha, &["commit", "-q", "-m", "close 5"]);
    let out = fx.ok(
        "alpha",
        &[
            "tasks",
            "start",
            "Write the thing",
            "--accept",
            "cargo test --test thing",
            "--size",
            "s",
            "--scope",
            "src",
            "--vision",
            "V-1",
        ],
    );
    assert!(out.contains("added T-ALPHA-6"), "{out}");
    let record = std::fs::read_to_string(alpha.join(".agents/tasks/T-ALPHA-6.json")).unwrap();
    assert!(
        record.contains(
            "\"cmd\": [\n      \"cargo\",\n      \"test\",\n      \"--test\",\n      \"thing\"\n    ]"
        ),
        "{record}"
    );
    assert!(record.contains("\"created_by\": \"ALPHA\""), "{record}");
    fx.refused(
        "alpha",
        &[
            "tasks", "start", "Bad", "--accept", "rm -rf .", "--scope", "docs", "--vision", "V-1",
        ],
        "runs `cargo` or a repository script",
    );
}

#[test]
fn close_commits_only_when_the_acceptance_passes() {
    let fx = Fx::new("close");
    let alpha = fx.clone_of("alpha");
    let head = fx.git(&alpha, &["rev-parse", "HEAD"]);
    fx.refused("alpha", &["tasks", "close", "T-A-3"], "is not done");
    assert_eq!(
        fx.git(&alpha, &["rev-parse", "HEAD"]),
        head,
        "a failed close committed"
    );
    assert!(alpha.join(".agents/tasks/T-A-3.json").exists());
    let out = fx.ok("alpha", &["tasks", "close", "T-A-1"]);
    assert!(out.contains("closed T-A-1"), "{out}");
    let message = fx.git(&alpha, &["log", "-1", "--format=%B"]);
    assert!(
        message.starts_with("chore(tasks): close T-A-1") && message.contains("Task: T-A-1"),
        "{message}"
    );
    assert!(!alpha.join(".agents/tasks/T-A-1.json").exists());
    std::fs::write(alpha.join("README"), "changed\n").unwrap();
    fx.refused(
        "alpha",
        &["tasks", "close", "T-A-2"],
        "commit or set aside your changes",
    );
}

#[test]
fn start_and_finish_are_one_command_each() {
    let fx = Fx::new("startfinish");
    let alpha = fx.clone_of("alpha");
    let (code, _, err) = fx.tool(
        "alpha",
        &["tasks", "start", "No scope", "--accept", "cargo --version"],
    );
    assert!(
        code == 2 && err.contains("needs --scope"),
        "a start without a scope is a usage error: {code} {err}"
    );
    let out = fx.ok(
        "alpha",
        &[
            "tasks",
            "start",
            "Write the started thing",
            "--accept",
            "cargo --version",
            "--scope",
            "src",
            "--size",
            "s",
            "--vision",
            "V-1",
        ],
    );
    assert!(
        out.contains("added T-ALPHA-1 and committed its record"),
        "{out}"
    );
    assert!(out.contains("shipped"), "{out}");
    let head = fx.git(&alpha, &["rev-parse", "HEAD"]);
    assert_eq!(fx.origin_ref("refs/heads/main"), Some(head.clone()));
    assert_eq!(
        fx.git(&alpha, &["log", "-1", "--format=%s"]),
        "chore(tasks): add T-ALPHA-1, Write the started thing"
    );
    assert!(
        fx.origin_ref("refs/claims/T-ALPHA-1").is_some(),
        "start claims the task"
    );
    commit_work(&fx, "alpha", "src/started.rs", "T-ALPHA-1");
    let out = fx.ok("alpha", &["tasks", "finish", "T-ALPHA-1"]);
    assert!(
        out.contains("closed T-ALPHA-1") && out.contains("shipped"),
        "{out}"
    );
    assert_eq!(
        fx.origin_ref("refs/heads/main"),
        Some(fx.git(&alpha, &["rev-parse", "HEAD"]))
    );
    assert!(!alpha.join(".agents/tasks/T-ALPHA-1.json").exists());
    assert!(
        fx.origin_ref("refs/claims/T-ALPHA-1").is_none(),
        "finishing releases the claim"
    );
}

fn commit_work(fx: &Fx, agent: &str, file: &str, task: &str) {
    let dir = fx.clone_of(agent);
    let path = dir.join(file);
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(&path, format!("// {agent}\n")).unwrap();
    fx.git(&dir, &["add", "-A"]);
    fx.git(
        &dir,
        &["commit", "-q", "-m", &format!("feat: work\n\nTask: {task}")],
    );
}

#[test]
fn ship_fast_forwards_main_posts_landed_and_renews_the_claim() {
    let fx = Fx::new("ship");
    let alpha = fx.clone_of("alpha");
    let (_, nothing, _) = fx.tool("alpha", &["ship"]);
    assert!(nothing.contains("nothing to ship"), "{nothing}");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    commit_work(&fx, "alpha", "src/lib.rs", "T-A-1");
    let out = fx.ok("alpha", &["ship"]);
    assert!(
        out.contains("shipped") && out.contains("landing work renewed the claim"),
        "{out}"
    );
    assert_eq!(
        fx.origin_ref("refs/heads/main"),
        Some(fx.git(&alpha, &["rev-parse", "HEAD"]))
    );
    let landed = fx.origin_blob("refs/board/items/2");
    assert!(
        landed.contains("\"kind\": \"landed\"") && landed.contains("Tasks: T-A-1"),
        "{landed}"
    );
}

#[test]
fn ship_keeps_each_backup_current_in_the_background() {
    let fx = Fx::new("backup");
    let alpha = fx.clone_of("alpha");
    let backup = fx.root.join("backup.git");
    fx.git(
        &fx.root,
        &["init", "-q", "--template=", "--bare", "backup.git"],
    );
    fx.git(
        &alpha,
        &["config", "--add", "susi.backup", backup.to_str().unwrap()],
    );
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    commit_work(&fx, "alpha", "src/lib.rs", "T-A-1");
    let out = fx.ok("alpha", &["ship"]);
    assert!(out.contains("backing up to"), "{out}");
    let head = fx.git(&alpha, &["rev-parse", "HEAD"]);
    // The push is detached: the ship has returned; the backup catches up on its own.
    let mut caught_up = false;
    for _ in 0..100 {
        let main = fx.try_git(
            &backup,
            &["rev-parse", "--verify", "--quiet", "refs/heads/main"],
        );
        let landed = fx.try_git(
            &backup,
            &["rev-parse", "--verify", "--quiet", "refs/board/items/2"],
        );
        if main.as_deref() == Some(head.as_str()) && landed.is_some() {
            caught_up = true;
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(50));
    }
    let log = std::fs::read_to_string(alpha.join(".git/susi-backup.log")).unwrap_or_default();
    assert!(
        caught_up,
        "the backup has main and the Board's landed item: {log}"
    );
    assert!(log.contains("current"), "{log}");
    assert!(
        fx.try_git(
            &backup,
            &["rev-parse", "--verify", "--quiet", "refs/claims/T-A-1"]
        )
        .is_none(),
        "claims are leases and never copied"
    );
}

#[test]
fn shipping_a_close_releases_its_claim() {
    let fx = Fx::new("shipclose");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    fx.ok("alpha", &["tasks", "close", "T-A-1"]);
    let out = fx.ok("alpha", &["ship"]);
    assert!(
        out.contains("T-A-1 is closed on main: its claim is released"),
        "{out}"
    );
    assert!(fx.origin_ref("refs/claims/T-A-1").is_none());
}

#[test]
fn ship_refuses_uncommitted_work_and_a_main_that_moved_on() {
    let fx = Fx::new("shiprefuse");
    let alpha = fx.clone_of("alpha");
    std::fs::write(alpha.join("README"), "dirty\n").unwrap();
    fx.refused("alpha", &["ship"], "only committed work ships");
    fx.git(&alpha, &["checkout", "--", "README"]);
    commit_work(&fx, "alpha", "src/a.rs", "T-A-1");
    fx.ok("alpha", &["ship"]);
    commit_work(&fx, "beta", "src/b.rs", "T-A-3");
    fx.refused("beta", &["ship"], "main has moved on");
}

#[test]
fn the_session_check_fast_forwards_and_shows_what_is_addressed_to_you() {
    let fx = Fx::new("check");
    commit_work(&fx, "beta", "src/b.rs", "T-A-3");
    fx.ok("beta", &["ship"]);
    fx.ok(
        "alpha",
        &["board", "post", "--kind", "note", "--title", "mine"],
    );
    fx.ok(
        "beta",
        &[
            "board",
            "post",
            "--kind",
            "request",
            "--title",
            "please look",
            "--to",
            "alpha",
        ],
    );
    fx.ok(
        "beta",
        &[
            "board",
            "post",
            "--kind",
            "note",
            "--title",
            "for nobody in particular",
        ],
    );
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src/a.rs"]);
    let out = fx.ok("alpha", &["workflow", "check"]);
    assert!(out.contains("fast-forwarded 1 commit(s)"), "{out}");
    assert!(
        out.contains("to alpha: please look") && !out.contains("for nobody"),
        "{out}"
    );
    assert!(out.contains("you hold T-A-1 until unix"), "{out}");
    assert_eq!(
        fx.git(&fx.clone_of("alpha"), &["rev-parse", "HEAD"]),
        fx.origin_ref("refs/heads/main").unwrap()
    );
    fx.ok(
        "alpha",
        &[
            "board", "post", "--kind", "answer", "--title", "done", "--re", "3",
        ],
    );
    let again = fx.ok("alpha", &["board", "check"]);
    assert!(
        again.contains("nothing is waiting for your answer"),
        "{again}"
    );
}

#[test]
fn ship_prints_a_heads_up_to_you_once() {
    let fx = Fx::new("hear");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    fx.ok(
        "beta",
        &[
            "board",
            "post",
            "--kind",
            "heads-up",
            "--title",
            "look at this",
            "--to",
            "alpha",
        ],
    );
    let check = fx.ok("alpha", &["workflow", "check"]);
    assert!(
        check.contains("for you since your last item:") && check.contains("look at this"),
        "{check}"
    );
    // Shown once: the ship after the check does not show it again (T-CLAUDE-16).
    commit_work(&fx, "alpha", "src/lib.rs", "T-A-1");
    let first = fx.ok("alpha", &["ship"]);
    assert!(!first.contains("look at this"), "{first}");
    fx.ok(
        "beta",
        &[
            "board",
            "post",
            "--kind",
            "heads-up",
            "--title",
            "and at this",
            "--to",
            "alpha",
        ],
    );
    commit_work(&fx, "alpha", "src/more.rs", "T-A-1");
    let second = fx.ok("alpha", &["ship"]);
    assert!(
        second.contains("for you since your last item:") && second.contains("and at this"),
        "{second}"
    );
    commit_work(&fx, "alpha", "src/last.rs", "T-A-1");
    let third = fx.ok("alpha", &["ship"]);
    assert!(
        !third.contains("and at this") && !third.contains("for you since your last item:"),
        "{third}"
    );
}

/// Codex's sequence on 2026-11-24: a heads-up, then its own claim on the Board, then its ship.
/// The claim is the agent's own item; the heads-up still reaches the ship.
#[test]
fn a_heads_up_survives_the_agents_own_posts_before_its_ship() {
    let fx = Fx::new("hearclaim");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    fx.ok(
        "beta",
        &[
            "board",
            "post",
            "--kind",
            "heads-up",
            "--title",
            "mid-task word",
            "--to",
            "alpha",
        ],
    );
    fx.ok(
        "alpha",
        &[
            "tasks", "claim", "T-A-1", "--scope", "src", "--scope", "docs",
        ],
    );
    fx.ok(
        "alpha",
        &[
            "board",
            "post",
            "--kind",
            "decision",
            "--title",
            "a note of mine",
        ],
    );
    commit_work(&fx, "alpha", "src/lib.rs", "T-A-1");
    let shipped = fx.ok("alpha", &["ship"]);
    assert!(
        shipped.contains("for you since your last item:") && shipped.contains("mid-task word"),
        "{shipped}"
    );
    assert!(!shipped.contains("a note of mine"), "{shipped}");
}

#[test]
fn work_in_a_public_repository_posts_to_the_agent_roots_susi_board() {
    let fx = Fx::new("public");
    let root = fx.root.join("agents/alpha");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".susi-agent-root"), "{\"agent\":\"alpha\"}\n").unwrap();
    let susi = root.join("susi-commercial-apps");
    fx.git(
        &fx.root,
        &[
            "clone",
            "-q",
            "--template=",
            fx.origin().to_str().unwrap(),
            susi.to_str().unwrap(),
        ],
    );
    let core_origin = fx.root.join("core.git");
    fx.git(
        &fx.root,
        &[
            "init",
            "-q",
            "--template=",
            "--bare",
            core_origin.to_str().unwrap(),
        ],
    );
    let seed = fx.root.join("core-seed");
    fx.git(
        &fx.root,
        &["init", "-q", "--template=", seed.to_str().unwrap()],
    );
    std::fs::create_dir_all(seed.join(".agents/tasks")).unwrap();
    std::fs::write(seed.join(".susi-repo-role"), "public-core\n").unwrap();
    std::fs::write(
        seed.join(".agents/tasks/T-C-1.json"),
        task_json("T-C-1", &[], &["cargo", "--version"]),
    )
    .unwrap();
    fx.git(&seed, &["add", "-A"]);
    fx.git(&seed, &["commit", "-q", "-m", "core seed"]);
    fx.git(
        &seed,
        &[
            "push",
            "-q",
            core_origin.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    let core = root.join("core");
    fx.git(
        &fx.root,
        &[
            "clone",
            "-q",
            "--template=",
            core_origin.to_str().unwrap(),
            core.to_str().unwrap(),
        ],
    );

    let out = Command::new(env!("CARGO_BIN_EXE_susi-tools"))
        .current_dir(&core)
        .env("GIT_CONFIG_GLOBAL", fx.root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("SUSI_AGENT", "alpha")
        .args(["tasks", "claim", "T-C-1", "--scope", "src"])
        .output()
        .unwrap();
    assert!(
        out.status.success(),
        "{}",
        String::from_utf8_lossy(&out.stderr)
    );
    let claim = fx.try_git(
        &core_origin,
        &["rev-parse", "--verify", "--quiet", "refs/claims/T-C-1"],
    );
    assert!(
        claim.is_some(),
        "the claim belongs to the repository the task is in"
    );
    assert!(
        fx.try_git(
            &core_origin,
            &["rev-parse", "--verify", "--quiet", "refs/board/items/1"]
        )
        .is_none(),
        "a Board item reached the public repository"
    );
    let item = fx.origin_blob("refs/board/items/1");
    assert!(item.contains("Claimed T-C-1"), "{item}");
}

#[test]
fn a_task_of_susis_queue_claims_files_in_another_repository_and_keeps_them_across_a_ship() {
    let fx = Fx::new("crossrepo");
    let root = fx.root.join("agents/alpha");
    std::fs::create_dir_all(&root).unwrap();
    std::fs::write(root.join(".susi-agent-root"), "{\"agent\":\"alpha\"}\n").unwrap();
    let susi = root.join("susi-commercial-apps");
    fx.git(
        &fx.root,
        &[
            "clone",
            "-q",
            "--template=",
            fx.origin().to_str().unwrap(),
            susi.to_str().unwrap(),
        ],
    );
    // A repository with no queue of its own: the task is Susi's.
    let tools_origin = fx.root.join("tools.git");
    fx.git(
        &fx.root,
        &[
            "init",
            "-q",
            "--template=",
            "--bare",
            tools_origin.to_str().unwrap(),
        ],
    );
    let seed = fx.root.join("tools-seed");
    fx.git(
        &fx.root,
        &["init", "-q", "--template=", seed.to_str().unwrap()],
    );
    std::fs::write(seed.join("README.md"), "tools\n").unwrap();
    fx.git(&seed, &["add", "-A"]);
    fx.git(&seed, &["commit", "-q", "-m", "tools seed"]);
    fx.git(
        &seed,
        &[
            "push",
            "-q",
            tools_origin.to_str().unwrap(),
            "HEAD:refs/heads/main",
        ],
    );
    let tools = root.join("susi-tools");
    fx.git(
        &fx.root,
        &[
            "clone",
            "-q",
            "--template=",
            tools_origin.to_str().unwrap(),
            tools.to_str().unwrap(),
        ],
    );
    let run = |args: &[&str]| {
        Command::new(env!("CARGO_BIN_EXE_susi-tools"))
            .current_dir(&tools)
            .env("GIT_CONFIG_GLOBAL", fx.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("SUSI_AGENT", "alpha")
            .args(args)
            .output()
            .unwrap()
    };

    let claimed = run(&["tasks", "claim", "T-A-1", "--scope", "src"]);
    assert!(
        claimed.status.success(),
        "{}",
        String::from_utf8_lossy(&claimed.stderr)
    );
    let on_tools =
        |name: &str| fx.try_git(&tools_origin, &["rev-parse", "--verify", "--quiet", name]);
    assert!(
        on_tools("refs/claims/T-A-1").is_some(),
        "the claim is kept where the files it covers live"
    );
    assert!(
        fx.origin_ref("refs/claims/T-A-1").is_none(),
        "nothing is claimed in Susi's own files"
    );

    std::fs::create_dir_all(tools.join("src")).unwrap();
    std::fs::write(tools.join("src/new.rs"), "// new\n").unwrap();
    fx.git(&tools, &["add", "-A"]);
    fx.git(
        &tools,
        &["commit", "-q", "-m", "feat: x", "-m", "Task: T-A-1"],
    );
    let shipped = run(&["ship"]);
    assert!(
        shipped.status.success(),
        "{}",
        String::from_utf8_lossy(&shipped.stderr)
    );
    let said = String::from_utf8_lossy(&shipped.stdout);
    assert!(
        said.contains("landing work renewed the claim"),
        "a ship here keeps a claim whose task is open in Susi's queue: {said}"
    );
    assert!(on_tools("refs/claims/T-A-1").is_some());
}

#[test]
fn a_ship_never_shortens_a_longer_lease() {
    let fx = Fx::new("longlease");
    fx.ok(
        "alpha",
        &["tasks", "claim", "T-A-1", "--scope", "src", "--hours", "3"],
    );
    let lease = |fx: &Fx| {
        let claim = fx.origin_blob("refs/claims/T-A-1");
        claim
            .split("\"lease_until_unix\":")
            .nth(1)
            .unwrap()
            .trim_end_matches('}')
            .parse::<i64>()
            .unwrap()
    };
    let long = lease(&fx);
    commit_work(&fx, "alpha", "src/lib.rs", "T-A-1");
    let out = fx.ok("alpha", &["ship"]);
    assert!(out.contains("landing work renewed the claim"), "{out}");
    assert_eq!(
        lease(&fx),
        long,
        "the ship's one-hour renewal shortened a three-hour lease"
    );
}

// ---------------------------------------------------------------------------------------------
// Landing (T-CLAUDE-9; the CEO, 2026-11-24: "Company host lands it"): a cloud agent's clone ships
// to refs/agents/<agent>, and a company host gates it and fast-forwards main.

/// The newest Board item's text.
fn last_item(fx: &Fx) -> String {
    let refs = fx.git(
        &fx.origin(),
        &["for-each-ref", "--format=%(refname)", "refs/board/items/"],
    );
    let n = refs
        .lines()
        .filter_map(|r| r.rsplit('/').next()?.parse::<u64>().ok())
        .max()
        .unwrap();
    fx.origin_blob(&format!("refs/board/items/{n}"))
}

/// A one-crate Cargo checkout in `agent`'s clone, its test passing or not, committed for `task`.
fn commit_crate(fx: &Fx, agent: &str, passing: bool, task: &str) {
    let dir = fx.clone_of(agent);
    std::fs::write(
        dir.join("Cargo.toml"),
        "[package]\nname = \"land-probe\"\nversion = \"0.1.0\"\nedition = \"2021\"\n\n[workspace]\n",
    )
    .unwrap();
    std::fs::create_dir_all(dir.join("src")).unwrap();
    let want = if passing { 2 } else { 3 };
    std::fs::write(
        dir.join("src/lib.rs"),
        format!("#[test]\nfn adds() {{\n    assert_eq!(1 + 1, {want});\n}}\n"),
    )
    .unwrap();
    std::fs::write(dir.join(".gitignore"), "/target\n").unwrap();
    let lock = Command::new("cargo")
        .args(["generate-lockfile", "--offline"])
        .current_dir(&dir)
        .output()
        .unwrap();
    assert!(
        lock.status.success(),
        "{}",
        String::from_utf8_lossy(&lock.stderr)
    );
    fx.git(&dir, &["add", "-A"]);
    fx.git(
        &dir,
        &[
            "commit",
            "-q",
            "-m",
            &format!("feat: a crate\n\nTask: {task}"),
        ],
    );
}

/// `susi-tools land` in `agent`'s clone, building in a target folder of the fixture's own.
fn land(fx: &Fx, agent: &str) -> String {
    let out = Command::new(env!("CARGO_BIN_EXE_susi-tools"))
        .current_dir(fx.clone_of(agent))
        .env("GIT_CONFIG_GLOBAL", fx.root.join("gitconfig"))
        .env("GIT_CONFIG_NOSYSTEM", "1")
        .env("SUSI_AGENT", agent)
        .env("COMPUTERNAME", "test-host")
        .env("CARGO_TARGET_DIR", fx.root.join("land-target"))
        .args(["land"])
        .output()
        .unwrap();
    let said = format!(
        "{}{}",
        String::from_utf8_lossy(&out.stdout),
        String::from_utf8_lossy(&out.stderr)
    );
    assert!(out.status.success(), "{said}");
    said
}

#[test]
fn a_clone_that_ships_for_landing_pushes_to_its_agent_ref_and_claims_its_own_record() {
    let fx = Fx::new("forlanding");
    let alpha = fx.clone_of("alpha");
    fx.git(&alpha, &["config", "susi.land", "host"]);
    let main = fx.origin_ref("refs/heads/main");
    let out = fx.ok(
        "alpha",
        &[
            "tasks",
            "start",
            "Cloud work",
            "--accept",
            "cargo --version",
            "--scope",
            "src",
            "--vision",
            "V-1",
        ],
    );
    assert!(
        out.contains("for a company host to gate and land") && out.contains("claimed"),
        "{out}"
    );
    assert_eq!(
        fx.origin_ref("refs/heads/main"),
        main,
        "main is the host's to move"
    );
    assert_eq!(
        fx.origin_ref("refs/agents/alpha"),
        Some(fx.git(&alpha, &["rev-parse", "HEAD"]))
    );
}

#[test]
fn a_host_lands_what_passes_its_gates_and_tells_once_what_does_not() {
    let fx = Fx::new("land");
    let alpha = fx.clone_of("alpha");
    fx.git(&alpha, &["config", "susi.land", "host"]);
    commit_crate(&fx, "alpha", true, "T-A-1");
    let shipped = fx.ok("alpha", &["ship"]);
    assert!(shipped.contains("refs/agents/alpha"), "{shipped}");
    assert!(last_item(&fx).contains("\"kind\": \"request\""));

    let said = land(&fx, "beta");
    assert!(
        said.contains("landed") && said.contains("refs/agents/alpha"),
        "{said}"
    );
    let tip = fx.git(&alpha, &["rev-parse", "HEAD"]);
    assert_eq!(fx.origin_ref("refs/heads/main"), Some(tip));
    let item = last_item(&fx);
    assert!(
        item.contains("\"kind\": \"landed\"") && item.contains("from refs/agents/alpha"),
        "{item}"
    );
    assert!(land(&fx, "beta").contains("nothing to land"));

    // A failing test: not landed, the agent told once, main where it was.
    let main = fx.origin_ref("refs/heads/main");
    commit_crate(&fx, "alpha", false, "T-A-1");
    fx.ok("alpha", &["ship"]);
    let said = land(&fx, "beta");
    assert!(
        said.contains("not landed") && said.contains("tests"),
        "{said}"
    );
    assert_eq!(fx.origin_ref("refs/heads/main"), main);
    let item = last_item(&fx);
    assert!(
        item.contains("\"kind\": \"answer\"") && item.contains("\"to\": \"alpha\""),
        "{item}"
    );
    assert!(land(&fx, "beta").contains("nothing to land"), "told once");
}

#[test]
fn a_host_does_not_land_work_that_main_has_moved_past() {
    let fx = Fx::new("landbehind");
    let alpha = fx.clone_of("alpha");
    fx.git(&alpha, &["config", "susi.land", "host"]);
    commit_work(&fx, "alpha", "src/a.rs", "T-A-1");
    fx.ok("alpha", &["ship"]);
    // Meanwhile beta ships to main straight.
    commit_work(&fx, "beta", "src/b.rs", "T-A-3");
    fx.ok("beta", &["ship"]);
    let main = fx.origin_ref("refs/heads/main");
    let said = land(&fx, "beta");
    assert!(said.contains("main moved on"), "{said}");
    assert_eq!(fx.origin_ref("refs/heads/main"), main);
    assert!(last_item(&fx).contains("Not landed: main moved past"));
}

#[test]
fn a_task_names_a_vision_the_ceo_set_on_main() {
    let fx = Fx::new("vision");
    let alpha = fx.clone_of("alpha");
    let start = |vision: Option<&str>| {
        let mut args = vec![
            "tasks",
            "start",
            "Serve a vision",
            "--accept",
            "cargo --version",
            "--scope",
            "src",
            "--size",
            "s",
        ];
        if let Some(v) = vision {
            args.extend(["--vision", v]);
        }
        fx.tool("alpha", &args)
    };
    let (code, _, err) = start(None);
    assert!(
        code == 2 && err.contains("needs --vision"),
        "a task without a vision is a usage error: {code} {err}"
    );
    let (code, _, err) = start(Some("V-9"));
    assert!(
        code == 1 && err.contains("V-9 is not a vision on Susi's main"),
        "{code} {err}"
    );
    let (code, _, err) = start(Some("nine"));
    assert!(code == 1 && err.contains("--vision V-<n>"), "{code} {err}");
    // A vision committed here but not on main is not one the CEO set yet.
    std::fs::write(
        alpha.join(".agents/visions/V-2.json"),
        r#"{"id":"V-2","statement":"only local"}"#,
    )
    .unwrap();
    fx.git(&alpha, &["add", "-A"]);
    fx.git(&alpha, &["commit", "-q", "-m", "a local vision"]);
    let (code, _, err) = start(Some("V-2"));
    assert!(
        code == 1 && err.contains("V-2 is not a vision"),
        "{code} {err}"
    );
    fx.git(&alpha, &["reset", "-q", "--hard", "HEAD~1"]);

    let out = fx.ok(
        "alpha",
        &[
            "tasks",
            "start",
            "Serve a vision",
            "--accept",
            "cargo --version",
            "--scope",
            "src",
            "--size",
            "s",
            "--vision",
            "V-1",
            "--rung",
            "built",
        ],
    );
    assert!(out.contains("added T-ALPHA-1"), "{out}");
    let record = std::fs::read_to_string(alpha.join(".agents/tasks/T-ALPHA-1.json")).unwrap();
    assert!(record.contains("\"vision\": \"V-1\""), "{record}");
    assert!(record.contains("\"rung\": \"built\""), "{record}");
    let listed = fx.ok("alpha", &["tasks", "list"]);
    assert!(
        listed
            .lines()
            .any(|l| l.contains("T-ALPHA-1") && l.contains("V-1")),
        "{listed}"
    );
}

#[test]
fn the_lead_adds_a_task_and_another_agent_works_from_its_brief_alone() {
    let fx = Fx::new("brief");
    let beta = fx.clone_of("beta");
    let out = fx.ok(
        "alpha",
        &[
            "tasks",
            "add",
            "Write the page",
            "--accept",
            "cargo --version",
            "--scope",
            "src/page.rs",
            "--scope",
            "tests/page.rs",
            "--read",
            "docs/seam.md",
            "--vision",
            "V-1",
            "--for",
            "beta",
            "--goal",
            "the page the seam describes",
            "--size",
            "s",
        ],
    );
    assert!(
        out.contains("added T-ALPHA-1") && out.contains("shipped"),
        "{out}"
    );
    assert!(
        fx.origin_ref("refs/claims/T-ALPHA-1").is_none(),
        "adding claims nothing"
    );
    let record = fx.origin_blob("refs/heads/main:.agents/tasks/T-ALPHA-1.json");
    assert!(record.contains("\"for\": \"beta\""), "{record}");

    // Meant for beta: alpha's next is another task, beta's is this one, with all it needs.
    let alphas = fx.ok("alpha", &["tasks", "next"]);
    assert!(!alphas.contains("T-ALPHA-1"), "{alphas}");
    let next = fx.ok("beta", &["tasks", "next"]);
    for part in [
        "Task T-ALPHA-1: Write the page",
        "For: beta.",
        "V-1, in the CEO's words: \"build what sells\"",
        "Goal: the page the seam describes",
        "Change only: src/page.rs, tests/page.rs",
        "Read first: docs/seam.md",
        "Done when this passes: cargo --version",
        "susi-tools tasks claim T-ALPHA-1",
        "Task: T-ALPHA-1",
        "susi-tools tasks finish T-ALPHA-1",
        "in Susi Tools, `susi-tools gates` alone",
        "the folder susi-commercial-apps",
    ] {
        assert!(next.contains(part), "{part:?} missing from:\n{next}");
    }
    // The brief is the same by id, from a clone that has not pulled the record yet.
    let brief = fx.ok("beta", &["tasks", "brief", "T-ALPHA-1"]);
    assert_eq!(brief, next);

    // Claimed with no --scope: the record's paths.
    fx.git(&beta, &["pull", "-q", "--ff-only"]);
    let claimed = fx.ok("beta", &["tasks", "claim", "T-ALPHA-1"]);
    assert!(claimed.contains("claimed T-ALPHA-1"), "{claimed}");
    let claim = fx.origin_blob("refs/claims/T-ALPHA-1");
    assert!(
        claim.contains("src/page.rs") && claim.contains("tests/page.rs"),
        "{claim}"
    );
    let after = fx.ok("beta", &["tasks", "next"]);
    assert!(
        !after.contains("T-ALPHA-1"),
        "a claimed task is not next: {after}"
    );
}

/// V-49: the lead is a role any agent takes. `lead` gives its bearings from the brick map on main;
/// `lead claim` takes the lead, refused while another agent's is live unless `--take`, which tells
/// the one it took from on the Board; only the agent that leads releases it.
#[test]
fn the_lead_is_a_role_any_agent_takes() {
    let fx = Fx::new("lead");
    let alpha = fx.clone_of("alpha");
    std::fs::write(
        alpha.join(".agents/bricks.json"),
        r#"{"schema":"susi.bricks/v1","rules":[],"bricks":[{"name":"vault","title":"The vault","visions":["V-1"],"sold_as":"SKU-V","rung":"built","state":"on main","next":"ship it to a customer","tasks":["T-A-1","T-A-9"],"waits_on_ceo":"its price"}],"foundation":["T-A-3"]}"#,
    )
    .unwrap();
    std::fs::write(
        alpha.join(".agents/visions/V-2.json"),
        r#"{"id":"V-2","statement":"something new"}"#,
    )
    .unwrap();
    fx.git(&alpha, &["add", "-A"]);
    fx.git(&alpha, &["commit", "-q", "-m", "the brick map"]);
    fx.git(&alpha, &["push", "-q", "origin", "HEAD:main"]);

    let bearings = fx.ok("alpha", &["lead"]);
    for part in [
        "lead: nobody leads",
        "vault",
        "built",
        "ship it to a customer",
        "T-A-1 (open)",
        "T-A-9 (closed)",
        "waits on the CEO:",
        "vault: its price",
        "visions placed nowhere (a brick's or the rules'): V-2",
        "foundation: T-A-3 (open)",
        "open tasks no brick names:",
        "  T-A-2 ",
    ] {
        assert!(
            bearings.contains(part),
            "{part:?} missing from:\n{bearings}"
        );
    }

    let strays = bearings.split("open tasks no brick names:").nth(1).unwrap();
    assert!(
        !strays.contains("T-A-3"),
        "a foundation task is no stray:\n{bearings}"
    );

    let out = fx.ok("alpha", &["lead", "claim"]);
    assert!(out.contains("ALPHA leads until unix"), "{out}");
    assert!(fx.origin_ref("refs/lead/current").is_some());
    let claim = fx.origin_blob("refs/lead/current");
    assert!(claim.contains("\"agent\": \"ALPHA\""), "{claim}");
    let out = fx.ok("alpha", &["lead", "claim"]);
    assert!(out.contains("(renewed)"), "{out}");
    assert!(fx.ok("alpha", &["lead"]).contains("ALPHA leads until unix"));

    fx.refused("beta", &["lead", "claim"], "ALPHA leads until unix");
    fx.refused("beta", &["lead", "release"], "ALPHA leads, not BETA");
    let out = fx.ok("beta", &["lead", "claim", "--take", "--hours", "2"]);
    assert!(out.contains("BETA leads until unix"), "{out}");
    // Alpha is told at its next look.
    let told = fx.ok("alpha", &["workflow", "check"]);
    assert!(
        told.contains("Leads Susi until unix") && told.contains("to ALPHA"),
        "{told}"
    );
    let bearings = fx.ok("alpha", &["lead"]);
    assert!(bearings.contains("BETA leads until unix"), "{bearings}");
    assert!(bearings.contains("lead claim --take"), "{bearings}");

    let out = fx.ok("beta", &["lead", "release"]);
    assert!(out.contains("released the lead"), "{out}");
    assert!(fx.origin_ref("refs/lead/current").is_none());
    assert!(fx.ok("beta", &["lead"]).contains("lead: nobody leads"));
    let out = fx.ok("beta", &["lead", "release"]);
    assert!(out.contains("nobody leads"), "{out}");
}

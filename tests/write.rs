// SPDX-License-Identifier: Apache-2.0
//! The commands that write, against made-up repositories: an `origin` bare repository and
//! clones of it, one per agent. No real Board item, claim or task record is used.

use std::path::{Path, PathBuf};
use std::process::Command;

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
        "[user]\n\tname = fixture\n\temail = fixture@example.test\n[commit]\n\tgpgsign = false\n[init]\n\tdefaultBranch = main\n",
    )
    .unwrap();
}

fn git_in(config: &Path, dir: &Path, args: &[&str]) {
    let out = Command::new("git")
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

/// The made-up repositories, built once per test run with Git and then copied for each test:
/// `origin.git`, whose main holds open tasks T-A-1, T-A-2 (waits on T-A-1) and T-A-3, and the
/// clones `alpha` and `beta`.
fn template() -> &'static Path {
    static TEMPLATE: std::sync::OnceLock<PathBuf> = std::sync::OnceLock::new();
    TEMPLATE.get_or_init(|| {
        let root =
            std::env::temp_dir().join(format!("susi-tools-w-template-{}", std::process::id()));
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
        root
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
        let out = Command::new("git")
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
        let out = Command::new("git")
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
            .env_remove("SUSI_TOOLS_REPO")
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
    let listed = fx.ok(
        "alpha",
        &["board", "list", "--repo", fx.origin().to_str().unwrap()],
    );
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
    let (_, list, _) = fx.tool(
        "alpha",
        &["tasks", "list", "--repo", fx.origin().to_str().unwrap()],
    );
    assert!(
        list.contains("claimed by ALPHA") && list.contains("claimed by BETA"),
        "{list}"
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
fn renew_extends_the_lease_and_keeps_when_it_was_claimed() {
    let fx = Fx::new("renew");
    fx.ok("alpha", &["tasks", "claim", "T-A-1", "--scope", "src"]);
    let before = fx.origin_blob("refs/claims/T-A-1");
    let out = fx.ok("alpha", &["tasks", "renew", "--hours", "2"]);
    assert!(out.contains("ALPHA holds T-A-1 until unix"), "{out}");
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
fn a_new_task_takes_a_number_never_used_before() {
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
            "add",
            "Write the thing",
            "--accept",
            "cargo test --test thing",
            "--size",
            "s",
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
        &["tasks", "add", "Bad", "--accept", "rm -rf ."],
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
    fx.ok("beta", &["workflow", "ship"]);
    fx.ok(
        "alpha",
        &["board", "post", "--kind", "note", "--title", "mine"],
    );
    fx.ok(
        "beta",
        &[
            "workflow",
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
    let again = fx.ok("alpha", &["workflow", "board", "check"]);
    assert!(
        again.contains("nothing is waiting for your answer"),
        "{again}"
    );
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
        .env_remove("SUSI_BOARD_REPO")
        .env_remove("SUSI_TOOLS_REPO")
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
fn renewing_never_shortens_a_longer_lease() {
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
    fx.ok("alpha", &["tasks", "renew"]);
    assert_eq!(
        lease(&fx),
        long,
        "a one-hour renewal shortened a three-hour lease"
    );
}

#[test]
fn a_wrapper_named_by_the_setting_runs_the_acceptance() {
    let fx = Fx::new("via");
    let alpha = fx.clone_of("alpha");
    let run = |via: &str| {
        Command::new(env!("CARGO_BIN_EXE_susi-tools"))
            .current_dir(&alpha)
            .env("GIT_CONFIG_GLOBAL", fx.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env("SUSI_AGENT", "alpha")
            .env("SUSI_TOOLS_ACCEPT_VIA", via)
            .args(["tasks", "close", "T-A-1"])
            .output()
            .unwrap()
    };
    // "cargo cargo --version" fails: the wrapper is really in front of the command.
    let refused = run("cargo");
    assert_eq!(
        refused.status.code(),
        Some(1),
        "{}",
        String::from_utf8_lossy(&refused.stderr)
    );
    assert!(alpha.join(".agents/tasks/T-A-1.json").exists());
    let pass_through = if cfg!(windows) { "cmd /C" } else { "env" };
    let closed = run(pass_through);
    assert!(
        closed.status.success(),
        "{}",
        String::from_utf8_lossy(&closed.stderr)
    );
    assert!(!alpha.join(".agents/tasks/T-A-1.json").exists());
}

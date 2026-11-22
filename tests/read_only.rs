// SPDX-License-Identifier: Apache-2.0
//! The read-only slice against made-up repositories: no real Board item or task record is used
//! (Susi Tools is public; the real Board is private).

use std::path::{Path, PathBuf};
use std::process::Command;

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

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

impl Fixture {
    fn new(tag: &str) -> Fixture {
        let root =
            std::env::temp_dir().join(format!("susi-tools-test-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(&root).unwrap();
        // No background `git maintenance run` after the fixture's own fetches and commits: it
        // writes `objects/maintenance.lock` while the test snapshots the checkout (git 2.56 on
        // Linux failed `reading_writes_nothing_in_the_checkout` about 1 run in 15 that way).
        std::fs::write(
            root.join("gitconfig"),
            "[maintenance]\n\tauto = false\n[gc]\n\tauto = 0\n",
        )
        .unwrap();
        let fx = Fixture { root };
        fx.git(
            &fx.root,
            &[
                "init",
                "-q",
                "--template=",
                "--bare",
                "-b",
                "main",
                "repo.git",
            ],
        );
        fx.git(
            &fx.root,
            &["init", "-q", "--template=", "-b", "main", "work"],
        );
        fx
    }

    fn bare(&self) -> PathBuf {
        self.root.join("repo.git")
    }

    fn work(&self) -> PathBuf {
        self.root.join("work")
    }

    fn git(&self, dir: &Path, args: &[&str]) -> String {
        let out = Command::new(git_program::path())
            .current_dir(dir)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .args([
                "-c",
                "user.name=fixture",
                "-c",
                "user.email=fixture@example.test",
                "-c",
                "commit.gpgsign=false",
            ])
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

    fn blob(&self, text: &str) -> String {
        let path = self.root.join("blob.tmp");
        std::fs::write(&path, text).unwrap();
        let oid = self.git(&self.bare(), &["hash-object", "-w", path.to_str().unwrap()]);
        std::fs::remove_file(path).unwrap();
        oid
    }

    fn board_item(&self, n: u64, kind: &str, title: &str) {
        let item = format!(
            r#"{{"schema":"susi-board-item-v1","n":{n},"kind":"{kind}","agent":"tester","host":"h","posted_unix":{},"title":"{title}","body":"b","task":"","prev":""}}"#,
            1_795_325_886 + n
        );
        let oid = self.blob(&item);
        self.git(
            &self.bare(),
            &["update-ref", &format!("refs/board/items/{n}"), &oid],
        );
    }

    fn task(&self, id: &str, title: &str) {
        let dir = self.work().join(".agents/tasks");
        std::fs::create_dir_all(&dir).unwrap();
        std::fs::write(
            dir.join(format!("{id}.json")),
            format!(r#"{{"id":"{id}","title":"{title}","size":"s","deps":[],"accept":{{"cmd":["cargo","test"]}}}}"#),
        )
        .unwrap();
    }

    fn publish(&self) {
        self.git(&self.work(), &["add", "-A"]);
        self.git(&self.work(), &["commit", "-q", "-m", "fixture"]);
        let bare = self.bare();
        self.git(
            &self.work(),
            &["push", "-q", bare.to_str().unwrap(), "HEAD:refs/heads/main"],
        );
    }

    fn claim(&self, id: &str, agent: &str, lease_until: i64) {
        let claim = format!(
            r#"{{"task":"{id}","agent":"{agent}","branch":"b","claimed_unix":1,"lease_until_unix":{lease_until},"scopes":["src"]}}"#
        );
        let oid = self.blob(&claim);
        self.git(
            &self.bare(),
            &["update-ref", &format!("refs/claims/{id}"), &oid],
        );
    }

    fn run(&self, args: &[&str]) -> (i32, String, String) {
        let out = Command::new(env!("CARGO_BIN_EXE_susi-tools"))
            .current_dir(&self.root)
            .env("GIT_CONFIG_GLOBAL", self.root.join("gitconfig"))
            .env("GIT_CONFIG_NOSYSTEM", "1")
            .env_remove("SUSI_TOOLS_REPO")
            .args(args)
            .output()
            .unwrap();
        (
            out.status.code().unwrap_or(-1),
            String::from_utf8_lossy(&out.stdout).into_owned(),
            String::from_utf8_lossy(&out.stderr).into_owned(),
        )
    }
}

#[test]
fn the_board_shows_its_newest_items_oldest_first() {
    let fx = Fixture::new("board");
    for n in 1..=12 {
        fx.board_item(n, "heads-up", &format!("item number {n}"));
    }
    let bare = fx.bare();
    let (code, out, err) = fx.run(&[
        "board",
        "list",
        "--repo",
        bare.to_str().unwrap(),
        "--limit",
        "3",
    ]);
    assert_eq!(code, 0, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].contains("3 item(s) shown, last is 12"), "{out}");
    assert!(
        lines[1].contains("item number 10") && lines[3].contains("item number 12"),
        "{out}"
    );
    assert!(
        lines[1].trim_start().starts_with("10  2026-11-22 05:38"),
        "{out}"
    );
}

#[test]
fn the_board_as_json_carries_each_number_object_and_item() {
    let fx = Fixture::new("boardjson");
    fx.board_item(1, "landed", "first");
    let bare = fx.bare();
    let (code, out, err) = fx.run(&["board", "list", "--repo", bare.to_str().unwrap(), "--json"]);
    assert_eq!(code, 0, "{err}");
    assert!(out.contains("\"n\": 1"), "{out}");
    assert!(out.contains("\"sha\": \""), "{out}");
    assert!(out.contains("\"schema\": \"susi-board-item-v1\""), "{out}");
}

#[test]
fn an_item_of_another_schema_is_refused_not_shown() {
    let fx = Fixture::new("schema");
    let oid = fx.blob(r#"{"schema":"something-else","n":1}"#);
    fx.git(&fx.bare(), &["update-ref", "refs/board/items/1", &oid]);
    let bare = fx.bare();
    let (code, _, err) = fx.run(&["board", "list", "--repo", bare.to_str().unwrap()]);
    assert_eq!(code, 1);
    assert!(err.contains("not susi-board-item-v1"), "{err}");
}

#[test]
fn open_tasks_show_live_and_lapsed_claims_and_skip_finished_records() {
    let fx = Fixture::new("tasks");
    fx.task("T-A-2", "second");
    fx.task("T-A-10", "tenth");
    fx.task("T-A-3", "third");
    let done = fx.work().join(".agents/tasks/done");
    std::fs::create_dir_all(&done).unwrap();
    std::fs::write(
        done.join("T-A-1.json"),
        r#"{"id":"T-A-1","title":"finished"}"#,
    )
    .unwrap();
    fx.publish();
    fx.claim("T-A-2", "ALPHA", now() + 3600);
    fx.claim("T-A-3", "BETA", now() - 60);
    let bare = fx.bare();
    let (code, out, err) = fx.run(&["tasks", "list", "--repo", bare.to_str().unwrap()]);
    assert_eq!(code, 0, "{err}");
    let lines: Vec<&str> = out.lines().collect();
    assert!(lines[0].starts_with("3 open task(s) at main"), "{out}");
    assert!(
        lines[1].contains("T-A-2") && lines[1].contains("claimed by ALPHA"),
        "{out}"
    );
    assert!(
        lines[2].contains("T-A-3") && lines[2].contains("claim by BETA lapsed"),
        "{out}"
    );
    assert!(
        lines[3].contains("T-A-10") && lines[3].contains("unclaimed"),
        "{out}"
    );
    assert!(
        !out.contains("finished"),
        "a finished record was listed: {out}"
    );
    let (_, json, _) = fx.run(&["tasks", "list", "--repo", bare.to_str().unwrap(), "--json"]);
    assert!(
        json.contains("\"claimed_by\": \"ALPHA\"")
            && json.contains("\"lapsed_claim_by\": \"BETA\""),
        "{json}"
    );
    assert!(json.contains("\"accept\": \"cargo test\""), "{json}");
}

/// Every file under `dir` with its size and modification time.
fn snapshot(dir: &Path) -> Vec<(PathBuf, u64, std::time::SystemTime)> {
    let mut out = Vec::new();
    let mut stack = vec![dir.to_path_buf()];
    while let Some(next) = stack.pop() {
        for entry in std::fs::read_dir(&next).unwrap().flatten() {
            let meta = entry.metadata().unwrap();
            if meta.is_dir() {
                stack.push(entry.path());
            } else {
                out.push((entry.path(), meta.len(), meta.modified().unwrap()));
            }
        }
    }
    out.sort();
    out
}

#[test]
fn reading_writes_nothing_in_the_checkout() {
    let fx = Fixture::new("nowrite");
    fx.task("T-A-1", "one");
    fx.publish();
    fx.board_item(1, "landed", "first");
    let work = fx.work();
    fx.git(
        &work,
        &[
            "fetch",
            "-q",
            fx.bare().to_str().unwrap(),
            "+refs/board/*:refs/board/*",
        ],
    );
    let before = snapshot(&work);
    for args in [
        vec!["board", "list", "--repo", work.to_str().unwrap()],
        vec!["tasks", "list", "--repo", work.to_str().unwrap()],
        vec!["board", "list", "--repo", work.to_str().unwrap(), "--json"],
    ] {
        let (code, _, err) = fx.run(&args);
        assert_eq!(code, 0, "{args:?}: {err}");
    }
    let (code, _, _) = fx.run(&[
        "board",
        "list",
        "--repo",
        work.to_str().unwrap(),
        "--limit",
        "x",
    ]);
    assert_eq!(code, 2, "a bad option is a usage error");
    assert_eq!(
        before,
        snapshot(&work),
        "a read changed a file in the checkout or its .git"
    );
}

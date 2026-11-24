// SPDX-License-Identifier: Apache-2.0
//! The copied-code report's threshold and source-directory exclusions.

use std::path::{Path, PathBuf};
use std::process::Command;

#[path = "../src/gates/copies.rs"]
mod copies;
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
    fn new() -> Fixture {
        let root = std::env::temp_dir().join(format!(
            "susi-tools-copies-test-{}-{}",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("src")).unwrap();
        std::fs::create_dir_all(root.join("tests")).unwrap();
        let fixture = Fixture { root };
        fixture.git(&["init", "-q", "--template=", "-b", "main"]);

        let function = repeated_function();
        std::fs::write(fixture.root.join("src/original.rs"), &function).unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-q", "-m", "base"]);
        let base = fixture.git(&["rev-parse", "HEAD"]);

        std::fs::write(fixture.root.join("src/copied.rs"), &function).unwrap();
        std::fs::write(
            fixture.root.join("src/short.rs"),
            function.lines().take(11).collect::<Vec<_>>().join("\n") + "\n",
        )
        .unwrap();
        std::fs::write(fixture.root.join("tests/copied.rs"), &function).unwrap();
        fixture.git(&["add", "."]);
        fixture.git(&["commit", "-q", "-m", "copies"]);
        fixture.git(&["update-ref", "refs/susi-tools/remote/main", &base]);
        fixture
    }

    fn git(&self, args: &[&str]) -> String {
        let output = Command::new(git_program::path())
            .current_dir(&self.root)
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
            output.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        String::from_utf8_lossy(&output.stdout).trim().to_owned()
    }
}

fn repeated_function() -> String {
    let mut function = String::from("pub fn repeated_function() {\n");
    for number in 0..38 {
        function.push_str(&format!("    let value_{number} = {number};\n"));
    }
    function.push_str("}\n");
    function
}

#[test]
fn reports_long_source_copy_but_not_short_or_tests_copy() {
    let fixture = Fixture::new();
    let report = copies::report(Path::new(&fixture.root));
    assert!(
        report.contains("copies: src/copied.rs:1 and src/original.rs:1"),
        "{report}"
    );
    assert!(!report.contains("src/short.rs"), "{report}");
    assert!(!report.contains("tests/copied.rs"), "{report}");
}

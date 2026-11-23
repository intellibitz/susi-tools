// SPDX-License-Identifier: Apache-2.0
//! The `git` program to start, found once per run.
//!
//! On Windows the `git` that `PATH` finds first is usually Git for Windows' launcher,
//! `<install>\cmd\git.exe`, which does nothing but start the real program,
//! `<install>\<prefix>\bin\git.exe`, as a second process: about 23 ms more for every call when
//! the host is fast and about 95 ms when it is busy (Susi's `docs/speed.md`, "The host",
//! measured 2026-11-22). The real program sets up its own environment (`MSYSTEM`, and its own
//! `bin` and `usr\bin` on the path for hooks and shell aliases) and reads the same system and
//! global configuration, so starting it directly behaves the same. Anywhere else, and whenever
//! that layout is not found, this is plain `git`, found on `PATH`.
//!
//! Both programs and the tests include this file (`#[path]`), so they start the same `git`. Its
//! tests are `susi-tools`' (`src/main.rs`), run once.

use std::ffi::OsStr;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;

/// The program every `git` command of this run starts.
pub fn path() -> &'static Path {
    static GIT: OnceLock<PathBuf> = OnceLock::new();
    GIT.get_or_init(|| {
        let found = if cfg!(windows) {
            std::env::var_os("PATH").and_then(|path| real_git(&path))
        } else {
            None
        };
        found.unwrap_or_else(|| PathBuf::from("git"))
    })
}

/// The real program behind the first `git.exe` on `path` when that one is Git for Windows'
/// launcher (a `cmd` folder beside an installed `<prefix>\bin\git.exe` and its
/// `<prefix>\libexec\git-core`); `None` when the first one is anything else, or there is none.
/// Empty and relative entries are skipped, as the operating system skips them.
pub fn real_git(path: &OsStr) -> Option<PathBuf> {
    let dir = std::env::split_paths(path)
        .filter(|dir| dir.is_absolute())
        .find(|dir| dir.join("git.exe").is_file())?;
    if !dir.file_name()?.eq_ignore_ascii_case("cmd") {
        return None;
    }
    let install = dir.parent()?;
    ["ucrt64", "mingw64", "clangarm64"]
        .iter()
        .map(|prefix| install.join(prefix))
        .find(|prefix| {
            prefix.join("bin").join("git.exe").is_file()
                && prefix.join("libexec").join("git-core").is_dir()
        })
        .map(|prefix| prefix.join("bin").join("git.exe"))
}

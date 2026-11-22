// SPDX-License-Identifier: Apache-2.0
//! Where the Susi Board lives. It is kept in Susi, the private repository, and only there: a
//! public repository (Susi Core, Susi Tools) never holds Board items, and its server refuses
//! them. Claims stay in each repository's own `origin`; Board items always go to Susi's.

use crate::git::Repo;
use std::path::{Path, PathBuf};

/// The role a checkout declares in its `.susi-repo-role` file, if any.
fn role(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(".susi-repo-role"))
        .ok()
        .map(|s| s.trim().to_owned())
}

/// The repository that holds the Board, for work done in `current`:
/// 1. `SUSI_BOARD_REPO`, when set;
/// 2. `current` itself, when it is Susi (`.susi-repo-role`: `private-commercial`) or a bare
///    repository that holds Board items;
/// 3. the `susi-commercial-apps` clone in the agent root that contains `current` (the directory
///    with a `.susi-agent-root` file).
pub fn board_repo(current: &Repo, current_path: &Path) -> Result<Repo, String> {
    if let Some(path) = std::env::var_os("SUSI_BOARD_REPO") {
        return Repo::open(Path::new(&path));
    }
    match current.work_tree() {
        Ok(work) => {
            if role(work).as_deref() == Some("private-commercial") {
                return Repo::open(work);
            }
        }
        Err(_) => {
            if !current.refs(crate::board::ITEMS)?.is_empty() {
                return Repo::open(current_path);
            }
        }
    }
    let start: PathBuf = current
        .work_tree()
        .map(Path::to_path_buf)
        .unwrap_or_else(|_| current_path.to_path_buf());
    let start = start.canonicalize().unwrap_or(start);
    for dir in start.ancestors() {
        if dir.join(".susi-agent-root").is_file() {
            let clone = dir.join("susi-commercial-apps");
            if role(&clone).as_deref() == Some("private-commercial") {
                return Repo::open(&clone);
            }
            return Err(format!(
                "the agent root {} has no Susi clone in susi-commercial-apps: clone Susi there, or set SUSI_BOARD_REPO",
                dir.display()
            ));
        }
    }
    Err(
        "cannot find the Board: work inside an agent root with a Susi clone in susi-commercial-apps, or set SUSI_BOARD_REPO"
            .into(),
    )
}

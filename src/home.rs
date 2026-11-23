// SPDX-License-Identifier: Apache-2.0
//! Where the Susi Board lives. It is kept in Susi, the private repository, and only there: a
//! public repository (Susi Tools) never holds Board items, and its firewall refuses
//! them. Claims stay in each repository's own `origin`; Board items always go to Susi's.

use crate::git::Repo;
use std::path::Path;

/// The role a checkout declares in its `.susi-repo-role` file, if any.
pub fn role(dir: &Path) -> Option<String> {
    std::fs::read_to_string(dir.join(".susi-repo-role"))
        .ok()
        .map(|s| s.trim().to_owned())
}

/// The repository that holds the Board, for work done in the checkout `current`:
/// 1. `current` itself, when it is Susi (`.susi-repo-role`: `private-commercial`);
/// 2. the `susi-commercial-apps` clone in the agent root that contains `current` (the directory
///    with a `.susi-agent-root` file).
pub fn board_repo(current: &Repo) -> Result<Repo, String> {
    let work = current.work_tree()?;
    if role(work).as_deref() == Some("private-commercial") {
        return Repo::open(work);
    }
    let start = work.canonicalize().unwrap_or_else(|_| work.to_path_buf());
    for dir in start.ancestors() {
        if dir.join(".susi-agent-root").is_file() {
            let clone = dir.join("susi-commercial-apps");
            if role(&clone).as_deref() == Some("private-commercial") {
                return Repo::open(&clone);
            }
            return Err(format!(
                "the agent root {} has no Susi clone in susi-commercial-apps: clone Susi there",
                dir.display()
            ));
        }
    }
    Err(
        "cannot find the Board: work inside an agent root with a Susi clone in susi-commercial-apps"
            .into(),
    )
}

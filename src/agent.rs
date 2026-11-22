// SPDX-License-Identifier: Apache-2.0
//! Who is acting, and on which host.

use crate::git::Repo;

/// The agent's name as given: `--agent`, else `$SUSI_AGENT`, else `git config susi.agent`.
pub fn name(given: Option<&str>, repo: &Repo) -> Result<String, String> {
    let raw = given
        .map(str::to_owned)
        .or_else(|| std::env::var("SUSI_AGENT").ok())
        .or_else(|| repo.try_read(&["config", "--get", "susi.agent"]))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default();
    if raw.is_empty() {
        return Err("say who you are: --agent <name>, SUSI_AGENT, or git config susi.agent".into());
    }
    Ok(raw)
}

/// The token claims are kept under: upper case, letters and digits only (`claude` -> `CLAUDE`).
pub fn token(name: &str) -> Result<String, String> {
    let token: String = name
        .trim()
        .to_ascii_uppercase()
        .chars()
        .filter(char::is_ascii_alphanumeric)
        .collect();
    if token.is_empty() {
        return Err("an agent name needs letters or digits".into());
    }
    Ok(token)
}

/// This machine's name, as the Board records it.
pub fn host() -> String {
    for var in ["COMPUTERNAME", "HOSTNAME"] {
        if let Ok(value) = std::env::var(var) {
            if !value.trim().is_empty() {
                return value.trim().to_owned();
            }
        }
    }
    std::fs::read_to_string("/etc/hostname")
        .map(|s| s.trim().to_owned())
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "unknown-host".to_owned())
}

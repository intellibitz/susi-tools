// SPDX-License-Identifier: Apache-2.0
//! Who is acting, and on which host.

use crate::git::Repo;

/// Names of roles, not of agents. A role is held by an agent working from its own root; a root
/// of the role's own would be shared by every vendor that holds it, and on 2026-11-25 and 26
/// `~/susi-agents/lead` was, by Cursor and others in turn (the CEO, 2026-11-26: "agents must be
/// working on their own unique worktrees").
pub const ROLES: [&str; 1] = ["lead"];

/// Why `name` cannot be an agent's name, when it is a role's.
pub fn role(name: &str) -> Option<String> {
    let name = name.trim();
    ROLES
        .iter()
        .any(|role| name.eq_ignore_ascii_case(role))
        .then(|| {
            format!(
                "`{name}` is a role, not an agent: agents never share a folder. Work as your own tool's agent (claude, codex, antigravity, grok, cursor) from your own root, and hold the lead with `susi-tools lead claim`"
            )
        })
}

/// The name as given, trimmed, or empty: `--agent`, else `$SUSI_AGENT`, else `git config
/// susi.agent`.
pub fn given(given: Option<&str>, repo: &Repo) -> String {
    given
        .map(str::to_owned)
        .or_else(|| std::env::var("SUSI_AGENT").ok())
        .or_else(|| repo.try_read(&["config", "--get", "susi.agent"]))
        .map(|s| s.trim().to_owned())
        .unwrap_or_default()
}

/// The agent's name as given: `--agent`, else `$SUSI_AGENT`, else `git config susi.agent`; a
/// role's name is refused.
pub fn name(given_name: Option<&str>, repo: &Repo) -> Result<String, String> {
    let raw = given(given_name, repo);
    if raw.is_empty() {
        return Err("say who you are: --agent <name>, SUSI_AGENT, or git config susi.agent".into());
    }
    if let Some(why) = role(&raw) {
        return Err(why);
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

#[cfg(test)]
mod tests {
    #[test]
    fn a_role_is_no_agents_name() {
        for name in ["lead", "Lead", " LEAD "] {
            assert!(super::role(name).is_some(), "{name}");
        }
        for name in [
            "claude",
            "codex",
            "antigravity",
            "grok",
            "cursor",
            "lander",
            "leader",
        ] {
            assert!(super::role(name).is_none(), "{name}");
        }
    }
}

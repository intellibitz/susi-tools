// SPDX-License-Identifier: Apache-2.0
//! `local-harvest`, the name Susi's release gate and history call the harvest tool by. Its
//! `audit` is Susi Tools' rebuilt one (`src/harvest.rs`), the same checks and messages
//! asked of Git in batches; every other command (`intake`, `scan`, `import`, `notices`,
//! `route`, `pickup`, `raw-features`, `harvested-by`) goes to the earlier program, installed
//! beside this one as `local-harvest-legacy` (or named by `LOCAL_HARVEST_LEGACY`), until it is
//! rebuilt too.
#![forbid(unsafe_code)]

#[path = "../git_program.rs"]
mod git_program;
#[path = "../harvest.rs"]
mod harvest;
#[path = "../json.rs"]
#[allow(dead_code)]
mod json;
#[path = "../sha256.rs"]
#[allow(dead_code)]
mod sha256;

use std::path::PathBuf;
use std::process::ExitCode;

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) != Some("audit") {
        return forward(&args);
    }
    let report = std::env::current_dir()
        .map_err(|e| e.to_string())
        .and_then(|cwd| harvest::run_audit(&cwd, &args[1..]));
    match report {
        Ok(report) if report.ok => {
            println!("{}", report.text);
            ExitCode::SUCCESS
        }
        Ok(report) => {
            eprintln!("local-harvest: {}", report.text);
            ExitCode::FAILURE
        }
        Err(message) => {
            eprintln!("local-harvest: {message}");
            ExitCode::FAILURE
        }
    }
}

fn forward(args: &[String]) -> ExitCode {
    let legacy = std::env::var_os("LOCAL_HARVEST_LEGACY")
        .map(PathBuf::from)
        .or_else(|| {
            let exe = if cfg!(windows) {
                "local-harvest-legacy.exe"
            } else {
                "local-harvest-legacy"
            };
            std::env::var_os("PATH").and_then(|path| {
                std::env::split_paths(&path)
                    .map(|dir| dir.join(exe))
                    .find(|candidate| candidate.is_file())
            })
        });
    let Some(legacy) = legacy else {
        eprintln!(
            "local-harvest: only `audit` is rebuilt; install the earlier program as local-harvest-legacy for {:?}",
            args.first().map(String::as_str).unwrap_or("")
        );
        return ExitCode::from(2);
    };
    match std::process::Command::new(&legacy).args(args).status() {
        Ok(status) => ExitCode::from(status.code().map_or(1, |c| u8::try_from(c).unwrap_or(1))),
        Err(e) => {
            eprintln!("local-harvest: cannot run {}: {e}", legacy.display());
            ExitCode::FAILURE
        }
    }
}

// SPDX-License-Identifier: Apache-2.0
//! `susi-tools`: the development program of the Susi AI OS ecosystem, rebuilt small in Susi Tools
//! (foundation plan, phase 3). This first slice only reads: the Board and the task queue. It
//! writes nothing anywhere, inside a checkout or out.

#![forbid(unsafe_code)]

mod board;
mod git;
mod json;
mod tasks;
mod time;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
susi-tools 0.1: the Susi AI OS development program (read-only slice)

Usage:
  susi-tools board list [--repo <path>] [--limit <n>] [--json]
  susi-tools tasks list [--repo <path>] [--ref <ref>] [--json]
  susi-tools --version | --help

--repo   the repository that holds the Board or the queue: a bare repository, a checkout
         or a worktree (default: $SUSI_TOOLS_REPO, else the current directory)
--ref    where the open task records are read (default: main)
--limit  how many of the newest Board items to show (default: 20)
--json   machine-readable output

Nothing here writes: not to a checkout, its index, its ignored files or anywhere else.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();
    match run(&args) {
        Ok(()) => ExitCode::SUCCESS,
        Err(Failure::Usage(message)) => {
            eprintln!("susi-tools: {message}\n\n{USAGE}");
            ExitCode::from(2)
        }
        Err(Failure::Run(message)) => {
            eprintln!("susi-tools: {message}");
            ExitCode::FAILURE
        }
    }
}

enum Failure {
    Usage(String),
    Run(String),
}

impl From<String> for Failure {
    fn from(message: String) -> Self {
        Failure::Run(message)
    }
}

struct Options {
    repo: Option<PathBuf>,
    reference: String,
    limit: usize,
    json: bool,
}

fn options(rest: &[String]) -> Result<Options, Failure> {
    let mut opts = Options {
        repo: None,
        reference: "main".into(),
        limit: 20,
        json: false,
    };
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        let mut value = |name: &str| {
            it.next()
                .cloned()
                .ok_or_else(|| Failure::Usage(format!("{name} needs a value")))
        };
        match arg.as_str() {
            "--repo" => opts.repo = Some(PathBuf::from(value("--repo")?)),
            "--ref" => opts.reference = value("--ref")?,
            "--limit" => {
                let raw = value("--limit")?;
                opts.limit = raw
                    .parse()
                    .map_err(|_| Failure::Usage(format!("--limit wants a number, not {raw:?}")))?;
            }
            "--json" => opts.json = true,
            other => return Err(Failure::Usage(format!("unknown option {other:?}"))),
        }
    }
    Ok(opts)
}

fn repository(opts: &Options) -> Result<git::Repo, Failure> {
    let path = opts
        .repo
        .clone()
        .or_else(|| std::env::var_os("SUSI_TOOLS_REPO").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("."));
    Ok(git::Repo::open(&path)?)
}

fn run(args: &[String]) -> Result<(), Failure> {
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    match words.as_slice() {
        [] | ["--help" | "-h" | "help"] => {
            print!("{USAGE}");
            Ok(())
        }
        ["--version" | "-V"] => {
            println!("susi-tools {}", env!("CARGO_PKG_VERSION"));
            Ok(())
        }
        ["board", "list", ..] => {
            let opts = options(&args[2..])?;
            let repo = repository(&opts)?;
            let (items, last) = board::latest(&repo, opts.limit)?;
            if opts.json {
                println!("{}", board::to_json(&items).to_pretty());
            } else {
                println!("board: {} item(s) shown, last is {last}", items.len());
                for item in &items {
                    println!("{}", board::line(item));
                }
            }
            Ok(())
        }
        ["tasks", "list", ..] => {
            let opts = options(&args[2..])?;
            let repo = repository(&opts)?;
            let open = tasks::open(&repo, &opts.reference, time::now_unix())?;
            if opts.json {
                println!("{}", tasks::to_json(&open).to_pretty());
            } else {
                println!("{} open task(s) at {}", open.len(), opts.reference);
                for t in &open {
                    let holder = match (&t.claim, t.live) {
                        (Some(c), true) => format!("claimed by {}", c.text("agent")),
                        (Some(c), false) => format!("claim by {} lapsed", c.text("agent")),
                        (None, _) => "unclaimed".to_owned(),
                    };
                    println!(
                        "  {:<22} {:<2} {:<26} {}",
                        t.id,
                        t.record.text("size"),
                        holder,
                        t.record.text("title")
                    );
                }
            }
            Ok(())
        }
        _ => Err(Failure::Usage(format!(
            "unknown command {:?}",
            words.join(" ")
        ))),
    }
}

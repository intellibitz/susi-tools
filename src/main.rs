// SPDX-License-Identifier: Apache-2.0
//! `susi-tools`: the development program of the Susi AI OS ecosystem, rebuilt small in Susi Tools
//! (foundation plan, phase 3): the Board, the task queue and its claims, and shipping, over plain
//! Git. It never writes a file of its own into a checkout: what it changes goes through Git.

#![forbid(unsafe_code)]

mod agent;
mod board;
mod check;
mod claims;
mod git;
mod git_program;
mod home;
mod hooks;
mod json;
mod ship;
mod tasks;
mod time;

use std::path::PathBuf;
use std::process::ExitCode;

const USAGE: &str = "\
susi-tools 0.2: the Susi AI OS development program

The Board
  susi-tools board list [--limit <n>] [--json]
  susi-tools board check
  susi-tools board post --kind <kind> --title <one line> [--body <text> | --body-file <file|->]
                        [--to <agent|all>] [--re <n>] [--task <id>]
Tasks
  susi-tools tasks list [--ref <ref>] [--json]
  susi-tools tasks add <title> --accept <command> [--goal <text>] [--size xs|s|m|l] [--dep <id>]...
  susi-tools tasks claim <id> --scope <path>... [--hours <1-24>]
  susi-tools tasks renew [--hours <1-24>]
  susi-tools tasks release <id>
  susi-tools tasks close <id>
  susi-tools tasks start <title> --accept <command> --scope <path>... [--goal <text>] [--size xs|s|m|l]
                         [--hours <1-24>]
                                 add, commit and ship the task's record, then claim it: one command
  susi-tools tasks finish <id>   close the task (its acceptance runs) and ship: one command
Shipping
  susi-tools ship
Harvested code: the audit is the second program, local-harvest audit [--range <A..B>] [--json]
The session start
  susi-tools workflow check      fast-forward a clean checkout, show what is addressed to you,
                                 say what you hold
The checks the Git hooks run (each refuses only what its rule names)
  susi-tools workflow root show [dir] | root init --agent <name> [dir]
  susi-tools workflow fresh       refuse a commit far behind the remote's main
  susi-tools workflow reuse --message-file <file> | <task>
  susi-tools workflow guard commit|merge

The earlier program's spellings work too: workflow board list|post|check, workflow ship.
Any other command goes to the earlier program, installed beside this one as susi-tools-legacy.

Every command also takes:
  --repo <path>    the repository: a checkout or worktree, or (for list) a bare repository
                   (default: $SUSI_TOOLS_REPO, else the current directory)
  --remote <name>  where the Board, claims and main live (default: origin)
  --agent <name>   who you are (default: $SUSI_AGENT, else git config susi.agent)

What each writes: board post creates one numbered item; claim, renew and release change your
claim refs in one atomic push; add writes the new task's record into the checkout for you to
commit; close runs the task's acceptance and, only if it passes, commits the record's removal;
ship pushes your committed work to main as a fast-forward and posts Landed. list writes
nothing.
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

#[derive(Default)]
struct Options {
    positional: Vec<String>,
    repo: Option<PathBuf>,
    remote: Option<String>,
    agent: Option<String>,
    reference: Option<String>,
    limit: Option<usize>,
    json: bool,
    kind: Option<String>,
    title: Option<String>,
    body: Option<String>,
    body_file: Option<String>,
    to: Option<String>,
    re: Option<u64>,
    task: Option<String>,
    accept: Option<String>,
    goal: Option<String>,
    size: Option<String>,
    deps: Vec<String>,
    scopes: Vec<String>,
    hours: Option<i64>,
}

fn parse(rest: &[String], allowed: &[&str]) -> Result<Options, Failure> {
    let mut o = Options::default();
    let mut it = rest.iter();
    while let Some(arg) = it.next() {
        if !arg.starts_with("--") {
            o.positional.push(arg.clone());
            continue;
        }
        let name = arg.as_str();
        if !allowed.contains(&name) && !["--repo", "--remote", "--agent"].contains(&name) {
            return Err(Failure::Usage(format!("{name} does not apply here")));
        }
        if name == "--json" {
            o.json = true;
            continue;
        }
        let value = it
            .next()
            .cloned()
            .ok_or_else(|| Failure::Usage(format!("{name} needs a value")))?;
        let number = |v: &str| {
            v.parse::<i64>()
                .map_err(|_| Failure::Usage(format!("{name} wants a number, not {v:?}")))
        };
        match name {
            "--repo" => o.repo = Some(PathBuf::from(value)),
            "--remote" => o.remote = Some(value),
            "--agent" => o.agent = Some(value),
            "--ref" => o.reference = Some(value),
            "--limit" => o.limit = Some(number(&value)?.max(0) as usize),
            "--kind" => o.kind = Some(value),
            "--title" => o.title = Some(value),
            "--body" => o.body = Some(value),
            "--body-file" => o.body_file = Some(value),
            "--to" => o.to = Some(value),
            "--re" => o.re = Some(number(&value)?.max(0) as u64),
            "--task" => o.task = Some(value),
            "--accept" => o.accept = Some(value),
            "--goal" => o.goal = Some(value),
            "--size" => o.size = Some(value),
            "--dep" => o.deps.push(value),
            "--scope" => o.scopes.push(value),
            "--hours" => o.hours = Some(number(&value)?),
            _ => return Err(Failure::Usage(format!("unknown option {name}"))),
        }
    }
    Ok(o)
}

impl Options {
    fn path(&self) -> PathBuf {
        self.repo
            .clone()
            .or_else(|| std::env::var_os("SUSI_TOOLS_REPO").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."))
    }

    /// The repository that holds the Board (Susi's), for work in `repo`.
    fn board(&self, repo: &git::Repo) -> Result<git::Repo, Failure> {
        Ok(home::board_repo(repo, &self.path())?)
    }

    fn repo(&self) -> Result<git::Repo, Failure> {
        let path = self
            .repo
            .clone()
            .or_else(|| std::env::var_os("SUSI_TOOLS_REPO").map(PathBuf::from))
            .unwrap_or_else(|| PathBuf::from("."));
        Ok(git::Repo::open(&path)?)
    }

    fn remote(&self) -> String {
        self.remote.clone().unwrap_or_else(|| "origin".into())
    }

    /// `--repo`, `--remote` and `--agent` as given, to pass on to the commands a command runs.
    fn common_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(repo) = &self.repo {
            args.push("--repo".to_owned());
            args.push(repo.display().to_string());
        }
        if let Some(remote) = &self.remote {
            args.push("--remote".to_owned());
            args.push(remote.clone());
        }
        if let Some(agent) = &self.agent {
            args.push("--agent".to_owned());
            args.push(agent.clone());
        }
        args
    }

    fn one_id(&self, what: &str) -> Result<String, Failure> {
        match self.positional.as_slice() {
            [id] => Ok(id.clone()),
            _ => Err(Failure::Usage(format!("{what} takes one task id"))),
        }
    }
}

fn read_body(o: &Options) -> Result<String, Failure> {
    match (&o.body, &o.body_file) {
        (Some(_), Some(_)) => Err(Failure::Usage(
            "give --body or --body-file, not both".into(),
        )),
        (Some(body), None) => Ok(body.clone()),
        (None, Some(file)) if file == "-" => {
            let mut text = String::new();
            std::io::Read::read_to_string(&mut std::io::stdin(), &mut text)
                .map_err(|e| Failure::Run(format!("reading the body from stdin: {e}")))?;
            Ok(text.trim_end().to_owned())
        }
        (None, Some(file)) => std::fs::read_to_string(file)
            .map(|t| t.trim_end().to_owned())
            .map_err(|e| Failure::Run(format!("reading {file}: {e}"))),
        (None, None) => Ok(String::new()),
    }
}

fn post_line(repo: &git::Repo, remote: &str, post: &board::Post<'_>) -> Result<(), Failure> {
    let (n, _) = board::post(repo, remote, post)?;
    println!("posted item {n} on the Board: {} {}", post.kind, post.title);
    Ok(())
}

fn claim_post(repo: &git::Repo, remote: &str, claim: &claims::Claim) -> Result<(), Failure> {
    let title = format!("Claimed {}", claim.task);
    let body = format!(
        "Lease until unix {} (scopes=[{}]).",
        claim.lease_until_unix,
        claim.scopes.join(", ")
    );
    post_line(
        repo,
        remote,
        &board::Post {
            kind: "claim",
            agent: &claim.agent,
            title: &title,
            body: &body,
            to: None,
            re: None,
            task: Some(&claim.task),
        },
    )
}

fn release_post(repo: &git::Repo, remote: &str, task: &str, agent: &str) -> Result<(), Failure> {
    let title = format!("Released {task}");
    post_line(
        repo,
        remote,
        &board::Post {
            kind: "handoff",
            agent,
            title: &title,
            body: "Released task lease on remote.",
            to: None,
            re: None,
            task: Some(task),
        },
    )
}

fn run(args: &[String]) -> Result<(), Failure> {
    // The earlier program's spellings still work: `workflow board …`, `workflow ship`.
    let mut args = args.to_vec();
    if args.first().map(String::as_str) == Some("workflow")
        && matches!(args.get(1).map(String::as_str), Some("board" | "ship"))
    {
        args.remove(0);
    }
    let args = &args[..];
    let words: Vec<&str> = args.iter().map(String::as_str).collect();
    let rest = args.get(2..).unwrap_or(&[]);
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
            let o = parse(rest, &["--limit", "--json"])?;
            let repo = o.repo()?;
            let limit = o.limit.unwrap_or(20);
            let (items, last) = if o.repo.is_some() || std::env::var_os("SUSI_TOOLS_REPO").is_some()
            {
                board::latest(&repo, limit)?
            } else {
                check::recent_items(&o.board(&repo)?, "origin", limit)?
            };
            if o.json {
                println!("{}", board::to_json(&items).to_pretty());
            } else {
                println!("board: {} item(s) shown, last is {last}", items.len());
                for item in &items {
                    println!("{}", board::line(item));
                }
            }
            Ok(())
        }
        ["board", "check", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            Ok(check::board_check(&o.board(&repo)?, "origin", &agent)?)
        }
        ["workflow", "check", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let board = o.board(&repo)?;
            Ok(check::workflow_check(&repo, &o.remote(), &board, &agent)?)
        }
        ["board", "post", ..] => {
            let o = parse(
                rest,
                &[
                    "--kind",
                    "--title",
                    "--body",
                    "--body-file",
                    "--to",
                    "--re",
                    "--task",
                ],
            )?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let body = read_body(&o)?;
            let kind = o
                .kind
                .clone()
                .ok_or_else(|| Failure::Usage("--kind is required".into()))?;
            let title = o
                .title
                .clone()
                .ok_or_else(|| Failure::Usage("--title is required".into()))?;
            post_line(
                &o.board(&repo)?,
                "origin",
                &board::Post {
                    kind: &kind,
                    agent: &agent,
                    title: &title,
                    body: &body,
                    to: o.to.as_deref(),
                    re: o.re,
                    task: o.task.as_deref(),
                },
            )
        }
        ["tasks", "list", ..] => {
            let o = parse(rest, &["--ref", "--json"])?;
            let repo = o.repo()?;
            let reference = o.reference.clone().unwrap_or_else(|| "main".into());
            let open = tasks::open(&repo, &reference, time::now_unix())?;
            if o.json {
                println!("{}", tasks::to_json(&open).to_pretty());
            } else {
                println!("{} open task(s) at {reference}", open.len());
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
        ["tasks", "add", ..] => {
            let o = parse(rest, &["--accept", "--goal", "--size", "--dep"])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let title = match o.positional.as_slice() {
                [title] => title.clone(),
                _ => return Err(Failure::Usage("tasks add takes one title".into())),
            };
            let accept: Vec<String> = o
                .accept
                .clone()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            let (id, path) = tasks::add(
                &repo,
                &tasks::NewTask {
                    title: &title,
                    goal: o.goal.as_deref().unwrap_or(""),
                    size: o.size.as_deref().unwrap_or("m"),
                    accept: &accept,
                    deps: &o.deps,
                    agent: &agent,
                },
            )?;
            println!("added {id} at {}: commit it to share it", path.display());
            Ok(())
        }
        ["tasks", "start", ..] => {
            let o = parse(
                rest,
                &[
                    "--accept", "--goal", "--size", "--dep", "--scope", "--hours",
                ],
            )?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let title = match o.positional.as_slice() {
                [title] => title.clone(),
                _ => return Err(Failure::Usage("tasks start takes one title".into())),
            };
            if o.scopes.is_empty() {
                return Err(Failure::Usage(
                    "tasks start needs --scope <path>: what the task will change".into(),
                ));
            }
            if !repo
                .read(&["status", "--porcelain", "--untracked-files=no"])?
                .trim()
                .is_empty()
            {
                return Err(Failure::Run(
                    "commit or set aside your changes first: tasks start commits only its own record"
                        .into(),
                ));
            }
            let accept: Vec<String> = o
                .accept
                .clone()
                .unwrap_or_default()
                .split_whitespace()
                .map(str::to_owned)
                .collect();
            let (id, path) = tasks::add(
                &repo,
                &tasks::NewTask {
                    title: &title,
                    goal: o.goal.as_deref().unwrap_or(""),
                    size: o.size.as_deref().unwrap_or("m"),
                    accept: &accept,
                    deps: &o.deps,
                    agent: &agent,
                },
            )?;
            let record = path.display().to_string();
            repo.read(&["add", "--", &record])?;
            repo.run_visible(&[
                "commit",
                "-q",
                "-m",
                &task_subject(&id, &title),
                "-m",
                &format!("Task: {id}"),
                "--",
                &record,
            ])?;
            println!("added {id} and committed its record");
            let common = o.common_args();
            run(&[vec!["ship".to_owned()], common.clone()].concat())?;
            let mut claim = vec!["tasks".to_owned(), "claim".to_owned(), id];
            for scope in &o.scopes {
                claim.push("--scope".into());
                claim.push(scope.clone());
            }
            if let Some(hours) = o.hours {
                claim.push("--hours".into());
                claim.push(hours.to_string());
            }
            claim.extend(common);
            run(&claim)
        }
        ["tasks", "finish", ..] => {
            let o = parse(rest, &[])?;
            let id = o.one_id("tasks finish")?;
            let common = o.common_args();
            run(&[
                vec!["tasks".to_owned(), "close".to_owned(), id],
                common.clone(),
            ]
            .concat())?;
            run(&[vec!["ship".to_owned()], common].concat())
        }
        ["tasks", "claim", ..] => {
            let o = parse(rest, &["--scope", "--hours"])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let id = o.one_id("tasks claim")?;
            let branch = repo
                .try_read(&["symbolic-ref", "--short", "-q", "HEAD"])
                .map(|b| b.trim().to_owned())
                .unwrap_or_default();
            let remote = o.remote();
            // A task of Susi's queue may claim files here (Susi Tools, Susi Core).
            let queue = o.board(&repo).ok();
            let claim = claims::claim(
                &repo,
                &remote,
                &claims::Request {
                    task: &id,
                    agent: &agent,
                    scopes: &o.scopes,
                    hours: o.hours.unwrap_or(claims::DEFAULT_HOURS),
                    branch: &branch,
                    queue: queue.as_ref(),
                },
            )?;
            println!(
                "{} claimed {} until unix {}",
                claim.agent, claim.task, claim.lease_until_unix
            );
            claim_post(&o.board(&repo)?, "origin", &claim)
        }
        ["tasks", "renew", ..] => {
            let o = parse(rest, &["--hours"])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let queue = o.board(&repo).ok();
            match claims::renew(
                &repo,
                &o.remote(),
                &agent,
                o.hours.unwrap_or(claims::DEFAULT_HOURS),
                queue.as_ref(),
            )? {
                Some(c) => println!(
                    "{} holds {} until unix {}",
                    c.agent, c.task, c.lease_until_unix
                ),
                None => println!("no live claim to renew"),
            }
            Ok(())
        }
        ["tasks", "release", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let id = o.one_id("tasks release")?;
            let remote = o.remote();
            if claims::release(&repo, &remote, &id, &agent)? {
                println!("released {id}");
                release_post(&o.board(&repo)?, "origin", &id, &agent::token(&agent)?)
            } else {
                println!("{id} has no claim to release");
                Ok(())
            }
        }
        ["tasks", "close", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let id = o.one_id("tasks close")?;
            let commit = tasks::close(&repo, &id)?;
            println!(
                "closed {id} in {}: ship it to publish (`susi-tools ship`)",
                &commit[..commit.len().min(12)]
            );
            Ok(())
        }
        ["ship", ..] => {
            let o = parse(args.get(1..).unwrap_or(&[]), &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let remote = o.remote();
            match ship::ship(&repo, &remote)? {
                ship::Shipped::Nothing(tip) => {
                    println!(
                        "nothing to ship: {remote}'s main ({}) already has it",
                        &tip[..12]
                    );
                    Ok(())
                }
                ship::Shipped::FastForward {
                    from,
                    to,
                    tasks: shipped,
                } => {
                    println!(
                        "shipped {} to {remote}'s main (was {})",
                        &to[..12],
                        &from[..12]
                    );
                    let title = format!("Shipped {} to main", &to[..12]);
                    let mut body = format!("mode=fast_forward source={to} main={to}");
                    if !shipped.is_empty() {
                        body.push_str(&format!("\nTasks: {}", shipped.join(", ")));
                    }
                    let board = o.board(&repo)?;
                    post_line(
                        &board,
                        "origin",
                        &board::Post {
                            kind: "landed",
                            agent: &agent,
                            title: &title,
                            body: &body,
                            to: None,
                            re: None,
                            task: shipped.first().map(String::as_str),
                        },
                    )?;
                    after_ship(&repo, &remote, &board, &agent)
                }
            }
        }
        ["workflow", "root" | "fresh" | "reuse" | "guard", ..] => hooks::run_cli(&words[1..])
            .map_err(|stop| match stop {
                hooks::Stop::Usage(m) => Failure::Usage(m),
                hooks::Stop::Refused(m) => Failure::Run(m),
            }),
        _ => forward(args),
    }
}

/// A command this program does not have yet goes to the earlier program, installed beside it as
/// `susi-tools-legacy` (or named by `SUSI_TOOLS_LEGACY`), so every instruction that names one
/// keeps working while the contracts are shortened. Without it, the command is unknown.
fn forward(args: &[String]) -> Result<(), Failure> {
    let legacy = std::env::var_os("SUSI_TOOLS_LEGACY")
        .map(PathBuf::from)
        .or_else(|| find_on_path("susi-tools-legacy"));
    let Some(legacy) = legacy else {
        return Err(Failure::Usage(format!(
            "unknown command {:?}",
            args.join(" ")
        )));
    };
    eprintln!(
        "susi-tools: \"{}\" is not in the rebuilt program yet; running the earlier one ({})",
        args.iter().take(2).cloned().collect::<Vec<_>>().join(" "),
        legacy.display()
    );
    let status = std::process::Command::new(&legacy)
        .args(args)
        .status()
        .map_err(|e| Failure::Run(format!("cannot run {}: {e}", legacy.display())))?;
    match status.code() {
        Some(0) => Ok(()),
        Some(code) => std::process::exit(code),
        None => Err(Failure::Run(
            "the earlier program stopped on a signal".into(),
        )),
    }
}

/// `chore(tasks): add <id>, <title>`, the title cut at a word so the subject keeps to 72
/// characters.
fn task_subject(id: &str, title: &str) -> String {
    let head = format!("chore(tasks): add {id}");
    let room = 72usize.saturating_sub(head.chars().count() + 2);
    let mut kept = String::new();
    for word in title.split_whitespace() {
        let next = if kept.is_empty() {
            word.chars().count()
        } else {
            kept.chars().count() + 1 + word.chars().count()
        };
        if next > room {
            break;
        }
        if !kept.is_empty() {
            kept.push(' ');
        }
        kept.push_str(word);
    }
    let kept = kept.trim_end_matches([',', '.', ';', ':']);
    if kept.is_empty() {
        head
    } else {
        format!("{head}, {kept}")
    }
}

fn find_on_path(name: &str) -> Option<PathBuf> {
    let exe = if cfg!(windows) {
        format!("{name}.exe")
    } else {
        name.to_owned()
    };
    std::env::split_paths(&std::env::var_os("PATH")?)
        .map(|dir| dir.join(&exe))
        .find(|candidate| candidate.is_file())
}

/// Landing work keeps a claim alive; a claim on a task this ship closed is released (Mandate 85).
fn after_ship(
    repo: &git::Repo,
    remote: &str,
    board: &git::Repo,
    agent: &str,
) -> Result<(), Failure> {
    let token = agent::token(agent)?;
    let state = claims::State::read(repo, remote)?;
    let Some((_, _, mine)) = state
        .of_agent(&token)
        .filter(|(_, _, c)| c.live(time::now_unix()))
    else {
        return Ok(());
    };
    let record = format!(
        "{}:{}{}.json",
        tasks::MAIN_SNAPSHOT,
        tasks::TASKS_DIR,
        mine.task
    );
    // The task is open while its record is on main here, or, for a task of Susi's queue claimed
    // in Susi Tools or Susi Core, on Susi's main.
    let open_here = repo.try_read(&["cat-file", "-e", &record]).is_some();
    let open_in_queue = !open_here
        && board.git_dir() != repo.git_dir()
        && tasks::fetch_main(board, "origin").is_ok()
        && board.try_read(&["cat-file", "-e", &record]).is_some();
    if open_here || open_in_queue {
        let renewed = claims::renew(repo, remote, agent, claims::DEFAULT_HOURS, Some(board))?;
        if let Some(c) = renewed {
            println!(
                "{} is yours until unix {}: landing work renewed the claim",
                c.task, c.lease_until_unix
            );
        }
        Ok(())
    } else {
        let task = mine.task.clone();
        if claims::release(repo, remote, &task, agent)? {
            println!("{task} is closed on main: its claim is released");
            release_post(board, "origin", &task, &token)?;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::git_program::real_git;
    use super::task_subject;
    use std::path::{Path, PathBuf};

    /// A made-up Git for Windows install under a fresh folder: the launcher in `cmd`, and the
    /// real program with its `libexec\git-core` under `ucrt64` when `real` is set.
    fn install(tag: &str, real: bool) -> PathBuf {
        let root =
            std::env::temp_dir().join(format!("susi-tools-git-{tag}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        touch(&root.join("Git/cmd/git.exe"));
        if real {
            touch(&root.join("Git/ucrt64/bin/git.exe"));
            std::fs::create_dir_all(root.join("Git/ucrt64/libexec/git-core")).unwrap();
        }
        root
    }

    fn touch(file: &Path) {
        std::fs::create_dir_all(file.parent().unwrap()).unwrap();
        std::fs::write(file, b"").unwrap();
    }

    fn path_of(dirs: &[PathBuf]) -> std::ffi::OsString {
        std::env::join_paths(dirs).unwrap()
    }

    #[test]
    fn the_real_git_is_started_instead_of_git_for_windows_launcher() {
        let root = install("launcher", true);
        let other = root.join("tools");
        std::fs::create_dir_all(&other).unwrap();
        let found = real_git(&path_of(&[other, root.join("Git/cmd")]));
        assert_eq!(
            found,
            Some(root.join("Git/ucrt64").join("bin").join("git.exe"))
        );
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn any_other_first_git_is_left_to_path_as_before() {
        let root = install("other", true);
        // A git that is not the launcher comes first on the path: it is the one started.
        touch(&root.join("scoop/shims/git.exe"));
        let first = path_of(&[root.join("scoop/shims"), root.join("Git/cmd")]);
        assert_eq!(real_git(&first), None);
        // The real program itself first on the path: nothing to replace.
        assert_eq!(real_git(&path_of(&[root.join("Git/ucrt64/bin")])), None);
        // A launcher without the real program beside it, and no git at all.
        let bare = install("bare", false);
        assert_eq!(real_git(&path_of(&[bare.join("Git/cmd")])), None);
        assert_eq!(real_git(&path_of(&[root.join("scoop")])), None);
        // Relative entries are never searched.
        assert_eq!(real_git(std::ffi::OsStr::new("Git/cmd")), None);
        let _ = std::fs::remove_dir_all(&root);
        let _ = std::fs::remove_dir_all(&bare);
    }

    #[test]
    fn a_task_commit_subject_keeps_to_72_characters_and_cuts_at_a_word() {
        assert_eq!(
            task_subject("T-CLAUDE-60", "Fix the gate"),
            "chore(tasks): add T-CLAUDE-60, Fix the gate"
        );
        let long = task_subject(
            "T-CLAUDE-60",
            "Make the agent vault installable by another company, under a licence it checks",
        );
        assert!(long.chars().count() <= 72, "{long}");
        assert!(long.starts_with("chore(tasks): add T-CLAUDE-60, Make the agent vault"));
        assert!(!long.ends_with(','), "{long}");
        assert_eq!(
            task_subject(
                "T-CLAUDE-60",
                "Supercalifragilisticexpialidocious-and-then-some-more-words"
            ),
            "chore(tasks): add T-CLAUDE-60"
        );
    }
}

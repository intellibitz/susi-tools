// SPDX-License-Identifier: Apache-2.0
//! `susi-tools`: the development program of the Susi AI OS ecosystem: the Board, the task queue
//! and its claims, shipping and the Git hooks' rules, over plain Git. It never writes a file of
//! its own into a checkout: what it changes goes through Git.

#![forbid(unsafe_code)]

mod agent;
mod backup;
mod board;
mod check;
mod claims;
mod gates;
mod git;
mod git_program;
mod githook;
mod home;
mod hooks;
mod json;
mod land;
mod lead;
mod ship;
mod speed;
mod tasks;
mod time;

use std::path::{Path, PathBuf};
use std::process::ExitCode;

const USAGE: &str = "\
susi-tools 0.2: the Susi AI OS development program

The Board
  susi-tools board list [--limit <n>]
  susi-tools board check
  susi-tools board post --kind <kind> --title <one line> [--body <text>]
                        [--to <agent|all>] [--re <n>] [--task <id>]
Tasks
  susi-tools tasks list
  susi-tools tasks add <title> --accept <command> --scope <path>... --vision V-<n> [--for <agent>]
                       [--read <path>]... [--rung built|used|ready|released|delivered]
                       [--goal <text>] [--size xs|s|m|l] [--dep <id>]...
                                 add, commit and ship a task's record for an agent to pick up
  susi-tools tasks brief <id>    the words to give an agent so it can do the task from nothing
  susi-tools tasks next          the brief of the next open task for you (meant for you or anyone,
                                 unclaimed, its dependencies closed)
  susi-tools tasks claim <id> [--scope <path>...] [--hours <1-24>]
                                 without --scope, the paths its record names
  susi-tools tasks release <id>
  susi-tools tasks close <id>
  susi-tools tasks start <title> --accept <command> --scope <path>... --vision V-<n>
                         [--read <path>]... [--rung <rung>] [--goal <text>] [--size xs|s|m|l]
                         [--dep <id>]... [--hours <1-24>]
                                 add, commit and ship the task's record, then claim it: one command
  susi-tools tasks finish <id>   close the task (its acceptance runs) and ship: one command
Shipping
  susi-tools ship
Speed
  susi-tools speed [--cold] [--json]
                                 time everyday cargo, git and check operations against the 10s line
  susi-tools land [--target <triple>]...
                                 on a company host: gate the work agents shipped for landing
                                 (refs/agents/<name>, from a clone with `git config susi.land host`)
                                 and fast-forward main with what passes; the Board says which
  susi-tools gates [--target <triple>]...
                                 every gate before a ship (tests, tests without default features,
                                 format, Clippy, Clippy per target), side by side, inside the line
Harvested code: the second program, local-harvest audit [--range <A..B>] | notices [--check]
The lead (a role any agent takes: the one the CEO is talking to)
  susi-tools lead                the lead's bearings: who leads, the brick map with each brick's
                                 rung, next step and tasks, what waits on the CEO, the visions
                                 placed nowhere, the open tasks no brick names, the Board for you
  susi-tools lead claim [--hours <1-24>] [--take]
                                 lead (8 hours unless told); --take when another agent leads and
                                 the CEO is talking to you: the Board tells the one you took it from
  susi-tools lead release        give the lead up
The session start
  susi-tools workflow check      fast-forward a clean checkout, show unanswered requests and the
                                 Board items addressed to you not shown yet, say what you hold
The Git hooks of Susi and Susi Tools: each .githooks/<stage> is one line that runs
  susi-tools hook pre-commit | commit-msg <file> | pre-push <remote> [url]
                  | post-checkout <old> <new> <flag>
                  (susi-tools hook --help for the options)
Your agent root, which holds your clones, worktrees, build targets and scratch (the hooks
refuse a commit inside another agent's root)
  susi-tools workflow root init --agent <name> [dir]   (default ~/susi-agents/<name>)

Any other command goes to the earlier program where it is installed beside this one as
susi-tools-legacy (the Windows host); elsewhere it is unknown.

Every command also takes:
  --repo <path>    the checkout or worktree (default: the current directory); its origin
                   holds main and the claims
  --agent <name>   who you are (default: $SUSI_AGENT, else git config susi.agent)

What each writes: board post creates one numbered item; claim and release change your claim
refs in one atomic push (claiming a task you hold again renews it); start commits the new
task's record and ships it, then claims; close runs the task's acceptance and, only if it
passes, commits the record's removal; ship pushes your committed work to main as a
fast-forward, posts Landed, prints Board items addressed to you not shown yet (the clone
keeps the last number shown), and renews your claim. list writes nothing.
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
        Err(Failure::Silent) => ExitCode::FAILURE,
    }
}

enum Failure {
    Usage(String),
    Run(String),
    /// The command already printed its report and exits non-zero.
    Silent,
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
    agent: Option<String>,
    limit: Option<usize>,
    kind: Option<String>,
    title: Option<String>,
    body: Option<String>,
    to: Option<String>,
    re: Option<u64>,
    task: Option<String>,
    accept: Option<String>,
    goal: Option<String>,
    size: Option<String>,
    deps: Vec<String>,
    scopes: Vec<String>,
    hours: Option<i64>,
    vision: Option<String>,
    rung: Option<String>,
    reads: Vec<String>,
    for_agent: Option<String>,
    cold: bool,
    take: bool,
    json: bool,
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
        if !allowed.contains(&name) && !["--repo", "--agent"].contains(&name) {
            return Err(Failure::Usage(format!("{name} does not apply here")));
        }
        match name {
            "--cold" => {
                o.cold = true;
                continue;
            }
            "--take" => {
                o.take = true;
                continue;
            }
            "--json" => {
                o.json = true;
                continue;
            }
            _ => {}
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
            "--agent" => o.agent = Some(value),
            "--limit" => o.limit = Some(number(&value)?.max(0) as usize),
            "--kind" => o.kind = Some(value),
            "--title" => o.title = Some(value),
            "--body" => o.body = Some(value),
            "--to" => o.to = Some(value),
            "--re" => o.re = Some(number(&value)?.max(0) as u64),
            "--task" => o.task = Some(value),
            "--accept" => o.accept = Some(value),
            "--goal" => o.goal = Some(value),
            "--size" => o.size = Some(value),
            "--dep" => o.deps.push(value),
            "--scope" => o.scopes.push(value),
            "--vision" => o.vision = Some(value),
            "--rung" => o.rung = Some(value),
            "--read" => o.reads.push(value),
            "--for" => o.for_agent = Some(value),
            "--hours" => o.hours = Some(number(&value)?),
            _ => return Err(Failure::Usage(format!("unknown option {name}"))),
        }
    }
    Ok(o)
}

/// Where main, the claims and (for Susi) the Board live: each repository's `origin`.
const REMOTE: &str = "origin";

impl Options {
    fn path(&self) -> PathBuf {
        self.repo.clone().unwrap_or_else(|| PathBuf::from("."))
    }

    /// The repository that holds the Board (Susi's), for work in `repo`.
    fn board(&self, repo: &git::Repo) -> Result<git::Repo, Failure> {
        Ok(home::board_repo(repo)?)
    }

    fn repo(&self) -> Result<git::Repo, Failure> {
        Ok(git::Repo::open(&self.path())?)
    }

    /// `--repo` and `--agent` as given, to pass on to the commands a command runs.
    fn common_args(&self) -> Vec<String> {
        let mut args = Vec::new();
        if let Some(repo) = &self.repo {
            args.push("--repo".to_owned());
            args.push(repo.display().to_string());
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
            let o = parse(rest, &["--limit"])?;
            let repo = o.repo()?;
            let limit = o.limit.unwrap_or(20);
            let (items, last) = check::recent_items(&o.board(&repo)?, REMOTE, limit)?;
            println!("board: {} item(s) shown, last is {last}", items.len());
            for item in &items {
                println!("{}", board::line(item));
            }
            Ok(())
        }
        ["board", "check", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            Ok(check::board_check(&o.board(&repo)?, REMOTE, &agent)?)
        }
        ["lead", "claim", ..] => {
            let o = parse(rest, &["--hours", "--take"])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let board = o.board(&repo)?;
            let hours = o.hours.unwrap_or(lead::DEFAULT_HOURS);
            let token = agent::token(&agent)?;
            match lead::claim(&board, REMOTE, &agent, hours, o.take)? {
                lead::Claimed::Renewed(l) => {
                    println!("{} leads until unix {} (renewed)", l.agent, l.until_unix);
                    Ok(())
                }
                lead::Claimed::Took { lead: l, from } => {
                    println!("{} leads until unix {}", l.agent, l.until_unix);
                    let title = format!("Leads Susi until unix {}", l.until_unix);
                    let body = match &from {
                        Some(previous) => format!(
                            "Took the lead from {previous}: the CEO is talking to {token} now."
                        ),
                        None => "Took the lead; nobody held it.".to_owned(),
                    };
                    post_line(
                        &board,
                        REMOTE,
                        &board::Post {
                            kind: if from.is_some() { "heads-up" } else { "claim" },
                            agent: &token,
                            title: &title,
                            body: &body,
                            to: from.as_deref(),
                            re: None,
                            task: None,
                        },
                    )
                }
            }
        }
        ["lead", "release", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let board = o.board(&repo)?;
            if lead::release(&board, REMOTE, &agent)? {
                println!("released the lead");
                post_line(
                    &board,
                    REMOTE,
                    &board::Post {
                        kind: "handoff",
                        agent: &agent::token(&agent)?,
                        title: "Released the lead",
                        body: "Nobody leads until the agent the CEO talks to next takes it.",
                        to: None,
                        re: None,
                        task: None,
                    },
                )
            } else {
                println!("nobody leads: nothing to release");
                Ok(())
            }
        }
        ["lead", ..] => {
            let o = parse(args.get(1..).unwrap_or(&[]), &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let board = o.board(&repo)?;
            print!("{}", lead::bearings(&board, REMOTE, &agent)?);
            Ok(check::board_check(&board, REMOTE, &agent)?)
        }
        ["workflow", "check", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let board = o.board(&repo)?;
            Ok(check::workflow_check(&repo, REMOTE, &board, &agent)?)
        }
        ["board", "post", ..] => {
            let o = parse(
                rest,
                &["--kind", "--title", "--body", "--to", "--re", "--task"],
            )?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let body = o.body.clone().unwrap_or_default();
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
                REMOTE,
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
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            // The remote's main and claims as they are now, not the checkout's own main.
            let main = tasks::fetch_main(&repo, REMOTE)?;
            let open = tasks::open_records(&repo, &main)?;
            // The claims came with main's fetch: reading them fetches nothing more.
            let claims = claims::State::read(&repo, REMOTE)?;
            let now = time::now_unix();
            println!("{} open task(s) at {REMOTE}'s main", open.len());
            for t in &open {
                let holder = match claims.on_task(&t.id) {
                    Some((_, _, c)) if c.live(now) => format!("claimed by {}", c.agent),
                    Some((_, _, c)) => format!("claim by {} lapsed", c.agent),
                    None => "unclaimed".to_owned(),
                };
                let vision = t.record.text("vision");
                println!(
                    "  {:<22} {:<2} {:<6} {:<26} {}",
                    t.id,
                    t.record.text("size"),
                    if vision.is_empty() {
                        "-"
                    } else {
                        vision.as_str()
                    },
                    holder,
                    t.record.text("title")
                );
            }
            Ok(())
        }
        ["tasks", "add" | "start", ..] => {
            let starting = words[1] == "start";
            let mut allowed = vec![
                "--accept", "--goal", "--size", "--dep", "--scope", "--vision", "--rung", "--read",
            ];
            allowed.push(if starting { "--hours" } else { "--for" });
            let o = parse(rest, &allowed)?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let command = words[1];
            let title = match o.positional.as_slice() {
                [title] => title.clone(),
                _ => return Err(Failure::Usage(format!("tasks {command} takes one title"))),
            };
            if o.scopes.is_empty() {
                return Err(Failure::Usage(format!(
                    "tasks {command} needs --scope <path>: what the task will change"
                )));
            }
            let Some(vision) = o.vision.clone() else {
                return Err(Failure::Usage(format!(
                    "tasks {command} needs --vision V-<n>: the CEO's vision it serves (Susi's .agents/visions)"
                )));
            };
            if !repo
                .read(&["status", "--porcelain", "--untracked-files=no"])?
                .trim()
                .is_empty()
            {
                return Err(Failure::Run(format!(
                    "commit or set aside your changes first: tasks {command} commits only its own record"
                )));
            }
            // A task serves a vision the CEO set: on Susi's main, not only in a checkout.
            tasks::vision_statement(&o.board(&repo)?, &vision)?;
            let accept = tasks::words(o.accept.as_deref().unwrap_or_default())
                .map_err(|why| Failure::Run(format!("--accept: {why}")))?;
            let (id, path) = tasks::add(
                &repo,
                &tasks::NewTask {
                    title: &title,
                    goal: o.goal.as_deref().unwrap_or(""),
                    size: o.size.as_deref().unwrap_or("m"),
                    accept: &accept,
                    deps: &o.deps,
                    agent: &agent,
                    vision: &vision,
                    rung: o.rung.as_deref().unwrap_or(""),
                    scope: &o.scopes,
                    read: &o.reads,
                    for_agent: o.for_agent.as_deref().unwrap_or(""),
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
            if !starting {
                println!("{id} is open for an agent to pick up: `susi-tools tasks brief {id}`");
                return Ok(());
            }
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
        ["tasks", "brief", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let id = o.one_id("tasks brief")?;
            let board = o.board(&repo).ok();
            let (record, repository) = tasks::find(&repo, board.as_ref(), &id)?;
            let vision = record.text("vision");
            let words = match (&board, vision.is_empty()) {
                (Some(board), false) => tasks::vision_statement(board, &vision).ok(),
                _ => None,
            };
            print!("{}", tasks::brief(&record, &repository, words.as_deref()));
            Ok(())
        }
        ["tasks", "next", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let main = tasks::fetch_main(&repo, REMOTE)?;
            let open = tasks::open_records(&repo, &main)?;
            let claims = claims::State::read(&repo, REMOTE)?;
            let Some(task) = tasks::next(&open, &claims, &agent, time::now_unix()) else {
                println!("no open task for {agent} at {REMOTE}'s main: every one is claimed, waits on another, or is meant for another agent");
                return Ok(());
            };
            let board = o.board(&repo).ok();
            let vision = task.record.text("vision");
            let words = match (&board, vision.is_empty()) {
                (Some(board), false) => tasks::vision_statement(board, &vision).ok(),
                _ => None,
            };
            print!(
                "{}",
                tasks::brief(&task.record, &tasks::repo_name(&repo), words.as_deref())
            );
            Ok(())
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
            // Without --scope, the paths the task's record names (a task made by `tasks add`).
            let scopes = if o.scopes.is_empty() {
                let board = o.board(&repo).ok();
                tasks::find(&repo, board.as_ref(), &id)
                    .map(|(record, _)| record.str_list("scope"))
                    .unwrap_or_default()
            } else {
                o.scopes.clone()
            };
            let branch = repo
                .try_read(&["symbolic-ref", "--short", "-q", "HEAD"])
                .map(|b| b.trim().to_owned())
                .unwrap_or_default();
            // A task of Susi's queue may claim files here (Susi Tools).
            let queue = o.board(&repo).ok();
            let claim = claims::claim(
                &repo,
                REMOTE,
                &claims::Request {
                    task: &id,
                    agent: &agent,
                    scopes: &scopes,
                    hours: o.hours.unwrap_or(claims::DEFAULT_HOURS),
                    branch: &branch,
                    queue: queue.as_ref(),
                },
            )?;
            println!(
                "{} claimed {} until unix {}",
                claim.agent, claim.task, claim.lease_until_unix
            );
            claim_post(&o.board(&repo)?, REMOTE, &claim)
        }
        ["tasks", "release", ..] => {
            let o = parse(rest, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let id = o.one_id("tasks release")?;
            if claims::release(&repo, REMOTE, &id, &agent)? {
                println!("released {id}");
                release_post(&o.board(&repo)?, REMOTE, &id, &agent::token(&agent)?)
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
            let remote = REMOTE;
            match ship::ship(&repo, remote, &agent)? {
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
                        REMOTE,
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
                    check::print_for_you_since_last(&board, REMOTE, &agent)?;
                    if backup::spawn(&repo, remote, &to) {
                        println!(
                            "backing up to {} in the background (.git/susi-backup.log)",
                            backup::urls(&repo).join(", ")
                        );
                    }
                    after_ship(&repo, remote, &board, &agent)
                }
                ship::Shipped::ForLanding {
                    main,
                    to,
                    agent_ref,
                    tasks: shipped,
                } => {
                    println!(
                        "shipped {} to {remote}'s {agent_ref} for a company host to gate and land (main is {})",
                        &to[..12],
                        &main[..12]
                    );
                    let title = format!("Land {} from {agent_ref}", &to[..12]);
                    let mut body = format!("mode=for_landing source={to} main={main}");
                    if !shipped.is_empty() {
                        body.push_str(&format!("\nTasks: {}", shipped.join(", ")));
                    }
                    let board = o.board(&repo)?;
                    post_line(
                        &board,
                        REMOTE,
                        &board::Post {
                            kind: "request",
                            agent: &agent,
                            title: &title,
                            body: &body,
                            to: None,
                            re: None,
                            task: shipped.first().map(String::as_str),
                        },
                    )
                }
            }
        }
        ["land", ..] => {
            // A company host's lander: gates what agents shipped for landing, lands what passes.
            let rest = args.get(1..).unwrap_or(&[]);
            let mut targets = Vec::new();
            let mut plain = Vec::new();
            let mut i = 0;
            while i < rest.len() {
                if rest[i] == "--target" {
                    let triple = rest
                        .get(i + 1)
                        .ok_or_else(|| Failure::Usage("--target takes a target triple".into()))?;
                    targets.push(triple.to_string());
                    i += 2;
                } else {
                    plain.push(rest[i].clone());
                    i += 1;
                }
            }
            let o = parse(&plain, &[])?;
            let repo = o.repo()?;
            let agent = agent::name(o.agent.as_deref(), &repo)?;
            let Some(outcomes) = land::land(&repo, REMOTE, &targets).map_err(Failure::Run)? else {
                println!("another land is running here: nothing done");
                return Ok(());
            };
            if outcomes.is_empty() {
                println!("nothing to land");
                return Ok(());
            }
            let board = o.board(&repo)?;
            for outcome in &outcomes {
                let to_agent = outcome.agent().to_owned();
                let (kind, title, body, to) = match outcome {
                    land::Outcome::Landed {
                        agent_ref,
                        tip,
                        tasks,
                        seconds,
                    } => {
                        println!(
                            "landed {} from {agent_ref} (gates {seconds:.1} s)",
                            &tip[..12]
                        );
                        let mut body = format!(
                            "mode=landed source={tip} main={tip}\nGates: {seconds:.1} s on {}",
                            agent::host()
                        );
                        if !tasks.is_empty() {
                            body.push_str(&format!("\nTasks: {}", tasks.join(", ")));
                        }
                        (
                            "landed",
                            format!("Landed {} from {agent_ref}", &tip[..12]),
                            body,
                            None,
                        )
                    }
                    land::Outcome::Failed {
                        agent_ref,
                        tip,
                        failed,
                        seconds,
                    } => {
                        let names: Vec<&str> = failed.iter().map(|(n, _)| n.as_str()).collect();
                        println!(
                            "not landed {} from {agent_ref}: {} failed ({seconds:.1} s)",
                            &tip[..12],
                            names.join(", ")
                        );
                        let mut body = format!(
                            "The gates failed on {}: fix it, run `susi-tools gates`, ship again.\n",
                            agent::host()
                        );
                        for (name, tail) in failed {
                            let tail: String = tail
                                .chars()
                                .rev()
                                .take(1200)
                                .collect::<Vec<_>>()
                                .into_iter()
                                .rev()
                                .collect();
                            body.push_str(&format!("\n== {name}\n{tail}\n"));
                        }
                        (
                            "answer",
                            format!(
                                "Not landed: {} from {agent_ref}, {} failed",
                                &tip[..12],
                                names.join(", ")
                            ),
                            body,
                            Some(to_agent),
                        )
                    }
                    land::Outcome::Behind {
                        agent_ref,
                        tip,
                        main,
                    } => {
                        println!("not landed {} from {agent_ref}: main moved on", &tip[..12]);
                        (
                            "answer",
                            format!("Not landed: main moved past {} from {agent_ref}", &tip[..12]),
                            format!(
                                "main is now {main}: merge it into your work (`git merge origin/main`), run the gates, ship again."
                            ),
                            Some(to_agent),
                        )
                    }
                };
                post_line(
                    &board,
                    REMOTE,
                    &board::Post {
                        kind,
                        agent: &agent,
                        title: &title,
                        body: &body,
                        to: to.as_deref(),
                        re: None,
                        task: None,
                    },
                )?;
            }
            Ok(())
        }
        ["speed", ..] => {
            let o = parse(args.get(1..).unwrap_or(&[]), &["--cold", "--json"])?;
            if !o.positional.is_empty() {
                return Err(Failure::Usage("speed takes no positional arguments".into()));
            }
            let repo = o.repo()?;
            let work = repo.work_tree()?;
            if !work.join("Cargo.toml").is_file() {
                return Err(Failure::Usage(format!(
                    "not a Cargo checkout: {}",
                    work.display()
                )));
            }
            let report = speed::run(
                &repo,
                &speed::Config {
                    cold: o.cold,
                    agent: o.agent.as_deref(),
                },
            )?;
            if o.json {
                print!("{}", report.render_json());
            } else {
                print!("{}", report.render_table());
            }
            if report.passed() {
                Ok(())
            } else {
                Err(Failure::Silent)
            }
        }
        ["gates", ..] => {
            let rest = args.get(1..).unwrap_or(&[]);
            let mut targets = Vec::new();
            let mut i = 0;
            while i < rest.len() {
                match rest[i].as_str() {
                    "--target" => {
                        let triple = rest.get(i + 1).ok_or_else(|| {
                            Failure::Usage("--target takes a target triple".into())
                        })?;
                        targets.push(triple.to_string());
                        i += 2;
                    }
                    other => {
                        return Err(Failure::Usage(format!("gates: unexpected `{other}`")));
                    }
                }
            }
            let repo = git::Repo::open(Path::new("."))?;
            let work = repo.work_tree()?.to_path_buf();
            let outcome = gates::run(&work, &targets).map_err(Failure::Run)?;
            print!("{}", outcome.render(&work));
            if outcome.passed() {
                Ok(())
            } else {
                Err(Failure::Silent)
            }
        }
        // The detached half of `ship`'s backups (`backup::spawn`); not for agents to type.
        ["backup-push", remote, head] => {
            let repo = git::Repo::open(Path::new("."))?;
            backup::push(&repo, remote, head).map_err(Failure::Run)
        }
        ["hook", ..] => githook::run_cli(&words[1..]).map_err(|stop| match stop {
            hooks::Stop::Usage(m) => Failure::Usage(m),
            hooks::Stop::Refused(m) => Failure::Run(m),
        }),
        ["workflow", "root", "init", ..] => {
            hooks::root_init_cli(&words[3..]).map_err(|stop| match stop {
                hooks::Stop::Usage(m) => Failure::Usage(m),
                hooks::Stop::Refused(m) => Failure::Run(m),
            })
        }
        _ => forward(args),
    }
}

/// A command this program does not have goes to the earlier program where one is installed
/// beside it as `susi-tools-legacy` (or named by `SUSI_TOOLS_LEGACY`): the Windows host, until its
/// agents need none of that program's commands. Without it, the command is unknown.
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

/// Landing work keeps a claim alive; a claim on a task this ship closed is released.
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
    // in Susi Tools, on Susi's main.
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
    fn any_other_first_git_is_left_to_path() {
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

# susi-tools

Susi Tools: the development tooling of the Susi AI OS ecosystem. Apache-2.0.

Susi is the AI OS product: the private, licensed commercial apps (repository
`susi`, which was `susi-commercial-apps` until 2026-11-22). It is the primary. Two public derivatives come from it, both Apache-2.0, and the three
move together:

- **Susi Core** (repository `susi-core`, public): the local-first AI OS socket.
- **Susi Tools** (this repository, `susi-tools`): everything used only while developing,
  testing and releasing Susi. Nothing here ships inside a production program.

How the three repositories fit together, and what is finished and what is
not: [`ECOSYSTEM.md`](ECOSYSTEM.md). The page lives here, and Susi and Susi Core
point to it.

## Rules of this repository

- **Apache-2.0, and nothing secret.** No credentials, keys, licence material or
  private records, in any file or in the history.
- **It stands alone.** It depends on neither the private product nor Susi Core
  (decided 2026-11-22), so anyone can build and use it.
- **Development code and production code stay apart.** A production program
  never contains development tooling; the tooling lives here.
- **No dependency on a hosting service.** Git is the only transport. Hosted
  copies of this repository are write-only backups; nothing is fetched from
  them and no hosted feature is used.
- **Every push passes the public firewall.** This repository's
  `.githooks/pre-push` hands each push to `susi-tools hook pre-push`, which
  runs the firewall Susi keeps. Once per clone: `git config core.hooksPath
  .githooks` and `git config susi.publicFirewall <path to Susi's
  deploy/public-firewall/firewall.sh>`; without the second, every push is
  refused.

## Status

Founded on 2026-11-22 with a clean history. What it holds so far:

- **`susi-tools` 0.2, the development program, rebuilt small** (`src/`). It uses
  nothing but the Rust standard library and the `git` program, and it never
  leaves a file of its own in a checkout: what it changes goes through Git.
  On Windows it starts the real `git.exe` behind Git for Windows' `cmd`
  launcher, found once per run (`src/git_program.rs`), which spares the
  launcher's second process on every call; elsewhere, plain `git`.
  - **Reads:** `board list`, `tasks list` (the open tasks on the remote's
    `main` and their claims, fetched first; until 2026-11-23 it read the
    checkout's own `main`, which can be far behind).
  - **Session start:** `workflow check` fast-forwards a clean checkout to the
    remote's `main`, lists requests addressed to you that you have not
    answered, and says what you hold; `board check` does the middle part.
  - **Writes:** `board post` (the next free number, naming the item before;
    two posters never share a number), `tasks start` (adds the task's record
    under a number never used before, history included, commits and ships
    it, and claims the task), `tasks claim | release` (the three claim refs in
    one atomic push, each guarded by the value it was read at; refused when
    another agent holds the task, a live claim of another agent covers a
    scope, you already hold another live claim, or a dependency is still
    open; claiming a task you hold again renews it with the scopes given),
    `tasks close` (runs the acceptance and, only if it passes, commits the
    record's removal) and `ship` (committed work only, a fast-forward of
    `main`, then Landed on the Board; renews your claim, or releases it when
    the ship closed its task). A clone that names backups (`git config --add
    susi.backup <url>`, once per backup) has each ship start a detached push of
    `main`, release tags, agents' published work, the Board and the finished
    task records to every one of them, never forced and never claims; the ship
    does not wait for it, and each outcome is a line of `.git/susi-backup.log`.
    A public repository (its `.susi-repo-role` says `public-…`: Susi Core,
    Susi Tools) is pushed only through its firewall: the pre-push guard runs
    the script Susi keeps, named by `git config susi.publicFirewall <path>`, on
    every push, as a server's pre-receive would, and refuses the push when none
    is named; its backups carry `main` only (a signed release tag is published by
    hand: the public copies never receive an unsigned one).
    `tasks finish` does close and ship in one
    command. The separate `tasks add` and `tasks renew` left on 2026-11-23:
    nothing called them (`tasks start` adds, a ship renews), and so did the
    earlier program's spellings `workflow board …` and `workflow ship`. A task of
    Susi's queue (which holds the ecosystem's tasks and the Board) can claim
    files here or in Susi Core: the claim is kept on the repository whose files
    it covers, and a ship there renews it while the task is open in Susi.
  - **The Git hooks of Susi and Susi Core** (`src/githook.rs`, since
    2026-11-23): each repository's `.githooks/<stage>` is one line that runs
    `susi-tools hook <stage>`, so the rules are written once, here, instead of
    the 1,400 lines of shell the two repositories used to copy between them.
    `pre-commit` checks the agent's own root and a fresh checkout, then the
    staged change (no new blob of 1 MiB or more, no secret-shaped line, no
    added `dbg!(`, identical `CLAUDE.md` and `GEMINI.md`, formatted Rust);
    `commit-msg` wants a subject that says what and a `Task:` trailer naming a
    task recorded or claimed on that repository, whose claim is yours and
    covers every staged file, and a reuse decision for a sized task;
    `pre-push` lets through only `main` (forward only), `refs/agents/<name>`,
    `vX.Y.Z` tags (created once), the task queue's blobs and Board items
    (created once); `post-checkout` lists finished worktrees after `git
    worktree add`. The staged files are the ones being committed: under `git
    commit -a` or `git commit <path>`, those of the index Git names for the
    hook (until 2026-11-23 the hooks read the repository's own index there,
    and missed them). A repository's hook adds what it needs: `--worktrees` (no
    commit in a primary checkout or straight on `main`), `--isolation` (the
    agent's own `SUSI_HOME`, build target and scratch, outside every checkout)
    and `--token-prefix <prefix>` (its own licence tokens count as secrets).
    The public repositories' boundary, that nothing of the private product
    enters them, is enforced at every push by the firewall Susi keeps (`git
    config susi.publicFirewall`, below), so this public program never spells
    out the product's markers.
    The rules run only as these stages: the separate `workflow root show`,
    `fresh`, `reuse` and `guard` checks the hooks called before 2026-11-23 left
    on that day. `workflow root init --agent <name> [dir]` marks an agent's
    root, the folder the root rule looks for.
  - **No release command since 2026-11-22.** It cut Susi Core's releases
    (v0.21.72 was its first and last), and Susi Core has had no program and cut
    no releases since that day; it left with them, and history keeps it.
  - **The harvest audit:** `local-harvest audit [--range <A..B>]`, a
    second program built here, which Susi's release gate calls by that name. The same
    checks and messages as the earlier program's audit (Susi's
    `tools/core-xtask`, commit `5404fd12`): each manifest's upstream blobs and
    evidence, the Apache-2.0 4(a) to 4(d) notices, the retired records and the
    `Harvest:` citations of the range. It keeps one `git cat-file --batch`
    running and walks cached trees instead of starting a process per question:
    on Susi's ledger (14 manifests, 46 retired records, 51 citations over 3,026
    commits) it gives the earlier program's answer to the character in 0.8 s
    warm with 12 git processes, where the earlier one took 4.8 s with 89.
    `local-harvest notices [--check]` (since 2026-11-23) writes the
    `THIRD_PARTY_NOTICES` the audit expects, as the earlier program did and as
    Susi's notices file, release gate and this audit tell people to run, or
    with `--check` only says whether it is current; on Susi's ledger at
    `d709cc59` (13 manifests) it writes Susi's file byte for byte.
    `local-harvest`'s other commands still go to the earlier program, installed
    beside it as `local-harvest-legacy` (on Linux, where it is not installed,
    they are refused).
  - **Same formats as the earlier program**, so both read each other's work.
    Checked on 2026-11-22 on Susi's live Board and queue: the earlier program
    read this program's claim (holder, lease, scopes), listed its Board
    items with no findings, and both refused a second claim by one agent.
    Timings there: `workflow check` 1.35 s (earlier program 16.5 s), a claim
    with its Board item 3 s, a ship 3 s.
  - **Tests:** 67, on made-up repositories and records only, run on Linux before every ship:
    0.5 s for the tests on the laptop's CachyOS system (`cargo nextest run`, warm, 2026-11-23);
    from Windows through WSL2 (`scripts/linux-suite.ps1` below) about 2.5 s for the tests, 4.5
    to 4.8 s for the whole run with nothing changed and 7.4 to 9.0 s after a one-file edit
    (2026-11-22).
    On Windows the same suite takes about 40 s, because each write test drives the real
    commands end to end (a `tasks start` starts 49 `git` processes) and Windows starts processes
    almost one at a time. History: on Windows the whole suite took 59 s on 2026-11-22 (the write tests, each building scratch
    repositories), over the 10-second line: Windows started a `git` process in about 117 ms
    that day, and those tests start hundreds. Their made-up repositories are now built once
    for every test process and run (until 2026-11-22 each test process built its own and left it
    in the temporary folder) and run no background `git maintenance`: 58 -> 50 s. The rest is the
    git work of the `susi-tools` commands these tests drive (about 9 s for one test alone,
    40-50 s when all run side by side). Since 2026-11-22 a run fetches a remote once and reuses
    that view until it pushes (a landing ship fetched four times), and ship asks git for HEAD,
    main and where they meet in two calls instead of four: the suite 50 -> 48 s, `workflow
    check` on Susi 3.7-4.3 -> 2.9-3.5 s, an empty ship 1.3 -> 1.1 s. The suite stays far over
    the line on Windows while each process start costs 100 ms or more there; on Linux it takes
    under a second (above).
  - **Installed since 2026-11-22**, in place of the earlier program, on the
    Windows development host (`~/.cargo/bin/susi-tools`). The earlier program
    stays installed beside it as `susi-tools-legacy`: any command this one does
    not have yet (among them `workflow vision`, `workflow prune`, `workflow
    start` and `harvest`, whose audit is `local-harvest audit` now) is handed
    to it, with a one-line note. The Linux hosts have no earlier program:
    there, such a command is unknown. The hand-off ends when no contract names a command it
    alone has.
  - Build it with `cargo build --release --offline`. Test it before every ship with
    `pwsh scripts/linux-suite.ps1 -Agent <you> -Lint` (from Windows, through WSL2), or
    `cargo nextest run --offline --locked` on a Linux host; before installing a new build on a
    Windows host, run the suite there too (`cargo nextest run --offline --locked`), since code
    that behaves differently on Windows runs only its Linux branch on Linux.
- [`scripts/speed-report.ps1`](scripts/speed-report.ps1) times a checkout's
  everyday operations (build, the whole suite, lint, formatting, the optimised
  build and the development program's own commands) against the 10-second line
  the CEO set on 2026-11-22: "no more waiting more than 10s for anything - test,
  releases whatever it is". It measures warm only and needs PowerShell 7 and
  cargo. Verified on Windows against Susi on 2026-11-22, and on Linux against
  Susi Tools on 2026-11-23 (CachyOS: all nine operations within the line, the
  longest `cargo test` at 1.0 s).
- [`scripts/linux-suite.ps1`](scripts/linux-suite.ps1) runs a checkout's whole suite on
  Linux, as it stands, committed or not: from Windows it copies the checkout's files onto a
  WSL2 distribution's own disk (keeping their times, so only what changed is rebuilt), builds
  and tests there with one copy and target folder per agent, and with `-Lint` runs Clippy;
  on Linux it runs the suite in place. Added on 2026-11-22, when the CEO chose Linux for
  Susi Tools' suite; verified on Windows through WSL2 (Ubuntu 24.04) that day, not yet on
  CachyOS.

Each piece is checked against the rules above before it lands.
This repository is public since 2026-11-22: its write-only backups are at
`github.com/intellibitz/susi-tools` and `gitlab.com/intellibitz/susi-tools`,
`main` and signed release tags only. The repository of record is a private Cloudflare
Artifacts repository (since 2026-11-23); nothing is fetched from the hosted
copies, every ship pushes them in the background, and no
outside pull request, patch or commit is merged. Anyone may fork it under
Apache-2.0, which grants rights to the code, not to the Susi name or marks.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

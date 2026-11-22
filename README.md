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

## Status

Founded on 2026-11-22 with a clean history. What it holds so far:

- **`susi-tools` 0.2, the development program, rebuilt small** (`src/`). It uses
  nothing but the Rust standard library and the `git` program, and it never
  leaves a file of its own in a checkout: what it changes goes through Git.
  On Windows it starts the real `git.exe` behind Git for Windows' `cmd`
  launcher, found once per run (`src/git_program.rs`), which spares the
  launcher's second process on every call; elsewhere, plain `git`.
  - **Reads:** `board list`, `tasks list` (any repository, `--repo`).
  - **Session start:** `workflow check` fast-forwards a clean checkout to the
    remote's `main`, lists requests addressed to you that you have not
    answered, and says what you hold; `board check` does the middle part.
  - **Writes:** `board post` (the next free number, naming the item before;
    two posters never share a number), `tasks claim | renew | release` (the
    three claim refs in one atomic push, each guarded by the value it was read
    at; refused when another agent holds the task, a live claim of another
    agent covers a scope, you already hold another live claim, or a
    dependency is still open), `tasks add` (a number never used before,
    history included), `tasks close` (runs the acceptance and, only if it
    passes, commits the record's removal) and `ship` (committed work only, a
    fast-forward of `main`, then Landed on the Board; renews your claim, or
    releases it when the ship closed its task). `tasks start` does add,
    commit, ship and claim in one command, and `tasks finish` does close and
    ship, in the same formats. The earlier program's
    spellings `workflow board …` and `workflow ship` work too.
  - **No release command since 2026-11-22.** It cut Susi Core's releases
    (v0.21.72 was its first and last), and Susi Core has had no program and cut
    no releases since that day; it left with them, and history keeps it.
  - **The harvest audit:** `susi-tools harvest audit [--range <A..B>] [--json]`,
    and the same as `local-harvest audit` through a second program,
    `local-harvest`, which Susi's release gate calls by that name. The same
    checks and messages as the earlier program's audit (Susi's
    `tools/core-xtask`, commit `5404fd12`): each manifest's upstream blobs and
    evidence, the Apache-2.0 4(a) to 4(d) notices, the retired records and the
    `Harvest:` citations of the range. It keeps one `git cat-file --batch`
    running and walks cached trees instead of starting a process per question:
    on Susi's ledger (14 manifests, 46 retired records, 51 citations over 3,026
    commits) it gives the earlier program's answer to the character in 0.8 s
    warm with 12 git processes, where the earlier one took 4.8 s with 89.
    `local-harvest`'s other commands still go to the earlier program, installed
    beside it as `local-harvest-legacy`.
  - **Same formats as the earlier program**, so both read each other's work.
    Checked on 2026-11-22 on Susi's live Board and queue: the earlier program
    read this program's claim (holder, lease, scopes), listed its Board
    items with no findings, and both refused a second claim by one agent.
    Timings there: `workflow check` 1.35 s (earlier program 16.5 s), a claim
    with its Board item 3 s, a ship 3 s.
  - **Tests:** 53, on made-up repositories and records only, run on Linux before every ship
    (`scripts/linux-suite.ps1` below): about 2.5 s for the tests, 4.5 to 4.8 s for the whole
    run from Windows with nothing changed and 7.4 to 9.0 s after a one-file edit (2026-11-22).
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
    about 2 s.
  - **Installed since 2026-11-22**, in place of the earlier program, on the
    Windows development host (`~/.cargo/bin/susi-tools`). The earlier program
    stays installed beside it as `susi-tools-legacy`: any command this one does
    not have yet (among them `workflow vision`, `workflow prune`,
    `workflow start`, `harvest` other than `audit`, `releases`, and the checks the Git hooks
    call) is handed to it, with a one-line note, so every instruction and hook
    keeps working. Not yet: the CachyOS host still runs the earlier program;
    the hand-off ends when no contract or hook names a command it alone has.
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
  cargo. Verified on Windows against Susi on 2026-11-22; not yet run on Linux.
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
`main` and signed release tags only. The repository of record is the local
canonical Git server; nothing is fetched from the hosted copies, and no
outside pull request, patch or commit is merged. Anyone may fork it under
Apache-2.0, which grants rights to the code, not to the Susi name or marks.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

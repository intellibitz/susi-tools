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
    releases it when the ship closed its task). The earlier program's
    spellings `workflow board …` and `workflow ship` work too.
  - **Same formats as the earlier program**, so both read each other's work.
    Checked on 2026-11-22 on Susi's live Board and queue: the earlier program
    read this program's claim (holder, lease, scopes), listed its Board
    items with no findings, and both refused a second claim by one agent.
    Timings there: `workflow check` 1.35 s (earlier program 16.5 s), a claim
    with its Board item 3 s, a ship 3 s.
  - **Tests:** 25, on made-up repositories only. On Linux they take 0.7 s; on
    Windows about 15 s, because Windows starts the many short `git`
    processes slowly when the tests run side by side.
  - **Installed since 2026-11-22**, in place of the earlier program, on the
    Windows development host (`~/.cargo/bin/susi-tools`). The earlier program
    stays installed beside it as `susi-tools-legacy`: any command this one does
    not have yet (among them `workflow vision`, `workflow prune`,
    `workflow start`, `harvest`, `releases`, and the checks the Git hooks
    call) is handed to it, with a one-line note, so every instruction and hook
    keeps working. Not yet: the CachyOS host still runs the earlier program;
    the hand-off ends when no contract or hook names a command it alone has.
  - Build it with `cargo build --release --offline`; test it with
    `cargo test --offline`.- [`scripts/speed-report.ps1`](scripts/speed-report.ps1) times a checkout's
  everyday operations (build, the whole suite, lint, formatting, the optimised
  build and the development program's own commands) against the 10-second line
  the CEO set on 2026-11-22: "no more waiting more than 10s for anything - test,
  releases whatever it is". It measures warm only and needs PowerShell 7 and
  cargo. Verified on Windows against Susi on 2026-11-22; not yet run on Linux.

Each piece is checked against the rules above before it lands.
This repository is public since 2026-11-22: its write-only backups are at
`github.com/intellibitz/susi-tools` and `gitlab.com/intellibitz/susi-tools`,
`main` and signed release tags only. The repository of record is the local
canonical Git server; nothing is fetched from the hosted copies, and no
outside pull request, patch or commit is merged. Anyone may fork it under
Apache-2.0, which grants rights to the code, not to the Susi name or marks.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

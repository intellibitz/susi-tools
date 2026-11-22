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

- **`susi-tools` 0.1, the development program, rebuilt small** (`src/`). Its
  first slice only reads: `susi-tools board list` and `susi-tools tasks list`
  show the Susi Board and the task queue of any repository (`--repo`, a bare
  repository, a checkout or a worktree). It uses nothing but the Rust standard
  library and the `git` program, and it writes nothing, inside a checkout or
  out (a test holds that). Checked on 2026-11-22 against the live Board and
  both queues: the same items, tasks and claims as the earlier program, in 0.2
  to 0.3 seconds where the earlier one took 1.4 to 23. Not yet: the commands
  that write (claim, release, post, ship, start a worktree, close a task), so
  it is not installed in place of the earlier program, whose source is still
  kept privately. Build it with `cargo build --release --offline`; test it
  with `cargo test --offline`.
- [`scripts/speed-report.ps1`](scripts/speed-report.ps1) times a checkout's
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

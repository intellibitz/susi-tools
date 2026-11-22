# susi-tools

Susi Tools: the development tooling of the Susi AI OS ecosystem. Apache-2.0.

Susi is the AI OS product: the private, licensed commercial apps (repository
`susi`, which was `susi-commercial-apps` until 2026-11-22). It is the primary. Two public derivatives come from it, both Apache-2.0, and the three
move together:

- **Susi Core** (repository `susi-core`, public): the local-first AI OS socket.
- **Susi Tools** (this repository, `susi-tools`): everything used only while developing,
  testing and releasing Susi. Nothing here ships inside a production program.

How the three repositories fit together, and what is finished and what is
not: [`ECOSYSTEM.md`](ECOSYSTEM.md), the same page in all three.

## Rules of this repository

- **Apache-2.0, and nothing secret.** No credentials, keys, licence material or
  private records, in any file or in the history.
- **It stands on Susi Core alone.** It never depends on the private product, so
  anyone can build it.
- **Development code and production code stay apart.** A production program
  never contains development tooling; the tooling lives here.
- **No dependency on a hosting service.** Git is the only transport. Hosted
  copies of this repository are write-only backups; nothing is fetched from
  them and no hosted feature is used.

## Status

Founded on 2026-11-22 with a clean history. It holds one tool so far:

- [`scripts/speed-report.ps1`](scripts/speed-report.ps1) times a checkout's
  everyday operations (build, the whole suite, lint, formatting, the optimised
  build and the development program's own commands) against the 10-second line
  the CEO set on 2026-11-22: "no more waiting more than 10s for anything - test,
  releases whatever it is". It measures warm only and needs PowerShell 7 and
  cargo. Verified on Windows against Susi on 2026-11-22; not yet run on Linux.

The development program `susi-tools` itself is still built from source kept
privately, and it moves here piece by piece, each piece checked against the
rules above before it lands. This repository is private on its backups until
the CEO says publish.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

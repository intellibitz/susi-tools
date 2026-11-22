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

Founded on 2026-11-22 with a clean history. It holds no tooling yet: the
development program `susi-tools` is still built from source kept privately, and
it moves here piece by piece, each piece checked against the rules above
before it lands. This repository is private on its backups until the CEO says
publish.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

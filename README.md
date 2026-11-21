# susi-tools

Development tooling for the Susi AI OS ecosystem. Apache-2.0.

Susi is built from three repositories that move together:

- **susi**: the Apache-2.0 core, a local-first AI OS socket.
- **susi-tools** (this repository): everything used only while developing,
  testing and releasing Susi. Nothing here ships inside a production program.
- **Susi Commercial Apps**: the licensed products built on the core.

## Rules of this repository

- **Apache-2.0, and nothing secret.** No credentials, keys, licence material or
  private records, in any file or in the history.
- **It stands on the Susi core alone.** It never depends on Susi Commercial
  Apps, so anyone can build it.
- **Development code and production code stay apart.** A production program
  never contains development tooling; the tooling lives here.
- **No dependency on a hosting service.** Git is the only transport. Hosted
  copies of this repository are write-only backups; nothing is fetched from
  them and no hosted feature is used.

## Status

Founded on 2026-11-22 with a clean history. The tooling is being moved in piece
by piece, and each piece is checked against the rules above before it lands.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

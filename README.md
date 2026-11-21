# susi-tools

Susi Tools: the development tooling of the Susi AI OS ecosystem. Apache-2.0.

Susi is the AI OS product: the private, licensed Susi Commercial Apps. It is the
primary. Two public derivatives come from it, both Apache-2.0, and the three
move together:

- **Susi Core**: the local-first AI OS socket.
- **Susi Tools** (this repository): everything used only while developing,
  testing and releasing Susi. Nothing here ships inside a production program.

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

Founded on 2026-11-22 with a clean history. The tooling is being moved in piece
by piece, and each piece is checked against the rules above before it lands.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

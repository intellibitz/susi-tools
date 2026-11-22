# The Susi AI OS ecosystem

This page is the same, byte for byte, in all three repositories of the Susi AI
OS ecosystem. A test in Susi and in Susi Core holds each copy to the same
fingerprint, so the page changes in all three at once or not at all.

## Three repositories

| Name | Repository | What it is | Licence | Public |
|---|---|---|---|---|
| **Susi** | `susi` | The product: licensed commercial applications. It is the primary; the other two derive from it. | Proprietary | No |
| **Susi Core** | `susi-core` | The local-first AI OS socket and its three pillars: GAWD (agent coordination, A2A, ACP and a parallel harness), GEMI (inference routing) and GMCP (MCP and governed tools). Linux x86_64. | Apache-2.0 | Yes |
| **Susi Tools** | `susi-tools` | Everything used only while developing, testing and releasing Susi. None of it ships inside a production program. | Apache-2.0 | Yes |

## How they relate

- **One direction.** Dependencies flow only from Susi to Susi Core and Susi
  Tools. Neither public repository needs the product to build or run.
- **Nothing secret in public.** Susi Core and Susi Tools carry no credential,
  key, licence material, private record or proprietary source, in any file or
  in their history.
- **Built by agents.** The software is written, tested and released by AI
  agents. Each repository's `AGENTS.md` is its contract.
- **One source of truth.** Each repository lives on a local Git server. GitHub
  and GitLab hold write-only backup copies, refreshed every ten minutes;
  nothing is ever fetched from them. The public copies of Susi Core are at
  `github.com/intellibitz/susi-core` and `gitlab.com/intellibitz/susi-core`,
  and those of Susi Tools (public since 2026-11-22) at
  `github.com/intellibitz/susi-tools` and `gitlab.com/intellibitz/susi-tools`.
  Public copies carry `main` and signed release tags only.
- **Published one way.** No outside pull request, patch or commit is merged.
  Anyone may fork the public repositories under Apache-2.0, which grants rights
  to the code, not to the Susi name or marks.

## Where it stands (2026-11-22)

Finished:

- The three repositories exist and move together under the names above.
  Until 2026-11-22 Susi Core's repository was called `susi` and Susi's was
  called `susi-commercial-apps`; older text uses those names. Susi Core's old
  public address, `github.com/intellibitz/susi`, now belongs to Susi's private
  backup and no longer leads to Susi Core: use `intellibitz/susi-core`.
- Susi Core builds and passes its tests offline on Linux. It has no line
  limit: its size is measured and reported.
- The offline proof passes: with every proxy dead and Git limited to local
  files, all three repositories clone from the local server, Susi builds,
  passes its tests and makes and rolls back a test release, Susi Core passes
  its Linux checks, the transfer between Windows and CachyOS goes round
  without changing anything, and the backup sync fails cleanly.

Not finished, said plainly:

- **The program names.** Susi Core's program is still called `susi` and
  Susi's is still called `susi-commercial`; they are renamed in a second step.
- **Susi is being rebuilt.** Since 2026-11-22 Susi holds only its licence and
  vault core. Its applications are being rebuilt, so a capability Susi Core
  points to the product for has no implementation there today.
- **Susi Tools holds one tool.** Its first, since 2026-11-22, is a speed
  report that times a checkout's everyday operations. The development program
  and Susi Core's development-only code (`scripts/`, `toolchain/`, `fuzz/`)
  move there piece by piece.
- **Speed.** Since 2026-11-22 nothing an agent waits on may take more than 10
  seconds, warm. Not met yet: Susi Core's Linux checks, Susi's whole test
  suite and its optimised build take longer. Each is measured, dated and its
  fix recorded in the repository it belongs to.
- **The latest compiler.** Rust 1.100.0 is installed but not yet used: two
  low-level parts (Susi's Windows start-up code and Susi Core's
  standard-library builder) are checked only against Rust 1.99.0 and must be
  re-checked before the compiler moves.
- **CachyOS.** The Linux side of the dual-boot setup needs the new sync script
  run there once, by hand; it then renames the old layout to the new names and
  imports. Until then the old script there stops at the new format without
  changing anything. Tested in a Linux copy, not yet on the real host.

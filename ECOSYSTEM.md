# The Susi AI OS ecosystem

This page is the same, byte for byte, in all three repositories of the Susi AI
OS ecosystem. A test in Susi and in Susi Core holds each copy to the same
fingerprint, so the page changes in all three at once or not at all.

## Three repositories

| Name | Repository | What it is | Licence | Public |
|---|---|---|---|---|
| **Susi** | `susi` | The product: licensed commercial applications. It is the primary; the other two derive from it. | Proprietary | No |
| **Susi Core** | `susi-core` | The local-first AI OS socket and its three pillars: GAWD (agent coordination, A2A, ACP and a parallel harness), GEMI (inference routing) and GMCP (MCP and governed tools). Linux x86_64. | Apache-2.0 | Yes |
| **Susi Tools** | `susi-tools` | Everything used only while developing, testing and releasing Susi. None of it ships inside a production program. | Apache-2.0 | Not yet |

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
  `github.com/intellibitz/susi-core` and `gitlab.com/intellibitz/susi-core`.
- **Published one way.** No outside pull request, patch or commit is merged.
  Anyone may fork the public repositories under Apache-2.0, which grants rights
  to the code, not to the Susi name or marks.

## Where it stands (2026-11-22)

Finished:

- The three repositories exist and move together under the names above.
  Until 2026-11-22 Susi Core's repository was called `susi` and Susi's was
  called `susi-commercial-apps`; older text uses those names, and the old
  public address of Susi Core redirects to the new one.
- Susi Core builds and passes its tests offline on Linux. It has no line
  limit: its size is measured and reported.

Not finished, said plainly:

- **The program names.** Susi Core's program is still called `susi` and
  Susi's is still called `susi-commercial`; they are renamed in a second step.
- **Susi is being rebuilt.** Since 2026-11-22 Susi holds only its licence and
  vault core. Its applications are being rebuilt, so a capability Susi Core
  points to the product for has no implementation there today.
- **Susi Tools is empty.** It holds no tooling yet. The development tooling and
  Susi Core's development-only code (`scripts/`, `toolchain/`, `fuzz/`) move
  there piece by piece.
- **The offline proof.** The test that clones, builds, tests and releases all
  three repositories with GitHub, GitLab and the internet cut off is being
  rewritten for three repositories.

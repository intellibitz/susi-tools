# The Susi AI OS ecosystem

This page is kept here, in Susi Tools, for all three repositories of the Susi
AI OS ecosystem; Susi and Susi Core point to it. Until 2026-11-22 each
repository kept a copy.

## Three repositories

| Name | Repository | What it is | Licence | Public |
|---|---|---|---|---|
| **Susi** | `susi` | The product: secure autonomy for a company's AI agents (the agent vault first, then the harness), sold under licence. It is the primary; the other two derive from it. | Proprietary | No |
| **Susi Core** | `susi-core` | The small libraries Susi builds on, and nothing else: home and path resolution, a typed error, and the wrappers of serde, serde_json, sha2 and getrandom. No program. | Apache-2.0 | Yes |
| **Susi Tools** | `susi-tools` | Everything used only while developing, testing and releasing Susi. None of it ships inside a production program. | Apache-2.0 | Yes |

## How they relate

- **One direction.** Dependencies flow only from Susi to Susi Core and Susi
  Tools. Neither public repository needs the product, and neither needs the
  other: Susi is built with Susi Tools and builds on Susi Core's libraries.
- **Everything traces to Susi** (decided 2026-11-22). Susi's features are what
  is sold; Susi Core holds only what those features use, and Susi Tools only
  what agents use to build Susi. What traces to nothing is removed.
- **Nothing secret in public.** Susi Core and Susi Tools carry no credential,
  key, licence material, private record or proprietary source, in any file or
  in their history.
- **Built by agents.** The software is written, tested and released by AI
  agents. Each repository's `AGENTS.md` is its contract.
- **One source of truth, on Cloudflare.** Since 2026-11-23 Cloudflare is the
  primary cloud and each repository's source of truth is a private Cloudflare
  Artifacts repository, so agents work from any machine. GitHub and GitLab are
  storage and branding only: Susi's private backups and the public copies of
  Susi Core (`github.com/intellibitz/susi-core`,
  `gitlab.com/intellibitz/susi-core`) and Susi Tools
  (`github.com/intellibitz/susi-tools`, `gitlab.com/intellibitz/susi-tools`).
  Every ship pushes them in the background; nothing is fetched from them.
  Public copies carry `main` and release tags only, and every push of a public
  repository passes the firewall Susi keeps.
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
- Susi Core is libraries only. Its program (an MCP server and a local daemon)
  left on 2026-11-22 because no feature of Susi used it; its last release,
  v0.21.72, stays published. With it went Susi Tools' release command, which
  cut Core's releases.
- Development moved to Linux, then off the laptop: since 2026-11-23 the
  repositories live on Cloudflare and no machine has to stay on. The laptop
  (Windows, CachyOS, openSUSE Tumbleweed) keeps each repository's full
  history, read-only; the keys and the company vault still come from it and
  from Google Drive, and move next. On Linux an agent's loop is
  sub-second (session check, task start, commit, ship), Susi's whole suite
  takes 6 to 7 seconds, Susi Core's gates about 3 and Susi Tools' suite under 1.

Not finished, said plainly:

- **The program names.** Susi's program is still called `susi-commercial`;
  it becomes `susi` in a later step. Susi Core's root package keeps the name
  `susi` and holds no program.
- **Susi is being rebuilt.** Since 2026-11-22 Susi holds only its licence and
  vault core, and builds the agent vault into a product another company can
  install.
- **Susi Tools is being filled.** It holds the development program, which hands
  every command it does not have yet to the earlier one, still built from
  source kept privately (on Linux that earlier program is not installed). It
  also holds a speed report and a runner for a checkout's suite on Linux.
  Since 2026-11-23 it holds the rules of Susi's and Susi Core's Git hooks
  (`susi-tools hook <stage>`); their hook files are one-line calls to it.
- **Speed.** Nothing an agent waits on may take more than 10 seconds, warm.
  Nothing measured is over the line on Linux today: Susi's release build, which
  took about 20 s, takes 4.0 to 4.8 s after a one-file change and 0.12 s with
  nothing changed (Susi's `docs/speed.md`, CachyOS, 2026-11-23).
- **The latest compiler.** Susi and Susi Core use Rust 1.99.0; Susi's Windows
  start-up code is checked only against it and must be re-checked before the
  compiler moves. Susi Tools names no compiler version.

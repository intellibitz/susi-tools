# The Susi AI OS ecosystem

This page is kept here, in Susi Tools, for all three repositories of the Susi
AI OS ecosystem; Susi and Susi Core point to it.

## Three repositories

| Name | Repository | What it is | Licence | Public |
|---|---|---|---|---|
| **Susi** | `susi` | The product: secure autonomy for a company's AI agents (the agent vault first; more products follow), sold under licence. It is the primary; the other two derive from it. | Proprietary | No |
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
  agents, from any machine. Susi's `AGENTS.md` is the contract for work in all
  three; Susi Core's own `AGENTS.md` adds what is Core's, and Susi Tools' rules
  are in its README.
- **One source of truth, on Cloudflare.** Cloudflare is the primary cloud,
  and each repository's source of truth is a private Cloudflare Artifacts
  repository, so agents work from any machine. GitHub and GitLab are
  storage and branding only: Susi's private backups and the public copies of
  Susi Core (`github.com/intellibitz/susi-core`,
  `gitlab.com/intellibitz/susi-core`) and Susi Tools
  (`github.com/intellibitz/susi-tools`, `gitlab.com/intellibitz/susi-tools`).
  Every ship pushes them in the background; nothing is fetched from them.
  Public copies carry `main` (pushed by every ship) and signed release tags
  (published by hand) only, and every push of a public repository passes the
  firewall Susi keeps.
- **Published one way.** No outside pull request, patch or commit is merged.
  Anyone may fork the public repositories under Apache-2.0, which grants rights
  to the code, not to the Susi name or marks.

## Where it stands (2026-11-23)

Finished:

- The three repositories move together under the names above. Older text,
  and the folders agents' clones sit in, still use the earlier names: `susi`
  for Susi Core and `susi-commercial-apps` for Susi. The public address
  `github.com/intellibitz/susi` belongs to Susi's private backup, not to Susi
  Core: Susi Core's public copy is `intellibitz/susi-core`.
- Susi Core is libraries only; it has no program, and its last release,
  v0.21.72, stays published. Susi's releases are made by Susi's own release
  program; Susi Tools has no release command.
- Development is Linux-first and runs from the repositories on Cloudflare, so
  no machine has to stay on. The laptop (Windows, CachyOS, openSUSE
  Tumbleweed) keeps each repository's full history, read-only; what still
  ties Susi to it is in Susi's `docs/infrastructure.md`. On Linux an agent's
  loop is sub-second (session check, task start, commit, ship), Susi's whole
  suite takes 6 to 7 seconds, Susi Core's gates about 3 and Susi Tools' suite
  under 1.

Not finished, said plainly:

- **The program names.** Susi's program is still called `susi-commercial`;
  it becomes `susi` in a later step. Susi Core's root package keeps the name
  `susi` and holds no program.
- **One product sells so far.** Susi sells the agent vault; which bricks are
  built to sell next is the CEO's to decide (Susi's `AGENTS.md`, "What Susi
  sells").
- **The Windows host has not moved yet.** Susi Tools holds the development
  program, the rules of every repository's Git hooks (`susi-tools hook
  <stage>`; the hook files are one-line calls to it) and the harvest audit.
  On the laptop's Windows system the earlier development program, built from
  source kept privately, still answers the commands Susi Tools lacks, and two
  PowerShell scripts (a speed report, and a runner for a checkout's suite
  under WSL2) serve that host; the Linux hosts have no earlier program. They
  go when the agents on the Windows host no longer use them.
- **Speed.** Nothing an agent waits on may take more than 10 seconds, warm.
  Nothing measured is over the line on Linux today: Susi's release build, which
  took about 20 s, takes 4.0 to 4.8 s after a one-file change and 0.12 s with
  nothing changed (Susi's `docs/speed.md`, CachyOS, 2026-11-23).
- **The latest compiler.** Susi and Susi Core use Rust 1.99.0; Susi's Windows
  start-up code is checked only against it and must be re-checked before the
  compiler moves. Susi Tools names no compiler version.

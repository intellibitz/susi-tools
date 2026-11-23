# The Susi AI OS ecosystem

This page is kept here, in Susi Tools, for both repositories of the Susi AI OS
ecosystem; Susi points to it.

## Two repositories

| Name | Repository | What it is | Licence | Public |
|---|---|---|---|---|
| **Susi** | `susi` | The product: secure autonomy for a company's AI agents (the agent vault first; more products follow), sold under licence. It is the primary. | Proprietary | No |
| **Susi Tools** | `susi-tools` | Everything used only while developing, testing and releasing Susi. None of it ships inside a production program. | Apache-2.0 | Yes |

**Susi Core** (`susi-core`), the public library repository, was retired on
2026-11-23 by the CEO's decision, once Susi built on none of its libraries. It
takes no further changes; its code stays available under Apache-2.0 in its
history, and its README names the last commit holding it.

## How they relate

- **One direction.** Susi is built with Susi Tools; Susi Tools needs nothing
  of Susi.
- **Everything traces to Susi** (decided 2026-11-22). Susi's features are what
  is sold, and Susi Tools holds only what agents use to build Susi. What traces
  to nothing is removed.
- **Nothing secret in public.** Susi Tools carries no credential, key, licence
  material, private record or proprietary source, in any file or in its
  history, and neither did Susi Core.
- **Built by agents.** The software is written, tested and released by AI
  agents, from any machine. Susi's `AGENTS.md` is the contract for work in
  both; Susi Tools' own rules are in its README.
- **One source of truth, on Cloudflare.** Cloudflare is the primary cloud,
  and each repository's source of truth is a private Cloudflare Artifacts
  repository, so agents work from any machine. GitHub and GitLab are
  storage and branding only: Susi's private backups and the public copies of
  Susi Tools (`github.com/intellibitz/susi-tools`,
  `gitlab.com/intellibitz/susi-tools`). Every ship pushes them in the
  background; nothing is fetched from them. Public copies carry `main`
  (pushed by every ship) and signed release tags (published by hand) only,
  and every push of a public repository passes the firewall Susi keeps.
- **Published one way.** No outside pull request, patch or commit is merged.
  Anyone may fork Susi Tools under Apache-2.0, which grants rights to the
  code, not to the Susi name or marks.

## Where it stands (2026-11-23)

Finished:

- Susi depends on no public library repository: its crates come from
  crates.io through the company's crate mirror. Susi's releases are made by
  Susi's own release program; Susi Tools has no release command.
- Development is Linux-first and runs from the repositories on Cloudflare, so
  no machine has to stay on; what still ties Susi to the laptop is in Susi's
  `docs/infrastructure.md`. On Linux an agent's loop is sub-second (session
  check, task start, commit, ship), Susi's whole suite takes about 6 seconds
  and Susi Tools' under 1.

Not finished, said plainly:

- **The names.** Susi's program is still called `susi-commercial` and agents'
  clones of Susi sit in folders named `susi-commercial-apps`; they are renamed
  in one window, on the CEO's word.
- **One product sells so far.** Susi sells the agent vault; which bricks are
  built to sell next is the CEO's to decide (Susi's `AGENTS.md`, "What Susi
  sells").
- **The Windows host has not moved yet.** On the laptop's Windows system the
  earlier development program, built from source kept privately, still
  answers the commands Susi Tools lacks, and two PowerShell scripts (a speed
  report, and a runner for a checkout's suite under WSL2) serve that host;
  the Linux hosts have no earlier program. They go when the agents on the
  Windows host no longer use them.
- **Speed.** Nothing an agent waits on may take more than 10 seconds, warm;
  nothing measured on Linux is over the line (Susi's `docs/speed.md`).
- **The latest compiler.** Susi uses Rust 1.99.0; its Windows start-up code is
  checked only against it and must be re-checked before the compiler moves.
  Susi Tools names no compiler version.

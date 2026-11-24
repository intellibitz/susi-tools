# susi-tools

Susi Tools: the development tooling of the Susi AI OS ecosystem. Apache-2.0.

Susi is the AI OS product: the private, licensed commercial apps (repository
`susi`; agents' clones of it still sit in folders named `susi-commercial-apps`).
It is the primary. Susi Tools (this repository, `susi-tools`, public,
Apache-2.0) holds everything used only while developing, testing and releasing
Susi; nothing here ships inside a production program. Susi Core, the public
library repository, was retired on 2026-11-23.

How the repositories fit together, and what is finished and what is not:
[`ECOSYSTEM.md`](ECOSYSTEM.md). The page lives here, and Susi points to it. Work here follows Susi's contract (its `AGENTS.md`: the loop, the
Board, tasks and claims); the rules below are this repository's own.

## Rules of this repository

- **Apache-2.0, and nothing secret.** No credentials, keys, licence material or
  private records, in any file or in the history.
- **It stands alone.** It depends on nothing of the private product (decided
  2026-11-22), so anyone can build and use it.
- **Development code and production code stay apart.** A production program
  never contains development tooling; the tooling lives here.
- **No dependency on a hosting service.** Git is the only transport. Hosted
  copies of this repository are write-only backups; nothing is fetched from
  them and no hosted feature is used.
- **Every push passes the public firewall.** This repository's
  `.githooks/pre-push` hands each push to `susi-tools hook pre-push`, which
  runs the firewall Susi keeps. Once per clone: `git config core.hooksPath
  .githooks` and `git config susi.publicFirewall <path to Susi's
  deploy/public-firewall/firewall.sh>`; without the second, every push is
  refused.

## What it holds

- **`susi-tools` 0.2, the development program** (`src/`). It uses nothing but
  the Rust standard library and the `git` program, and it never leaves a file
  of its own in a checkout: what it changes goes through Git. On Windows it
  starts the real `git.exe` behind Git for Windows' `cmd` launcher, found once
  per run (`src/git_program.rs`), which spares the launcher's second process on
  every call; elsewhere, plain `git`. `susi-tools --help` lists its commands.
  - **Session start:** `workflow check` fast-forwards a clean checkout to the
    remote's `main`, lists requests addressed to you that you have not
    answered, prints every Board item addressed to you (any kind, its `to` your
    name or token) that this clone has not shown you yet, under `for you since
    your last item:` (each is shown once, whatever you post in between: the
    clone keeps the last number shown, `susi.boardshown-<you>`; the first time
    it counts from your last Landed), and says what you hold; `board check`
    does the Board part.
  - **The lead** (a role, not a vendor: whichever agent the CEO is talking
    to leads that session; Susi's `docs/bricks.md`): `lead` prints its
    bearings (who leads, the brick map on main, `.agents/bricks.json`, with
    each brick's rung, next step and tasks, the foundation's tasks (the
    map's `foundation`: for the whole, no one brick's), what waits on the
    CEO, the visions placed nowhere and the open tasks the map names
    nowhere, then the Board for you); `lead claim [--hours <1-24>] [--take]` takes the lead,
    eight hours unless told, as one compare-and-swap ref on the Board's
    repository (`refs/lead/current`), refused while another agent's is live
    unless `--take`, which tells that agent on the Board; `lead release`
    gives it up.
  - **Reads:** `board list`, `tasks list` (the open tasks on the remote's
    `main`, each with the vision it serves, and their claims, fetched
    first), `tasks brief <id>` (the words to give an agent so it can do a
    task from nothing: the vision in the CEO's words, the goal, the paths it
    may change, what to read first, the check, and the loop that lands it)
    and `tasks next` (the brief of the first open task for you: meant for you
    or for anyone, unclaimed or its claim lapsed, its dependencies closed).
  - **Writes:** `board post` (the next free number, naming the item before;
    two posters never share a number), `tasks add` (adds a task's record
    under a number never used before, history included, and commits and
    ships it for an agent to pick up: the vision it serves, which must be on
    Susi's `main` (`--vision V-<n>`, Susi's `.agents/visions`; a task without
    one is refused), the rung of the brick ladder it moves (`--rung`), the
    paths it may change (`--scope`), what to read first (`--read`) and whom
    it is meant for (`--for`); Susi's `docs/agents-together.md`), `tasks
    start` (the same, then claims the task), `tasks claim | release` (the
    three claim refs in
    one atomic push, each guarded by the value it was read at; refused when
    another agent holds the task, a live claim of another agent covers a
    scope, you already hold another live claim, or a dependency is still
    open; claiming a task you hold again renews it with the scopes given;
    without `--scope`, the paths its record names),
    `tasks close` (runs the acceptance and, only if it passes, commits the
    record's removal), `tasks finish` (close, then ship) and `ship` (committed
    work only, a fast-forward of `main`, then Landed on the Board; then every
    Board item addressed to you not yet shown, as `workflow check` prints
    them; renews your claim, or releases it when
    the ship closed its task). A task of Susi's
    queue (which holds the ecosystem's tasks and the Board) can claim files
    here: the claim is kept on the repository whose files it covers, and a ship there renews it while the task is open in Susi.
  - **Backups:** a clone that names backups (`git config --add susi.backup
    <url>`, once per backup) has each ship start a detached push to every one
    of them, never forced and never claims; the ship does not wait for it, and
    each outcome is a line of `.git/susi-backup.log`. A private repository's
    backups carry `main`, release tags, agents' published work, the Board and
    the finished task records. A public repository's (its `.susi-repo-role`
    says `public-…`: Susi Tools) carry `main` only: a signed
    release tag is published by hand, so the public copies never receive an
    unsigned one.
  - **Speed:** `speed [--cold] [--json]` times a Cargo checkout's everyday
    operations (warm `cargo build` with nothing changed and after touching one
    source file, `cargo nextest run`, and with `--no-default-features` for the
    packages that have features, `cargo clippy --workspace --all-targets -- -D
    warnings`, `cargo fmt --all --check`, `git status`, `git fetch` and
    `susi-tools workflow check`) against the 10-second line, in plain Rust with
    no shell or PowerShell, restoring the touched file byte for byte even when
    a step fails. The suites and the lints are timed side by side, as `gates`
    runs them, so each time is an upper bound and `speed` itself stays inside
    the line (8.0 s on Susi, 2026-11-24; 17.7 s one after another). `--cold`
    also times a build after `cargo clean` in a throwaway target directory,
    reported apart. Replaces the retired `scripts/speed-report.ps1`.
  - **Landing:** a clone set with `git config susi.land host` (a cloud
    agent's, whose machine is too small for the gates inside the 10-second
    line: 4 cores took 37.6 s for Susi's, a company host 6.6 s) ships to
    `refs/agents/<agent>` instead of `main`, posts a request on the Board, and
    may claim the task its own unlanded work records. `land [--target
    <triple>]...`, run on a company host in a clone of its own (a timer may
    start it every minute; one run at a time), checks out each agent ref
    ahead of `main` in its own worktree, runs `gates` there, pushes what
    passes as `main` (a fast-forward, through the clone's hooks) and says so
    on the Board; work whose gates fail, or that `main` has moved past, stays
    where it is and its agent is told on the Board, once a commit (the CEO,
    2026-11-24: "Company host lands it").
  - **Gates:** `gates [--target <triple>]...` runs every gate before a ship in
    three lanes at once: the whole suite; the suite of the packages that have
    features, with `--no-default-features`, in its own target folder
    (`<target>/no-default-features`, so the two builds never overwrite each
    other's binaries); and the lints (`cargo fmt --check`, Clippy for both
    builds, and Clippy for every `--target` for both builds, with
    `SUSI_CROSS_CHECK=1`; without default features only for the packages that
    have features, since nothing else changes). It names each gate's time and the failing one's
    output, and says plainly when the whole run is over the 10-second line
    (the CEO, 2026-11-24: "a non negotiable mandate would be 10s"). On Susi:
    6.7 s, against 14.3 s one after another.
    It also reports each 12-or-more-line block newly added to a Rust source
    that repeats code elsewhere outside `tests/`, as `copies: path:line and
    path:line`; when there are none it prints `copies: none`. This report is
    advisory and does not change whether a gate passes.
  - **The public firewall:** a public repository is pushed only through the
    firewall Susi keeps, named by `git config susi.publicFirewall <path>`: its
    pre-push hook runs the script on every push, as a server's pre-receive
    would, and refuses the push when none is named. This public program never
    spells out the private product's markers.
  - **The Git hooks of Susi and Susi Tools** (`src/githook.rs`):
    each repository's `.githooks/<stage>` is one line that runs `susi-tools
    hook <stage>`, so the rules are written once, here. `pre-commit` checks the
    agent's own root and a fresh checkout, then the staged change (no new blob
    of 1 MiB or more, no secret-shaped line, no added `dbg!(`, identical
    `CLAUDE.md` and `GEMINI.md`, formatted Rust); `commit-msg` wants a subject
    that says what and a `Task:` trailer naming a task recorded or claimed on
    that repository, whose claim is yours and covers every staged file, and a
    reuse decision for a task of size `m` or `l`; `pre-push` lets through only
    `main` (forward only), `refs/agents/<name>`, `vX.Y.Z` tags (created once),
    the task queue's blobs and Board items (created once), and runs the public
    firewall for a public repository; `post-checkout` lists finished worktrees
    after `git worktree add`. The staged files are the ones being committed:
    under `git commit -a` or `git commit <path>`, those of the index Git names
    for the hook. A repository's hook adds what it needs: `--isolation` (the
    agent's own `SUSI_HOME`, build target and scratch, outside every checkout)
    and `--token-prefix <prefix>` (its own licence tokens count as secrets).
    Each refusal says why its rule exists and how to proceed. Susi Tools
    itself has only the pre-push hook.
  - **Agent roots:** `workflow root init --agent <name> [dir]` marks an agent's
    root (default `~/susi-agents/<name>`, under `HOME`, else `USERPROFILE`),
    the folder the root rule looks for.
  - **The earlier program, on the Windows host only.** That host still has the
    earlier development program installed beside this one as
    `susi-tools-legacy`: any command this one does not have (among them
    `workflow vision`, `workflow prune` and `workflow start`) is handed to it,
    with a one-line note. Board items, task records, claims and
    agent-root markers have the earlier program's formats, so both read each
    other's work. The Linux hosts have no earlier program: there, such a
    command is unknown. The hand-off ends when no agent on the Windows host
    needs a command it alone has.
- **`local-harvest`**, a second program built here, which Susi's release gate
  calls by that name. `audit [--range <A..B>]` checks the ledger of harvested
  Apache-2.0 code against Git: each manifest's upstream blobs and evidence,
  the Apache-2.0 4(a) to 4(d) notices, the retired records and the `Harvest:`
  citations of the range. In a shallow clone (every fresh clone of Susi from
  Cloudflare, whose copy starts at `17339d28`, such as a lander's) a retired
  record or correction naming a commit beyond its history is taken as recorded,
  and the summary says how many; a clone with the whole history checks them
  all (T-CLAUDE-10). `notices [--check]` writes the `THIRD_PARTY_NOTICES`
  the audit expects, or with `--check` only says whether it is current. It
  keeps one `git cat-file --batch` running and walks cached trees instead of
  starting a process per question: on Susi's ledger it took 0.8 s warm with 12
  git processes (2026-11-22). Its other commands go to the earlier program
  where it is installed as `local-harvest-legacy` (the Windows host); elsewhere
  they are refused.
- **Tests:** 75, on made-up repositories and records only, beside the code
  they test. Run them before every ship with `cargo nextest run --offline
  --locked`, then `cargo fmt --all --check` and `cargo clippy --all-targets
  --offline --locked -- -D warnings`: the suite takes about 1.2 s warm on the
  laptop's CachyOS system (2026-11-23). On Windows it takes about 40 s
  (2026-11-22), because each write test drives the real commands end to end
  and Windows starts processes almost one at a time; run it there too before
  installing a new build on a Windows host, since code that behaves
  differently on Windows runs only its Linux branch on Linux. Build with
  `cargo build --release --offline`.
- **One PowerShell script, for the Windows host.**
  [`scripts/linux-suite.ps1`](scripts/linux-suite.ps1) runs a checkout's whole
  suite on Linux as it stands, committed or not: from Windows it copies the
  checkout onto a WSL2 distribution's own disk (keeping file times, so only
  what changed is rebuilt) and builds and tests there, with one copy and target
  folder per agent, and with `-Lint` runs Clippy; on Linux it runs the same
  steps in place, which `cargo nextest run` does directly. It goes when the
  Windows host's agents no longer use it. The earlier
  `scripts/speed-report.ps1` was removed once `susi-tools speed` replaced it
  portably in Rust.

This repository is public: its write-only backups are at
`github.com/intellibitz/susi-tools` and `gitlab.com/intellibitz/susi-tools`.
The repository of record is a private Cloudflare Artifacts repository
(`susi/susi-tools`); nothing is fetched from the hosted copies, every ship
pushes them in the background, and no outside pull request, patch or commit is
merged. Anyone may fork it under Apache-2.0, which grants rights to the code,
not to the Susi name or marks.

## License

Apache License, Version 2.0. See [LICENSE](LICENSE) and [NOTICE](NOTICE).

# SPDX-License-Identifier: Apache-2.0
<#
.SYNOPSIS
    Run a checkout's whole test suite on Linux: from Windows through WSL2, or directly on a
    Linux host. The standard way to test Susi Tools before a ship.

.DESCRIPTION
    On the Windows development host a process start costs 60 to 170 ms and processes start
    almost one at a time, so Susi Tools' suite, whose write tests drive the real commands end to
    end, took about 40 s there on 2026-11-22; on Linux, on the same laptop, 2.5 to 4.2 s (Susi's
    docs/speed.md). This runs, in the checkout given, exactly as it stands (committed or not):

      1. the whole suite: cargo nextest run --offline --locked (cargo test without
         cargo-nextest);
      2. with -Lint, cargo clippy --all-targets --offline --locked -- -D warnings.

    From Windows it runs inside a WSL2 distribution (the default one, or -Distro). It copies the
    checkout's files (what `git ls-files` lists: tracked, and new ones not ignored; deleted ones
    left out) onto the distribution's own disk with their modification times, so cargo rebuilds
    only what changed, and builds and tests that copy: a one-file rebuild took 2.3 to 2.9 s there
    against 4.2 to 5.3 s reading the checkout through /mnt (2026-11-22), and the copy 0.6 s. Each
    agent has its own copy and target folder (~/susi-tools-linux/<agent>/), so agents never share
    build state. The distribution's compiler wrapper (RUSTC_WRAPPER, set to sccache on this host's
    distribution without sccache installed) is switched off for the run, and cargo works
    offline. On Linux it runs the same steps in place, with the caller's own CARGO_TARGET_DIR.

    What it does not cover: code that behaves differently on Windows (`cfg!(windows)`, such as
    finding the real git behind Git for Windows' launcher) runs its Linux branch here. Run the
    suite on Windows too (cargo nextest run --offline --locked) before installing a new build on
    a Windows host.

    Exit code: 0 when every step passed; the failing step's exit code otherwise; 2 when it cannot
    run (no checkout, no WSL, no cargo).

.EXAMPLE
    pwsh scripts/linux-suite.ps1 -Agent claude

.EXAMPLE
    pwsh scripts/linux-suite.ps1 -Agent claude -Checkout C:\Users\me\susi-agents\claude\susi-tools -Lint
#>
[CmdletBinding()]
param(
    [string]$Agent = $env:SUSI_AGENT,
    [string]$Checkout = '.',
    [string]$Distro = '',
    [switch]$Lint
)

$ErrorActionPreference = 'Stop'
function Write-Step([string]$message) {
    Write-Host ("{0} linux-suite: {1}" -f (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ'), $message)
}

$agentSeg = ($Agent + '').ToLowerInvariant() -replace '[^a-z0-9_-]', '-'
if (-not $agentSeg.Trim('-')) {
    Write-Step 'give -Agent <your agent name> (or set SUSI_AGENT): each agent builds in its own folder'
    exit 2
}
$root = (Resolve-Path -LiteralPath $Checkout -ErrorAction SilentlyContinue)
if (-not $root -or -not (Test-Path (Join-Path $root 'Cargo.toml'))) {
    Write-Step "not a Cargo checkout: $Checkout"
    exit 2
}
$root = $root.Path
$started = [Diagnostics.Stopwatch]::StartNew()

if ($IsLinux) {
    Set-Location $root
    $env:CARGO_NET_OFFLINE = 'true'
    $suite = if ((& cargo nextest --version 2>$null) -and $LASTEXITCODE -eq 0) { @('nextest', 'run') } else { @('test') }
    Write-Step "suite in ${root}: cargo $($suite -join ' ') --offline --locked"
    & cargo @suite --offline --locked
    if ($LASTEXITCODE -ne 0) { Write-Step "suite failed ($LASTEXITCODE)"; exit $LASTEXITCODE }
    if ($Lint) {
        Write-Step 'lint: cargo clippy --all-targets --offline --locked -- -D warnings'
        & cargo clippy --all-targets --offline --locked -- -D warnings
        if ($LASTEXITCODE -ne 0) { Write-Step "lint failed ($LASTEXITCODE)"; exit $LASTEXITCODE }
    }
    Write-Step ("every step passed in {0:N1} s" -f $started.Elapsed.TotalSeconds)
    exit 0
}

if (-not (Get-Command wsl.exe -ErrorAction SilentlyContinue)) {
    Write-Step 'WSL2 is not installed on this host: run the suite on a Linux host instead'
    exit 2
}
$pick = if ($Distro) { @('-d', $Distro) } else { @() }
# The checkout's files, as Git sees them on this side (a linked worktree's `.git` names a Windows
# path that Linux git cannot follow).
$listed = @(& git -C $root -c core.quotepath=off ls-files --cached --others --exclude-standard 2>$null)
if ($LASTEXITCODE -ne 0 -or -not $listed) {
    Write-Step "git cannot list the files of $root"
    exit 2
}
$deleted = @(& git -C $root -c core.quotepath=off ls-files --deleted 2>$null)
$files = if ($deleted) { $gone = [Collections.Generic.HashSet[string]]::new([string[]]$deleted); @($listed | Where-Object { -not $gone.Contains($_) }) } else { $listed }
# One line, single quotes only: it reaches bash through wsl.exe's command line unchanged. The
# file list arrives on standard input (PowerShell ends its lines with CR LF: the CRs go).
$windowsPath = "'" + $root.Replace("'", "'\''") + "'"
$steps = @(
    'set -u -o pipefail',
    'IFS=$''\n''',
    "src=`$(wslpath -a -u $windowsPath) || exit 2",
    "home=`$HOME/susi-tools-linux/$agentSeg",
    'copy=$home/checkout',
    'rm -rf $copy && mkdir -p $copy || exit 2',
    'tr -d ''\r'' | (cd $src && tar -cf - -T -) | (cd $copy && tar -xf -) || { echo cannot copy the checkout from $src; exit 2; }',
    'cd $copy || exit 2',
    'export RUSTC_WRAPPER= CARGO_BUILD_RUSTC_WRAPPER= CARGO_NET_OFFLINE=true CARGO_TARGET_DIR=$home/target',
    'command -v cargo >/dev/null || { echo no cargo in this WSL distribution; exit 2; }',
    'if command -v cargo-nextest >/dev/null; then suite=nextest_run; else suite=test; fi',
    'echo ''  linux-suite, in WSL: suite of'' $src, copied to $copy, on $(uname -sr): cargo ${suite/_/ } --offline --locked',
    'if [ $suite = test ]; then cargo test --offline --locked </dev/null || exit $?; else cargo nextest run --offline --locked </dev/null || exit $?; fi'
)
if ($Lint) {
    $steps += 'echo ''  linux-suite, in WSL: lint: cargo clippy --all-targets --offline --locked -- -D warnings'''
    $steps += 'cargo clippy --all-targets --offline --locked -- -D warnings </dev/null || exit $?'
}
Write-Step "running in WSL ($(if ($Distro) { $Distro } else { 'default distribution' })) for agent $agentSeg, $($files.Count) files"
$files | & wsl.exe @pick -e bash -lc ($steps -join '; ')
$code = $LASTEXITCODE
if ($code -ne 0) {
    Write-Step ("a step failed (exit {0}) after {1:N1} s" -f $code, $started.Elapsed.TotalSeconds)
    exit $code
}
Write-Step ("every step passed in {0:N1} s, WSL start included" -f $started.Elapsed.TotalSeconds)
exit 0

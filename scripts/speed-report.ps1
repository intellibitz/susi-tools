# SPDX-License-Identifier: Apache-2.0
<#
.SYNOPSIS
    Time the everyday operations of a Cargo checkout against the 10-second line (vision V-42:
    "no more waiting more than 10s for anything - test, releases whatever it is").

.DESCRIPTION
    Runs each operation once, warm, in the checkout given, and prints how long it took and whether
    it stayed within the line. Of the checkout's files it changes only one file's modification
    time, to time a one-file rebuild; the development program's commands below keep their usual
    effects. Cold timings (empty caches) are not taken here.

    Operations: cargo build (nothing changed, then one file touched), the whole suite with
    cargo-nextest when it is installed and with cargo test (both timed after the tests are built),
    Clippy, rustfmt and, with -Release, the optimised build. When the development program
    susi-tools is on PATH it also times its check, Board-check and task-list commands, run exactly
    as an agent runs them and with their usual effects (the check fast-forwards a clean checkout
    that is behind the remote's main).

    It knows nothing of any one product and depends only on cargo, so it runs against any Cargo
    checkout. Verified on 2026-11-22 against Susi on Windows (PowerShell 7) and on 2026-11-23
    against Susi Tools on Linux (CachyOS, PowerShell 7); not yet run against Susi Core.

    Exit code: 0 when every timed operation is within the line, 1 when any is over or failed,
    2 when the checkout cannot be used.

.EXAMPLE
    pwsh scripts/speed-report.ps1 -Checkout <path to a clone> -Agent <your agent name>

.EXAMPLE
    pwsh scripts/speed-report.ps1 -Checkout <path> -Agent <name> -Release -LineSeconds 10
#>
[CmdletBinding()]
param(
    [Parameter(Mandatory = $true)][string]$Checkout,
    [Parameter(Mandatory = $true)][string]$Agent,
    [double]$LineSeconds = 10,
    [string]$TouchFile = 'src/main.rs',
    [switch]$Release
)

$ErrorActionPreference = 'Continue'
if (-not (Test-Path (Join-Path $Checkout 'Cargo.toml'))) {
    Write-Error "$(Get-Date -Format o) not a Cargo checkout: $Checkout"
    exit 2
}
Set-Location $Checkout
$env:SUSI_AGENT = $Agent
$rows = New-Object System.Collections.Generic.List[object]

function Measure-Op([string]$Name, [scriptblock]$Block) {
    Write-Host "$(Get-Date -Format o) timing: $Name"
    $sw = [Diagnostics.Stopwatch]::StartNew()
    $null = & $Block 2>&1
    $code = $LASTEXITCODE
    $sw.Stop()
    $s = [Math]::Round($sw.Elapsed.TotalSeconds, 1)
    $within = if ($code -ne 0) { 'FAILED' } elseif ($s -le $LineSeconds) { 'yes' } else { 'NO' }
    $rows.Add([pscustomobject]@{ Operation = $Name; Seconds = $s; Within = $within; Exit = $code })
}

$stamp = (Get-Date).ToUniversalTime().ToString('yyyy-MM-ddTHH:mm:ssZ')
Measure-Op 'cargo build, nothing changed' { cargo build --offline --locked }
if (Test-Path $TouchFile) {
    (Get-Item $TouchFile).LastWriteTime = Get-Date
    Measure-Op "cargo build, $TouchFile touched" { cargo build --offline --locked }
}
$null = cargo test --workspace --offline --locked --no-run 2>&1
if (Get-Command cargo-nextest -ErrorAction SilentlyContinue) {
    $null = cargo nextest run --workspace --offline --locked --no-run 2>&1
    Measure-Op 'whole suite, cargo nextest' { cargo nextest run --workspace --offline --locked --no-fail-fast }
}
Measure-Op 'whole suite, cargo test' { cargo test --workspace --offline --locked --no-fail-fast }
Measure-Op 'cargo clippy, warm' { cargo clippy --workspace --all-targets --offline --locked -- -D warnings }
Measure-Op 'cargo fmt --check' { cargo fmt --all --check }
if (Get-Command susi-tools -ErrorAction SilentlyContinue) {
    Measure-Op 'susi-tools workflow check' { susi-tools workflow check --agent $Agent }
    Measure-Op 'susi-tools board check' { susi-tools board check --agent $Agent }
    Measure-Op 'susi-tools tasks list' { susi-tools tasks list --json }
}
if ($Release) {
    # Runs after the touch above, so the root package is rebuilt with the release profile.
    Measure-Op "cargo build --release, $TouchFile touched" { cargo build --release --offline --locked }
}

Write-Host "speed report for $Checkout at $(git -C $Checkout log --oneline -1) ($stamp, line $LineSeconds s, warm)"
$rows | Format-Table -AutoSize | Out-String -Width 160 | Write-Host
$bad = @($rows | Where-Object { $_.Within -ne 'yes' })
Write-Host ("{0} of {1} operations within the line" -f ($rows.Count - $bad.Count), $rows.Count)
exit $(if ($bad.Count) { 1 } else { 0 })

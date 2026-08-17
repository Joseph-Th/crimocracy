# verify.ps1 -- local Crimocracy completion gate (no hosted CI).
#
# Runs the same four-stage contract the repository treats as authoritative, in order, and stops at
# the first failing stage so a broken change fails fast:
#   1. cargo fmt --check
#   2. cargo clippy --locked --lib --example gameplay_harness -- -D warnings
#   3. cargo test --locked --all-targets --quiet
#   4. cargo test --locked --example gameplay_harness tests::smoke_mode_covers_canonical_paths
#      -- --ignored --exact --nocapture
#
# Build parallelism is cargo-autodetected; pass -Jobs N to cap it (e.g. for a quieter machine).
# Exit code is non-zero when any stage fails, so the script can gate commits or hooks.

[CmdletBinding()]
param(
    [int]$Jobs = 0
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

function Invoke-CargoStage {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [bool]$AllowJobs = $true
    )
    Write-Host "== Stage: $Name ==" -ForegroundColor Cyan
    if ($AllowJobs -and $Jobs -gt 0) {
        # `-j` is a cargo option and must precede any `--` separator, otherwise it
        # would be forwarded to the tool subprocess (e.g. rustc via clippy).
        $jobsArgs = @("-j", "$Jobs")
        $separatorIndex = [Array]::IndexOf($Arguments, "--")
        if ($separatorIndex -ge 0) {
            $Arguments = $Arguments[0..($separatorIndex - 1)] + $jobsArgs + $Arguments[$separatorIndex..($Arguments.Length - 1)]
        } else {
            $Arguments += $jobsArgs
        }
    }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    & cargo @Arguments
    $exit = $LASTEXITCODE
    $sw.Stop()
    if ($exit -ne 0) {
        Write-Host "[FAIL] $Name ($([math]::Round($sw.Elapsed.TotalSeconds, 1))s) -- cargo exited $exit" -ForegroundColor Red
        exit $exit
    }
    Write-Host "[PASS] $Name ($([math]::Round($sw.Elapsed.TotalSeconds, 1))s)" -ForegroundColor Green
}

Write-Host "CRIMOCRACY LOCAL GATE" -ForegroundColor Cyan

$gate = [System.Diagnostics.Stopwatch]::StartNew()

Invoke-CargoStage "fmt" @("fmt", "--check") -AllowJobs $false
Invoke-CargoStage "clippy (lib + harness)" @(
    "clippy", "--locked", "--lib", "--example", "gameplay_harness", "--", "-D", "warnings"
)
Invoke-CargoStage "all-target tests" @("test", "--locked", "--all-targets", "--quiet")
Invoke-CargoStage "harness smoke contract" @(
    "test", "--locked", "--example", "gameplay_harness", "tests::smoke_mode_covers_canonical_paths",
    "--", "--ignored", "--exact", "--nocapture"
)

$gate.Stop()
Write-Host "GATE PASS in $([math]::Round($gate.Elapsed.TotalSeconds, 1))s (4/4 stages)" -ForegroundColor Green
# verify.ps1 -- local Crimocracy completion gate for a solo developer.
#
# Optimized for fast incremental iteration: cached warm runs are ~0.7-1.5s for the
# fast lane and ~5-10s for the full gate, because all stages reuse artifacts and
# avoid redundant rebuilds. A small library edit costs ~7-16s to re-check and
# ~23-31s to rebuild+link tests (see Cargo.toml profile notes for measured tuning).
# Cold builds are dominated by rustc.
#
# Incremental is measurably slower for the dev-profile everyday loop, but
# measurably faster for the single-binary harness profile ([profile.harness]
# sets incremental = true). The pin is therefore scoped per stage below:
# dev-profile stages force CARGO_INCREMENTAL=0 so an inherited variable cannot
# re-enable dev incremental; harness-profile stages clear it so the profile's
# own setting governs.
#
# Stages (full gate, in order, fail-fast):
#   1. cargo fmt --check
#   2. cargo test --locked --lib --tests --quiet   (lib + integration, excludes examples)
#   3. cargo test --locked --quiet --example gameplay_harness --lib   (harness unit tests)
#   4. cargo test --locked --quiet --example gameplay_harness tests::smoke_mode_covers_canonical_paths -- --ignored --exact --nocapture
#   5. gameplay-harness full mode on [profile.harness] (--samples 1): exercises every
#      narrative/probe contract that smoke mode does not cover, in seconds
#   6. cargo clippy --locked --lib --example gameplay_harness -- -D warnings
#
# Tests run before clippy so the hot test cache is not invalidated by clippy's
# driver hash. Clippy is last: you get test signal even if lint fails. The smoke
# contract is selected fail-closed (exact count must be 1). See
# `Get-SmokeContractSelectableCount` and `Invoke-SmokeContractSelectionSelfTest`.
#
# Fast lanes for iteration:
#   .\scripts\verify.cmd -Fast              # fmt + lib tests (skip soak) ~1s warm
#   .\scripts\verify.cmd -Fast -Harness     # fmt + smoke contract only
#   cargo check-fast / test-fast / harness  # even more targeted, via .cargo aliases
#
# Additional options for targeted debugging:
#   -Filter <pattern>   Run only matching lib tests (implies -Fast)
#   -NoClippy / -NoFmt  Skip stages when you know they already pass
#   -Jobs N             Cap cargo parallelism (e.g. -Jobs 2 on a hot laptop)
#   -Verbose / -Detail  Show cargo output even for passing quiet stages
#
# Exit code is non-zero on the first failing stage so hooks can gate commits.

[CmdletBinding()]
param(
    [int]$Jobs = 0,
    [switch]$Fast,
    [switch]$Harness,
    [switch]$SelfTest,
    [string]$Filter = "",
    [switch]$NoClippy,
    [switch]$NoFmt,
    [switch]$Detail
)

# Support -Verbose (common parameter) as alias for -Detail without collision.
if (-not $Detail -and $PSBoundParameters.ContainsKey('Verbose')) {
    $Detail = $true
}
if ($VerbosePreference -eq 'Continue' -and -not $Detail) {
    $Detail = $true
}

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

$SmokeContract = "tests::smoke_mode_covers_canonical_paths"

# ── helpers ──────────────────────────────────────────────────────────────────

function Get-SmokeContractSelectableCount {
    param([string[]]$ListingLines)
    return @($ListingLines | Where-Object {
        $_ -match ('^' + [regex]::Escape($SmokeContract) + ':\s+test\s*$')
    }).Count
}

function Invoke-SmokeContractSelectionSelfTest {
    $present = @("$($SmokeContract): test", "1 test, 0 benchmarks")
    $missing = @("tests::some_other_renamed_contract: test", "1 test, 0 benchmarks")
    $ambiguous = @("$($SmokeContract): test", "tests::smoke_mode_covers_canonical_paths: test", "2 tests, 0 benchmarks")
    $zero = @("0 test, 0 benchmarks")
    $expectations = @(
        @{ Name = "present"; Lines = $present; Want = 1 },
        @{ Name = "missing"; Lines = $missing; Want = 0 },
        @{ Name = "ambiguous"; Lines = $ambiguous; Want = 2 },
        @{ Name = "zero"; Lines = $zero; Want = 0 }
    )
    foreach ($case in $expectations) {
        $got = Get-SmokeContractSelectableCount -ListingLines $case.Lines
        if ($got -ne $case.Want) {
            Write-Host "[FAIL] smoke-selection selftest '$($case.Name)': expected $($case.Want), got $got" -ForegroundColor Red
            exit 1
        }
        Write-Host "[PASS] smoke-selection selftest '$($case.Name)' ($got)" -ForegroundColor Green
    }
}

function Assert-SmokeContractSelectable {
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    $smokeListing = & cargo test --locked --quiet --example gameplay_harness -- --list --ignored 2>&1
    $code = $LASTEXITCODE
    $ErrorActionPreference = $prevEAP
    if ($code -ne 0) {
        Write-Host "[FAIL] harness smoke contract -- could not list ignored tests (cargo exited $code)" -ForegroundColor Red
        Write-Host ($smokeListing -join "`n") -ForegroundColor DarkGray
        exit $code
    }
    $smokeSelectable = Get-SmokeContractSelectableCount -ListingLines $smokeListing
    if ($smokeSelectable -ne 1) {
        Write-Host "[FAIL] harness smoke contract -- expected exactly '$SmokeContract', found $smokeSelectable matching line(s)" -ForegroundColor Red
        Write-Host ($smokeListing | Where-Object { $_ -match "smoke_mode" } | Out-String) -ForegroundColor DarkGray
        exit 1
    }
}

function Invoke-CargoStage {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [bool]$AllowJobs = $true,
        # Harness-profile stages run under [profile.harness] (incremental there
        # is a measured win), so the dev pin must be cleared for them.
        [switch]$HarnessProfile,
        [switch]$ShowOutputOnPass
    )
    if ($HarnessProfile) { Clear-DevIncrementalPin } else { Set-DevIncrementalPin }
    $displayName = if ($Name.Length -gt 28) { $Name.Substring(0, 28) } else { $Name.PadRight(28) }
    Write-Host "  $displayName " -NoNewline -ForegroundColor Cyan
    $cargoArgs = $Arguments
    if ($AllowJobs -and $Jobs -gt 0) {
        $jobsArgs = @("-j", "$Jobs")
        $sep = [Array]::IndexOf($cargoArgs, "--")
        if ($sep -ge 0) {
            $cargoArgs = $cargoArgs[0..($sep - 1)] + $jobsArgs + $cargoArgs[$sep..($cargoArgs.Length - 1)]
        } else {
            $cargoArgs += $jobsArgs
        }
    }
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    $output = & cargo @cargoArgs 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $ErrorActionPreference = $prevEAP
    $sw.Stop()
    $elapsed = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    $timing = ("{0,5}s" -f $elapsed)

    if ($exit -ne 0) {
        Write-Host "FAIL $timing" -ForegroundColor Red
        Write-Host $output -ForegroundColor DarkGray
        Write-Host "  -> cargo $($cargoArgs -join ' ') exited $exit" -ForegroundColor Red
        exit $exit
    }
    if ($Detail -or $ShowOutputOnPass) {
        $trimmed = $output.Trim()
        if ($trimmed) { Write-Host $trimmed -ForegroundColor DarkGray }
    }
    $script:GateStagesPassed++
    Write-Host "ok   $timing" -ForegroundColor Green
}

function Skip-Stage {
    param([Parameter(Mandatory = $true)][string]$Name, [string]$Reason)
    $displayName = if ($Name.Length -gt 28) { $Name.Substring(0, 28) } else { $Name.PadRight(28) }
    Write-Host "  $displayName SKIP ($Reason)" -ForegroundColor Yellow
    $script:GateStagesSkipped++
}

# ── header ───────────────────────────────────────────────────────────────────

# Dev-profile stages must never let an inherited CARGO_INCREMENTAL=1 re-enable
# dev incremental; harness-profile stages need the variable unset so the
# [profile.harness] incremental setting applies.
function Set-DevIncrementalPin {
    $env:CARGO_INCREMENTAL = "0"
}

function Clear-DevIncrementalPin {
    Remove-Item Env:\CARGO_INCREMENTAL -ErrorAction SilentlyContinue
}

# Gate bookkeeping: Invoke-CargoStage counts every passing stage; explicit SKIP
# branches increment the skipped count so the summary stays honest.
$script:GateStagesPassed = 0
$script:GateStagesSkipped = 0

$gitBranch = ""
try { $gitBranch = (& git rev-parse --abbrev-ref HEAD 2>$null).Trim() } catch {}
$gitShort = ""
try { $gitShort = (& git rev-parse --short HEAD 2>$null).Trim() } catch {}

Write-Host ""
Write-Host "CRIMOCRACY LOCAL VERIFICATION" -ForegroundColor Cyan
if ($gitBranch) { Write-Host "  branch $gitBranch @ $gitShort  " -NoNewline -ForegroundColor DarkGray; Write-Host "|  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor DarkGray }

if ($SelfTest) {
    if ($Fast -or $Harness -or $Filter) {
        Write-Host "[FAIL] -SelfTest cannot be combined with -Fast, -Harness, or -Filter" -ForegroundColor Red
        exit 1
    }
    Invoke-SmokeContractSelectionSelfTest
    Write-Host "SMOKE-SELECTION SELFTEST PASS" -ForegroundColor Green
    exit 0
}

if ($Harness -and -not $Fast) {
    Write-Host "[FAIL] -Harness requires -Fast" -ForegroundColor Red
    exit 1
}

if ($Filter -and -not $Fast) {
    Write-Host "  note: -Filter implies -Fast (focused lib tests)" -ForegroundColor Yellow
    $Fast = $true
}

# ── fast lanes ───────────────────────────────────────────────────────────────

if ($Fast) {
    $gate = [System.Diagnostics.Stopwatch]::StartNew()
    if ($Filter) {
        Write-Host "FAST: focused lib tests matching '$Filter'" -ForegroundColor Yellow
        if (-not $NoFmt) { Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false }
        Invoke-CargoStage "test-focused $Filter" @("test", "--locked", "--lib", "--quiet", $Filter) -ShowOutputOnPass:$Detail
        $gate.Stop()
        Write-Host "FAST PASS ($([math]::Round($gate.Elapsed.TotalSeconds,1))s)  filter: $Filter" -ForegroundColor Green
        Write-Host "  next: cargo test-focused <filter>  |  .\scripts\verify.cmd  (full gate before push)" -ForegroundColor DarkGray
        exit 0
    }

    $lane = if ($Harness) { "harness smoke" } else { "library unit tests (soak excluded)" }
    Write-Host "FAST LANE: $lane" -ForegroundColor Yellow
    if (-not $NoFmt) { Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false }
    if ($Harness) {
        Assert-SmokeContractSelectable
        Invoke-CargoStage "harness smoke" @("test", "--locked", "--quiet", "--example", "gameplay_harness", $SmokeContract, "--", "--ignored", "--exact", "--nocapture") -ShowOutputOnPass
    } else {
        Invoke-CargoStage "lib tests (no soak)" @("test", "--locked", "--lib", "--quiet", "--", "--skip", "soak")
    }
    $gate.Stop()
    Write-Host "FAST PASS ($([math]::Round($gate.Elapsed.TotalSeconds,1))s)  $lane" -ForegroundColor Green
    Write-Host "  next: .\scripts\verify.cmd  (full gate before push)  |  cargo soak  (if you touched invariants/persistence)" -ForegroundColor DarkGray
    exit 0
}

# ── full completion gate ─────────────────────────────────────────────────────

$gate = [System.Diagnostics.Stopwatch]::StartNew()
$jobsDisplay = if ($Jobs -eq 0) { "auto" } else { "$Jobs" }
Write-Host "FULL GATE  (fmt -> tests -> harness units -> harness smoke -> harness full -> clippy)  [Jobs=$jobsDisplay]" -ForegroundColor Cyan

if ($NoFmt) {
    Skip-Stage -Name "fmt --check" -Reason "--NoFmt"
} else {
    Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false
}

Invoke-CargoStage "lib+integration tests" @("test", "--locked", "--lib", "--tests", "--quiet")

# The harness example carries its own unit tests for options parsing and the financial
# branch contracts; --lib --tests never compiles example test targets, so run them here.
Invoke-CargoStage "harness unit tests" @("test", "--locked", "--quiet", "--example", "gameplay_harness", "--lib")

Assert-SmokeContractSelectable
Invoke-CargoStage "harness smoke" @("test", "--locked", "--quiet", "--example", "gameplay_harness", $SmokeContract, "--", "--ignored", "--exact", "--nocapture") -ShowOutputOnPass

# Full mode exercises every narrative, probe, and cross-branch contract that smoke mode
# skips; a single-sample run costs seconds and has caught contract drift the gate
# previously could not see. It runs on [profile.harness] (dev semantics, opt-level 1,
# incremental = true): warm full-mode runtime ~1-2s, and after a library edit the
# profile's incremental cache cuts the rebuild from ~75s to ~10-20s.
Invoke-CargoStage "harness full (n=1)" @("run", "--locked", "--profile", "harness", "--quiet", "--example", "gameplay_harness", "--", "--mode", "full", "--samples", "1") -HarnessProfile

if ($NoClippy) {
    Skip-Stage -Name "clippy (lib+harness)" -Reason "--NoClippy"
} else {
    Invoke-CargoStage "clippy (lib+harness)" @("clippy", "--locked", "--lib", "--example", "gameplay_harness", "--", "-D", "warnings")
}

$gate.Stop()
Write-Host ""
$skippedNote = if ($script:GateStagesSkipped -gt 0) { ", $($script:GateStagesSkipped) skipped by flag" } else { "" }
Write-Host "GATE PASS  $($script:GateStagesPassed) stages$skippedNote in $([math]::Round($gate.Elapsed.TotalSeconds,1))s" -ForegroundColor Green
Write-Host "  tip: use -Fast for iteration, -Filter <name> for one test, -Jobs N on a hot machine" -ForegroundColor DarkGray

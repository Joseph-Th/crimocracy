# verify.ps1 -- local verification gate for a solo developer.
#
# Design: cheapest proof first, warm caches reused, fail-fast.
#
# Warm cache (no file changed):          fmt 0.6s  check 0.06s  test 0.11s  full gate 2-3s
# After touching one lib file:           check 6s   test 12s    full gate 15-20s
# Cold (cargo clean):                    ~60-90s (dominated by rustc)
#
# Incremental is slower for [profile.dev] but faster for the single-binary
# [profile.harness]; the pin is scoped per stage: dev stages force
# CARGO_INCREMENTAL=0, harness stages clear it so the profile governs.
# See Cargo.toml for measured profile tuning and alternatives that lost.
#
# Stages (full gate, in order, fail-fast):
#   1. cargo fmt --check
#   2. cargo test --locked --lib --tests --quiet         (lib + integration)
#   3. cargo test --locked --quiet --example gameplay_harness --lib
#   4. harness smoke contract (exact ignored test, fail-closed)
#   5. harness full --samples 1 on [profile.harness]      (narratives + probes)
#   6. cargo clippy --locked --lib --example gameplay_harness -- -D warnings
#
# Tests run before clippy so the hot test cache is not invalidated by clippy's
# driver hash. Clippy is last: you get test signal even if lint fails.
#
# Lanes:
#   .\scripts\verify.cmd                  full gate (before push)
#   .\scripts\verify.cmd -Fast            fmt + lib tests --skip soak  (~0.7s warm)
#   .\scripts\verify.cmd -Fast -Harness   fmt + smoke contract only    (~0.7s warm)
#   .\scripts\verify.cmd -Check           type-check only, no tests     (~0.06s warm)
#   .\scripts\verify.cmd -Fast -Filter X  fmt + matching lib tests      (~0.5s warm)
#   cargo check-fast / test-fast / harness  even more targeted, via .cargo aliases
#
# Flags: -Jobs N  cap cargo parallelism  |  -NoClippy -NoFmt  skip known-passing
#        -Verbose (-Detail)  show cargo output on success
# Exit code is non-zero on first failing stage.

[CmdletBinding()]
param(
    [int]$Jobs = 0,
    [switch]$Fast,
    [switch]$Harness,
    [switch]$Check,
    [switch]$SelfTest,
    [string]$Filter = "",
    [switch]$NoClippy,
    [switch]$NoFmt,
    [switch]$Detail
)

if (-not $Detail -and $PSBoundParameters.ContainsKey('Verbose')) { $Detail = $true }
if ($VerbosePreference -eq 'Continue' -and -not $Detail) { $Detail = $true }

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
        # Hint at the cheapest re-check for the failure domain.
        $hint = switch -Wildcard ($Name) {
            "fmt*"              { "fix formatting: cargo fmt" }
            "lib*tests"         { "re-run: cargo test-focused <filter>  or  cargo test --lib -- --nocapture" }
            "harness unit*"     { "re-run: cargo test --quiet --example gameplay_harness --lib -- --nocapture" }
            "harness smoke"     { "re-run: cargo harness-rush  or  cargo test --example gameplay_harness -- --ignored --nocapture" }
            "harness full*"     { "re-run: cargo harness-full --samples 1" }
            "clippy*"           { "fix lints: cargo clippy --lib --example gameplay_harness -- -D warnings" }
            default             { "" }
        }
        if ($hint) { Write-Host "  hint: $hint" -ForegroundColor Yellow }
        exit $exit
    }
    if ($Detail -or $ShowOutputOnPass) {
        $trimmed = $output.Trim()
        if ($trimmed) { Write-Host $trimmed -ForegroundColor DarkGray }
    }
    $script:GateStagesPassed++
    # Record timing for the summary table.
    $script:GateTimings += [pscustomobject]@{ Stage = $Name; Seconds = $elapsed }
    Write-Host "ok   $timing" -ForegroundColor Green
}

function Skip-Stage {
    param([Parameter(Mandatory = $true)][string]$Name, [string]$Reason)
    $displayName = if ($Name.Length -gt 28) { $Name.Substring(0, 28) } else { $Name.PadRight(28) }
    Write-Host "  $displayName SKIP ($Reason)" -ForegroundColor Yellow
    $script:GateStagesSkipped++
}

# ── incremental pin scoping ──────────────────────────────────────────────────

function Set-DevIncrementalPin { $env:CARGO_INCREMENTAL = "0" }
function Clear-DevIncrementalPin { Remove-Item Env:\CARGO_INCREMENTAL -ErrorAction SilentlyContinue }

# ── bookkeeping ──────────────────────────────────────────────────────────────

$script:GateStagesPassed = 0
$script:GateStagesSkipped = 0
$script:GateTimings = @()

$gitBranch = ""
try { $gitBranch = (& git rev-parse --abbrev-ref HEAD 2>$null).Trim() } catch {}
$gitShort = ""
try { $gitShort = (& git rev-parse --short HEAD 2>$null).Trim() } catch {}

Write-Host ""
Write-Host "CRIMOCRACY LOCAL VERIFICATION" -ForegroundColor Cyan
if ($gitBranch) {
    Write-Host "  branch $gitBranch @ $gitShort  " -NoNewline -ForegroundColor DarkGray
    Write-Host "|  $(Get-Date -Format 'yyyy-MM-dd HH:mm:ss')" -ForegroundColor DarkGray
}

if ($SelfTest) {
    if ($Fast -or $Harness -or $Filter -or $Check) {
        Write-Host "[FAIL] -SelfTest cannot be combined with -Fast, -Harness, -Check, or -Filter" -ForegroundColor Red
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
if ($Check -and ($Fast -or $Harness -or $Filter)) {
    Write-Host "[FAIL] -Check cannot be combined with -Fast, -Harness, or -Filter" -ForegroundColor Red
    exit 1
}
if ($Filter -and -not $Fast) {
    Write-Host "  note: -Filter implies -Fast (focused lib tests)" -ForegroundColor Yellow
    $Fast = $true
}

# ── type-check lane (fastest: no linking) ──────────────────────────────────

if ($Check) {
    $gate = [System.Diagnostics.Stopwatch]::StartNew()
    Write-Host "CHECK LANE: type-check (lib + harness)" -ForegroundColor Yellow
    if (-not $NoFmt) { Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false }
    Invoke-CargoStage "check (lib + harness)" @("check", "--locked", "--lib", "--example", "gameplay_harness")
    $gate.Stop()
    Write-Host "CHECK PASS ($([math]::Round($gate.Elapsed.TotalSeconds,1))s)  type-check only" -ForegroundColor Green
    Write-Host "  next: .\scripts\verify.cmd -Fast  (tests)  |  cargo check-fast  (lib only, even faster)" -ForegroundColor DarkGray
    exit 0
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
Write-Host "FULL GATE  (fmt -> lib+integration -> harness units -> smoke -> full n=1 -> clippy)  [Jobs=$jobsDisplay]" -ForegroundColor Cyan

if ($NoFmt) {
    Skip-Stage -Name "fmt --check" -Reason "--NoFmt"
} else {
    Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false
}

Invoke-CargoStage "lib+integration tests" @("test", "--locked", "--lib", "--tests", "--quiet")

# Harness unit tests (options parsing, financial contracts) live in the example
# target; --lib --tests never compiles example test targets, so run them here.
Invoke-CargoStage "harness unit tests" @("test", "--locked", "--quiet", "--example", "gameplay_harness", "--lib")

Assert-SmokeContractSelectable
Invoke-CargoStage "harness smoke" @("test", "--locked", "--quiet", "--example", "gameplay_harness", $SmokeContract, "--", "--ignored", "--exact", "--nocapture") -ShowOutputOnPass

# Full mode exercises every narrative, probe, and cross-branch contract that
# smoke skips; a single sample costs ~5s and has caught drift that smoke did
# not. Runs on [profile.harness] (opt-level 1, incremental = true): after a
# lib edit the incremental cache cuts the rebuild from ~75s to ~10-20s.
Invoke-CargoStage "harness full (n=1)" @("run", "--locked", "--profile", "harness", "--quiet", "--example", "gameplay_harness", "--", "--mode", "full", "--samples", "1") -HarnessProfile

if ($NoClippy) {
    Skip-Stage -Name "clippy (lib+harness)" -Reason "--NoClippy"
} else {
    Invoke-CargoStage "clippy (lib+harness)" @("clippy", "--locked", "--lib", "--example", "gameplay_harness", "--", "-D", "warnings")
}

$gate.Stop()
$totalSec = [math]::Round($gate.Elapsed.TotalSeconds, 1)
Write-Host ""
# Compact per-stage timing table so a slow gate shows where time went.
if ($script:GateTimings.Count -gt 0) {
    $table = ($script:GateTimings | ForEach-Object { "$($_.Stage): $($_.Seconds)s" }) -join "  |  "
    Write-Host "  stages: $table" -ForegroundColor DarkGray
}
$skippedNote = if ($script:GateStagesSkipped -gt 0) { ", $($script:GateStagesSkipped) skipped by flag" } else { "" }
Write-Host "GATE PASS  $($script:GateStagesPassed) stages$skippedNote in ${totalSec}s" -ForegroundColor Green
Write-Host "  tip: -Fast for iteration  |  -Check for type-check only  |  -Filter <name> for one test  |  -Jobs N on a hot machine" -ForegroundColor DarkGray

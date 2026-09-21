# verify.ps1 -- local verification gate for a solo developer.
#
# Design: cheapest proof first, warm caches reused, fail-fast.
#
# Cargo profiles own cache and incremental behavior. This gate deliberately does not override
# CARGO_INCREMENTAL, so focused and repeated local runs can reuse the repository's normal cache.
#
# Stages (broad gate, in order, fail-fast):
#   1. compile-free documentation contracts
#   2. cargo fmt --check
#   3. cargo test --locked --lib --quiet
#   4. fast gameplay-harness implementation contracts
#   5. gameplay-harness smoke executable
#   6. cargo clippy --locked --lib --example gameplay_harness -- -D warnings
#
# Tests run before clippy so the hot test cache is not invalidated by clippy's
# driver hash. Clippy is last: you get test signal even if lint fails.
#
# Lanes:
#   .\scripts\verify.cmd                  broad gate for contracts that require it
#   .\scripts\verify.cmd -Fast            fmt + lib tests --skip soak
#   .\scripts\verify.cmd -Harness         fmt + harness contracts + smoke
#   .\scripts\verify.cmd -Check           fmt + lib type-check only
#   .\scripts\verify.cmd -Fast -Filter X  fmt + matching lib tests
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
    [string]$Filter = "",
    [switch]$NoClippy,
    [switch]$NoFmt,
    [switch]$Detail
)

if (-not $Detail -and $PSBoundParameters.ContainsKey('Verbose')) { $Detail = $true }
if ($VerbosePreference -eq 'Continue' -and -not $Detail) { $Detail = $true }

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

# ── helpers ──────────────────────────────────────────────────────────────────

function Invoke-CargoStage {
    param(
        [Parameter(Mandatory = $true)][string]$Name,
        [Parameter(Mandatory = $true)][string[]]$Arguments,
        [bool]$AllowJobs = $true,
        [int]$MinimumPassed = 0,
        [switch]$ShowOutputOnPass
    )
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
        # Trim cargo's build noise on failure — show only errors and the failing command.
        $trimmed = $output.Trim()
        if ($trimmed) {
            # Cap failure output to keep the terminal scannable.
            $lines = $trimmed -split "`n"
            if ($lines.Count -gt 80) {
                $lines = $lines[0..79] + @("  ... ($($lines.Count - 80) more lines truncated; re-run with -Detail for full output)")
            }
            Write-Host ($lines -join "`n") -ForegroundColor DarkGray
        }
        Write-Host "  -> cargo $($cargoArgs -join ' ') exited $exit" -ForegroundColor Red
        # Hint at the cheapest re-check for the failure domain.
        $hint = switch -Wildcard ($Name) {
            "fmt*"              { "fix formatting: cargo fmt" }
            "lib*tests"         { "re-run: cargo test-focused <filter>  or  cargo test --lib -- --nocapture" }
            "test-focused*"     { "re-run: cargo test-focused <filter> -- --nocapture" }
            "harness contracts*" { "re-run: cargo test-harness -- --nocapture" }
            "harness smoke"      { "re-run: cargo harness  or  cargo harness-rush" }
            "clippy*"           { "fix lints: cargo clippy --lib --example gameplay_harness -- -D warnings" }
            "check*"            { "re-run: cargo check-fast  or  cargo check-all" }
            default             { "" }
        }
        if ($hint) { Write-Host "  hint: $hint" -ForegroundColor Yellow }
        exit $exit
    }
    # Extract pass count for the success line when available. Some Cargo test
    # invocations report multiple binaries; sum them for one concise stage result.
    $countSuffix = ""
    $totalPassed = 0
    $allPassed = [regex]::Matches($output, '(\d+) passed')
    if ($allPassed.Count -gt 0) {
        foreach ($m in $allPassed) { $totalPassed += [int]$m.Groups[1].Value }
        if ($allPassed.Count -gt 1) {
            $countSuffix = "  $totalPassed passed ($($allPassed.Count) binaries)"
        } else {
            $countSuffix = "  $totalPassed passed"
        }
        $allFailed = [regex]::Matches($output, '(\d+) failed')
        $totalFailed = 0
        foreach ($m in $allFailed) { $totalFailed += [int]$m.Groups[1].Value }
        if ($totalFailed -gt 0) { $countSuffix += "  $totalFailed FAILED" }
    }
    if ($MinimumPassed -gt 0 -and $totalPassed -lt $MinimumPassed) {
        Write-Host "FAIL $timing" -ForegroundColor Red
        Write-Host "  expected at least $MinimumPassed matching test(s), but Cargo ran $totalPassed" -ForegroundColor Red
        Write-Host "  -> cargo $($cargoArgs -join ' ')" -ForegroundColor Red
        exit 1
    }
    if ($Detail -or $ShowOutputOnPass) {
        $trimmed = $output.Trim()
        if ($trimmed) { Write-Host $trimmed -ForegroundColor DarkGray }
    }
    $script:GateStagesPassed++
    # Record timing for the summary table.
    $script:GateTimings += [pscustomobject]@{ Stage = $Name; Seconds = $elapsed }
    if ($countSuffix) {
        Write-Host "ok   $timing$countSuffix" -ForegroundColor Green
    } else {
        Write-Host "ok   $timing" -ForegroundColor Green
    }
}

function Invoke-DocsStage {
    $displayName = "docs contracts".PadRight(28)
    Write-Host "  $displayName " -NoNewline -ForegroundColor Cyan
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $prevEAP = $ErrorActionPreference
    $ErrorActionPreference = "SilentlyContinue"
    $output = & powershell -NoProfile -ExecutionPolicy Bypass -File "$PSScriptRoot\check-docs.ps1" 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $ErrorActionPreference = $prevEAP
    $sw.Stop()
    $elapsed = [math]::Round($sw.Elapsed.TotalSeconds, 1)
    $timing = ("{0,5}s" -f $elapsed)
    if ($exit -ne 0) {
        Write-Host "FAIL $timing" -ForegroundColor Red
        if ($output.Trim()) { Write-Host $output.Trim() -ForegroundColor DarkGray }
        Write-Host "  -> .\scripts\check-docs.cmd" -ForegroundColor Red
        exit $exit
    }
    $script:GateStagesPassed++
    $script:GateTimings += [pscustomobject]@{ Stage = "docs contracts"; Seconds = $elapsed }
    Write-Host "ok   $timing" -ForegroundColor Green
}

function Skip-Stage {
    param([Parameter(Mandatory = $true)][string]$Name, [string]$Reason)
    $displayName = if ($Name.Length -gt 28) { $Name.Substring(0, 28) } else { $Name.PadRight(28) }
    Write-Host "  $displayName SKIP ($Reason)" -ForegroundColor Yellow
    $script:GateStagesSkipped++
}

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
    Write-Host "  branch $gitBranch @ $gitShort" -ForegroundColor DarkGray
}

if ($Harness -and ($Fast -or $Filter)) {
    Write-Host "[FAIL] -Harness is its own lane; do not combine it with -Fast or -Filter" -ForegroundColor Red
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
    Write-Host "CHECK LANE: library type-check" -ForegroundColor Yellow
    if (-not $NoFmt) { Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false }
    Invoke-CargoStage "check lib" @("check", "--locked", "--lib")
    $gate.Stop()
    Write-Host "CHECK PASS ($([math]::Round($gate.Elapsed.TotalSeconds,1))s)  type-check only" -ForegroundColor Green
    Write-Host "  harness compile: cargo check-harness  |  behavior: cargo test-focused <filter>" -ForegroundColor DarkGray
    exit 0
}

# ── fast lanes ───────────────────────────────────────────────────────────────

if ($Harness) {
    $gate = [System.Diagnostics.Stopwatch]::StartNew()
    Write-Host "HARNESS LANE: implementation contracts + canonical smoke" -ForegroundColor Yellow
    if (-not $NoFmt) { Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false }
    Invoke-CargoStage "harness contracts" @("test", "--locked", "--quiet", "--example", "gameplay_harness")
    Invoke-CargoStage "harness smoke" @("run", "--locked", "--quiet", "--example", "gameplay_harness", "--", "--mode", "smoke")
    $gate.Stop()
    $totalSec = [math]::Round($gate.Elapsed.TotalSeconds, 1)
    if ($script:GateTimings.Count -gt 0) {
        $table = ($script:GateTimings | ForEach-Object { "$($_.Stage): $($_.Seconds)s" }) -join "  |  "
        Write-Host "  stages: $table" -ForegroundColor DarkGray
    }
    Write-Host "HARNESS PASS ($totalSec`s)" -ForegroundColor Green
    Write-Host "  scenario-scale checks remain explicit: cargo test-harness-deep  |  cargo harness-full" -ForegroundColor DarkGray
    exit 0
}

if ($Fast) {
    $gate = [System.Diagnostics.Stopwatch]::StartNew()
    if ($Filter) {
        Write-Host "FAST: focused lib tests matching '$Filter'" -ForegroundColor Yellow
        if (-not $NoFmt) { Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false }
        Invoke-CargoStage "test-focused $Filter" @("test", "--locked", "--lib", "--quiet", $Filter) -MinimumPassed 1 -ShowOutputOnPass:$Detail
        $gate.Stop()
        Write-Host "FAST PASS ($([math]::Round($gate.Elapsed.TotalSeconds,1))s)  filter: $Filter" -ForegroundColor Green
        Write-Host "  broader gate is required only for persistence, invariants, cross-domain work, or verification infrastructure" -ForegroundColor DarkGray
        exit 0
    }

    $lane = "library unit tests (soak excluded)"
    Write-Host "FAST LANE: $lane" -ForegroundColor Yellow
    if (-not $NoFmt) { Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false }
    Invoke-CargoStage "lib tests (no soak)" @("test", "--locked", "--lib", "--quiet", "--", "--skip", "soak")
    $gate.Stop()
    $totalSec = [math]::Round($gate.Elapsed.TotalSeconds, 1)
    if ($script:GateTimings.Count -gt 0) {
        $table = ($script:GateTimings | ForEach-Object { "$($_.Stage): $($_.Seconds)s" }) -join "  |  "
        Write-Host "  stages: $table" -ForegroundColor DarkGray
    }
    Write-Host "FAST PASS ($totalSec`s)  $lane" -ForegroundColor Green
    Write-Host "  broader gate is required only when the changed contract needs it; soak remains explicit stress evidence" -ForegroundColor DarkGray
    exit 0
}

# ── broad completion gate ────────────────────────────────────────────────────

$gate = [System.Diagnostics.Stopwatch]::StartNew()
$jobsDisplay = if ($Jobs -eq 0) { "auto" } else { "$Jobs" }
Write-Host "BROAD GATE  (docs -> fmt -> lib -> harness contracts -> smoke -> clippy)  [Jobs=$jobsDisplay]" -ForegroundColor Cyan

Invoke-DocsStage

if ($NoFmt) {
    Skip-Stage -Name "fmt --check" -Reason "--NoFmt"
} else {
    Invoke-CargoStage "fmt --check" @("fmt", "--check") -AllowJobs:$false
}

Invoke-CargoStage "lib tests" @("test", "--locked", "--lib", "--quiet")

# Verification-infrastructure and cross-domain changes need the harness adapter's fast contracts,
# but scenario-scale comparisons stay in their explicit deep tier.
Invoke-CargoStage "harness contracts" @("test", "--locked", "--quiet", "--example", "gameplay_harness")
Invoke-CargoStage "harness smoke" @("run", "--locked", "--quiet", "--example", "gameplay_harness", "--", "--mode", "smoke")

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
Write-Host "BROAD PASS  $($script:GateStagesPassed) stages$skippedNote in ${totalSec}s" -ForegroundColor Green
Write-Host "  deep gameplay evidence stays explicit: cargo test-harness-deep  |  cargo harness-full --samples N" -ForegroundColor DarkGray

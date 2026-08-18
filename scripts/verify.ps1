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
    [int]$Jobs = 0,
    # Run only the smoke-selection fail-closed regression, then exit. Shortens the loop when editing
    # the selection logic without running the full four-stage gate.
    [switch]$SelfTest
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

# The fully-qualified name of the fixed ignored gameplay smoke contract that stage 4 must run.
$SmokeContract = "tests::smoke_mode_covers_canonical_paths"

# Returns the number of listing lines that select exactly the smoke contract. Cargo exits 0 even when
# an exact filter matches zero tests, so this count (rather than the test process exit code) is what
# proves the contract is still present and selectable.
function Get-SmokeContractSelectableCount {
    param([string[]]$ListingLines)
    return @($ListingLines | Where-Object {
        $_ -match ('^' + [regex]::Escape($SmokeContract) + ':\s+test\s*$')
    }).Count
}

function Invoke-SmokeContractSelectionSelfTest {
    # Script-level regression for the fail-closed selection check: present, missing, and
    # ambiguous/zero-selection listings must be classified correctly without depending on the
    # ordinary stage running at all.
    $present = @(
        "$($SmokeContract): test",
        "1 test, 0 benchmarks"
    )
    $missing = @(
        "tests::some_other_renamed_contract: test",
        "1 test, 0 benchmarks"
    )
    $ambiguous = @(
        "$($SmokeContract): test",
        "tests::smoke_mode_covers_canonical_paths: test",
        "2 tests, 0 benchmarks"
    )
    $zero = @("0 test, 0 benchmarks")
    $expectations = @(
        @{ Name = "present";  Lines = $present;  Want = 1 },
        @{ Name = "missing";  Lines = $missing;  Want = 0 },
        @{ Name = "ambiguous"; Lines = $ambiguous; Want = 2 },
        @{ Name = "zero";     Lines = $zero;     Want = 0 }
    )
    foreach ($case in $expectations) {
        $got = Get-SmokeContractSelectableCount -ListingLines $case.Lines
        if ($got -ne $case.Want) {
            Write-Host "[FAIL] smoke-selection selftest '$($case.Name)': expected $($case.Want), got $got" -ForegroundColor Red
            exit 1
        }
        Write-Host "[PASS] smoke-selection selftest '$($case.Name)' ($got match)" -ForegroundColor Green
    }
}

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

if ($SelfTest) {
    Invoke-SmokeContractSelectionSelfTest
    Write-Host "SMOKE-SELECTION SELFTEST PASS" -ForegroundColor Green
    exit 0
}

$gate = [System.Diagnostics.Stopwatch]::StartNew()

Invoke-CargoStage "fmt" @("fmt", "--check") -AllowJobs $false
Invoke-CargoStage "clippy (lib + harness)" @(
    "clippy", "--locked", "--lib", "--example", "gameplay_harness", "--", "-D", "warnings"
)
Invoke-CargoStage "all-target tests" @("test", "--locked", "--all-targets", "--quiet")

# Fail closed on smoke-contract selection BEFORE running it: cargo test exits 0 even when an exact
# filter matches zero tests, so a renamed/removed contract would otherwise silently erase this stage
# while the gate still reports GATE PASS. List the example's ignored tests and require that the
# fully qualified smoke contract is exactly selectable.
$smokeListing = & cargo test --locked --example gameplay_harness -- --list --ignored
if ($LASTEXITCODE -ne 0) {
    Write-Host "[FAIL] harness smoke contract -- could not list the gameplay_harness ignored tests (cargo exited $LASTEXITCODE)" -ForegroundColor Red
    exit $LASTEXITCODE
}
$smokeSelectable = Get-SmokeContractSelectableCount -ListingLines $smokeListing
if ($smokeSelectable -ne 1) {
    Write-Host "[FAIL] harness smoke contract -- expected exactly the ignored test '$SmokeContract' to be selectable, but found $smokeSelectable matching line(s). The smoke contract may have been renamed or removed." -ForegroundColor Red
    exit 1
}
Invoke-CargoStage "harness smoke contract" @(
    "test", "--locked", "--example", "gameplay_harness", $SmokeContract,
    "--", "--ignored", "--exact", "--nocapture"
)

$gate.Stop()
Write-Host "GATE PASS in $([math]::Round($gate.Elapsed.TotalSeconds, 1))s (4/4 stages)" -ForegroundColor Green
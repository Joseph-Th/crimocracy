# watch.ps1 -- zero-touch targeted iteration loop for a solo developer.
#
# Watches repository code/config (*.rs, *.toml; target\ ignored) for saves
# and reruns one focused lane, so you edit-and-save instead of retyping
# commands. Each run reuses cargo's warm cache; only the lane you chose is
# ever built. Documentation edits deliberately do not trigger Rust builds.
#
# Usage:
#   powershell -NoProfile -File scripts\watch.ps1                  # lib type-check (default)
#   powershell -NoProfile -File scripts\watch.ps1 -Filter <name>   # matching lib tests only
#   powershell -NoProfile -File scripts\watch.ps1 -Harness         # harness smoke executable
#
# The first run starts immediately; later runs start ~300ms after your last
# save. Press Ctrl+C to stop. Works on both Windows PowerShell 5.1 and pwsh.
# For anything beyond these presets, use scripts\verify.cmd directly.

[CmdletBinding()]
param(
    [string]$Filter = "",
    [switch]$Harness,
    [switch]$Clear
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

if ($Filter -and $Harness) {
    Write-Host "[FAIL] -Filter and -Harness select different lanes" -ForegroundColor Red
    exit 1
}

# ── resolve the lane once ────────────────────────────────────────────────────

$title = if ($Filter) {
    "focused tests: $Filter"
} elseif ($Harness) {
    "harness smoke"
} else {
    "lib type-check"
}

$cargoArgs = if ($Filter) {
    @("test", "--locked", "--lib", "--quiet", $Filter)
} elseif ($Harness) {
    @("run", "--locked", "--quiet", "--example", "gameplay_harness",
        "--", "--mode", "smoke")
} else {
    @("check", "--locked", "--quiet", "--lib")
}

# Each run prints its measured elapsed time without inferring cache or rebuild state.
$laneIsCheck = -not $Filter -and -not $Harness

function Invoke-Lane {
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $output = & cargo @cargoArgs 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $sw.Stop()
    # Extract test count from cargo output for concise summary.
    $testCount = ""
    $passed = $null
    if (-not $laneIsCheck) {
        $m = [regex]::Match($output, '(\d+) passed')
        if ($m.Success) {
            $passed = [int]$m.Groups[1].Value
            $testCount = "  $passed passed"
        }
        $fm = [regex]::Match($output, '(\d+) failed')
        if ($fm.Success) { $testCount += "  $($fm.Groups[1].Value) FAILED" }
    }
    if ($Filter -and $exit -eq 0 -and ($null -eq $passed -or $passed -eq 0)) {
        $exit = 1
        $output = "focused filter '$Filter' matched zero tests"
    }
    [pscustomobject]@{
        Exit      = $exit
        Seconds   = [math]::Round($sw.Elapsed.TotalSeconds, 1)
        Output    = $output.Trim()
        TestCount = $testCount
    }
}

# ── watcher setup (PS 5.1 compatible: single Filter, global event state) ─────

$global:WatchPending = $false
$global:WatchOverflow = $false
$global:WatchLastPath = ""

$watcher = New-Object System.IO.FileSystemWatcher
$watcher.Path = (Get-Location).Path
$watcher.IncludeSubdirectories = $true
$watcher.Filter = '*.*'
$watcher.InternalBufferSize = 65536

$onChange = {
    $path = $Event.SourceEventArgs.FullPath
    if ($path -notmatch '[\\/]target[\\/]' -and $path -match '\.(rs|toml)$') {
        $global:WatchPending = $true
        $global:WatchLastPath = $path
    }
}
$onOverflow = { $global:WatchOverflow = $true }

$subscriptions = @(
    (Register-ObjectEvent $watcher Changed -Action $onChange),
    (Register-ObjectEvent $watcher Created -Action $onChange),
    (Register-ObjectEvent $watcher Renamed -Action $onChange),
    (Register-ObjectEvent $watcher Error -Action $onOverflow)
)

try {
    $watcher.EnableRaisingEvents = $true
    Write-Host "CRIMOCRACY WATCH" -ForegroundColor Cyan
    Write-Host "  lane: $title   (save a file to rerun, Ctrl+C to stop)" -ForegroundColor DarkGray
    Write-Host "  tip:  -Filter <name> for one behavior  |  -Harness for gameplay smoke" -ForegroundColor DarkGray

    $runCount = 0
    while ($true) {
        $runCount++
        Write-Host ""
        Write-Host ("[{0}] run #{1}: {2}" -f (Get-Date -Format 'HH:mm:ss'), $runCount, $title) -ForegroundColor Yellow
        $result = Invoke-Lane
        $status = if ($result.Exit -eq 0) { "PASS" } else { "FAIL" }
        $color = if ($result.Exit -eq 0) { "Green" } else { "Red" }
        $countInfo = if ($result.TestCount) { $result.TestCount } else { "" }
        Write-Host ("{0}  {1,5}s{2}" -f $status, $result.Seconds, $countInfo) -ForegroundColor $color
        if ($result.Output -and $result.Exit -ne 0) {
            # Cap watch failure output so the terminal stays scannable.
            $lines = $result.Output -split "`n"
            if ($lines.Count -gt 60) {
                $lines = $lines[0..59] + @("  ... ($($lines.Count - 60) more lines; re-run with cargo test -- --nocapture)")
            }
            Write-Host ($lines -join "`n") -ForegroundColor DarkGray
        }

        if ($Clear) { Clear-Host }
        # Wait for the first change, then let editors finish writing (~300ms).
        $triggered = $false
        do {
            $global:WatchPending = $false
            $global:WatchOverflow = $false
            $global:WatchLastPath = ""
            $idle = [System.Diagnostics.Stopwatch]::StartNew()
            while (-not $global:WatchPending -and -not $global:WatchOverflow -and
                    $idle.Elapsed.TotalMinutes -lt 30) {
                Start-Sleep -Milliseconds 120
            }
            if ($global:WatchOverflow) {
                Write-Host "  [watch: buffer overflow -- rerunning]" -ForegroundColor Yellow
                $triggered = $true
                break
            }
            if ($global:WatchPending) {
                $changed = if ($global:WatchLastPath) {
                    $rel = $global:WatchLastPath.Replace((Get-Location).Path + [IO.Path]::DirectorySeparatorChar, "")
                    " ($rel)"
                } else { "" }
                Start-Sleep -Milliseconds 300
                $global:WatchPending = $false
                Write-Host "  [watch: change detected$changed -- rerunning]" -ForegroundColor DarkGray
                $triggered = $true
            }
        } until ($triggered)
    }
}
finally {
    $watcher.EnableRaisingEvents = $false
    $watcher.Dispose()
    foreach ($subscription in $subscriptions) {
        Unregister-Event -SubscriptionId $subscription.Id -ErrorAction SilentlyContinue
    }
}

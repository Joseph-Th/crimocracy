# watch.ps1 -- zero-touch targeted iteration loop for a solo developer.
#
# Watches repository sources (*.rs, *.toml, *.md; target\ ignored) for saves
# and reruns one focused lane, so you edit-and-save instead of retyping
# commands. Each run reuses cargo's warm cache; only the lane you chose is
# ever built. Markdown is watched because tests/documentation_contracts.rs
# compiles the authority documents in via include_str!.
#
# Usage:
#   powershell -NoProfile -File scripts\watch.ps1                  # fast lib tests (soak excluded)
#   powershell -NoProfile -File scripts\watch.ps1 -Filter <name>   # matching lib tests only
#   powershell -NoProfile -File scripts\watch.ps1 -Harness         # harness smoke contract
#   powershell -NoProfile -File scripts\watch.ps1 -Check           # type-check lib + harness only
#
# The first run starts immediately; later runs start ~300ms after your last
# save. Press Ctrl+C to stop. Works on both Windows PowerShell 5.1 and pwsh.
# For anything beyond these presets, use scripts\verify.cmd directly.

[CmdletBinding()]
param(
    [string]$Filter = "",
    [switch]$Harness,
    [switch]$Check,
    [switch]$Clear
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

# ── resolve the lane once ────────────────────────────────────────────────────

$title = if ($Filter) {
    "focused tests: $Filter"
} elseif ($Harness) {
    "harness smoke contract"
} elseif ($Check) {
    "type-check (lib + harness)"
} else {
    "fast lib tests (soak excluded)"
}

# Soak tests carry the "soak" substring by convention; fast lanes skip them
# with a substring filter so renames cannot silently un-exclude them.
$cargoArgs = if ($Filter) {
    @("test", "--locked", "--lib", "--quiet", $Filter)
} elseif ($Harness) {
    @("test", "--locked", "--quiet", "--example", "gameplay_harness",
        "tests::smoke_mode_covers_canonical_paths", "--", "--ignored", "--exact")
} elseif ($Check) {
    @("check", "--locked", "--quiet", "--lib", "--example", "gameplay_harness")
} else {
    @("test", "--locked", "--lib", "--quiet", "--", "--skip", "soak")
}

# Warm-cache reference (no file changed): check 0.06s / test 0.11s.
# The per-run timing printed below tells you whether a lane paid a rebuild.
$laneIsCheck = $Check

function Invoke-Lane {
    $env:CARGO_INCREMENTAL = "0"
    $sw = [System.Diagnostics.Stopwatch]::StartNew()
    $output = & cargo @cargoArgs 2>&1 | Out-String
    $exit = $LASTEXITCODE
    $sw.Stop()
    # Extract test count from cargo output for concise summary.
    $testCount = ""
    if (-not $laneIsCheck) {
        $m = [regex]::Match($output, '(\d+) passed')
        if ($m.Success) { $testCount = "  $($m.Groups[1].Value) passed" }
        $fm = [regex]::Match($output, '(\d+) failed')
        if ($fm.Success) { $testCount += "  $($fm.Groups[1].Value) FAILED" }
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
    if ($path -notmatch '[\\/]target[\\/]' -and $path -match '\.(rs|toml|md)$') {
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
    Write-Host "  tip:  .\scripts\watch.cmd -Check  for type-check only (fastest)" -ForegroundColor DarkGray

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
        if ($result.Output -and ($result.Exit -ne 0 -or $Filter)) {
            # Cap watch failure output so the terminal stays scannable.
            $lines = $result.Output -split "`n"
            if ($lines.Count -gt 60) {
                $lines = $lines[0..59] + @("  ... ($($lines.Count - 60) more lines; re-run with cargo test -- --nocapture)")
            }
            Write-Host ($lines -join "`n") -ForegroundColor DarkGray
        } elseif ($result.Exit -eq 0 -and $result.Seconds -gt 3) {
            Write-Host "  (rebuild: $($result.Seconds)s -- file change triggered recompile)" -ForegroundColor DarkGray
        } elseif ($result.Exit -eq 0) {
            # Warm cache hit — confirm the lane is fast.
            Write-Host "  (warm cache)" -ForegroundColor DarkGray
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

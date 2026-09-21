# rust-diagnostics.ps1 -- bounded, project-safe entry points for optional Rust diagnostics.
#
# These commands are review aids, not verification gates. The wrapper keeps the useful defaults
# local to this repository and, most importantly, gives every executing cargo-mutants run a
# unique ignored output directory so concurrent agents cannot overwrite each other's evidence.

[CmdletBinding()]
param(
    [Parameter(Mandatory = $true, Position = 0)]
    [ValidateSet("Modules", "Orphans", "MutantsList", "Mutants", "Expand")]
    [string]$Mode,

    [string]$Focus = "",
    [ValidateRange(1, 8)]
    [int]$MaxDepth = 4,

    [string]$File = "",
    [string]$Filter = "",
    [string]$TestFilter = "",
    [string]$Name = "",
    [ValidateRange(1, 8)]
    [int]$Jobs = 2,

    [string]$Item = "",
    [ValidateRange(0, 20)]
    [int]$Context = 4,
    [ValidateRange(20, 500)]
    [int]$MaxLines = 200
)

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

function Invoke-CargoDiagnostic {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    & cargo @Arguments
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
}

function Get-CargoDiagnosticOutput {
    param([Parameter(Mandatory = $true)][string[]]$Arguments)
    $output = @(& cargo @Arguments)
    if ($LASTEXITCODE -ne 0) {
        exit $LASTEXITCODE
    }
    return $output
}

function Write-BoundedExpandedOutput {
    param(
        [Parameter(Mandatory = $true)][object[]]$Lines,
        [string]$Pattern = "",
        [int]$SurroundingLines = 4,
        [int]$LineLimit = 200
    )

    if (-not $Pattern) {
        $count = [Math]::Min($Lines.Count, $LineLimit)
        for ($index = 0; $index -lt $count; $index++) {
            Write-Output $Lines[$index]
        }
        if ($Lines.Count -gt $LineLimit) {
            Write-Host "... expansion truncated at $LineLimit lines; pass -Filter <regex> or call cargo expand directly for an intentional full dump." -ForegroundColor DarkGray
        }
        return
    }

    $selected = @{}
    for ($index = 0; $index -lt $Lines.Count; $index++) {
        if ([string]$Lines[$index] -notmatch $Pattern) {
            continue
        }
        $start = [Math]::Max(0, $index - $SurroundingLines)
        $end = [Math]::Min($Lines.Count - 1, $index + $SurroundingLines)
        for ($contextIndex = $start; $contextIndex -le $end; $contextIndex++) {
            $selected[$contextIndex] = $true
        }
    }
    if ($selected.Count -eq 0) {
        throw "Expand filter '$Pattern' matched no generated lines."
    }

    $indices = @($selected.Keys | Sort-Object)
    $printed = 0
    $emittedIndices = 0
    $previous = -2
    foreach ($index in $indices) {
        if ($printed -ge $LineLimit) {
            break
        }
        if ($index -gt ($previous + 1)) {
            Write-Output "..."
            $printed += 1
            if ($printed -ge $LineLimit) {
                break
            }
        }
        Write-Output $Lines[$index]
        $printed += 1
        $emittedIndices += 1
        $previous = $index
    }
    if ($emittedIndices -lt $indices.Count) {
        Write-Host "... filtered expansion truncated at $LineLimit lines; narrow -Filter or increase -MaxLines deliberately." -ForegroundColor DarkGray
    }
}

switch ($Mode) {
    "Modules" {
        $arguments = @(
            "modules", "structure", "--lib", "--no-fns", "--no-traits", "--no-types",
            "--max-depth", "$MaxDepth"
        )
        if ($Focus) {
            $arguments += @("--focus-on", $Focus)
        }
        Invoke-CargoDiagnostic $arguments
    }

    "Orphans" {
        # Split tests.rs modules are linked only under #[cfg(test)], so cfg-test is mandatory for
        # an honest orphan scan in this repository.
        Invoke-CargoDiagnostic @("modules", "orphans", "--lib", "--cfg-test")
    }

    "MutantsList" {
        if (-not $File) {
            throw "MutantsList requires -File <owner.rs>."
        }
        $arguments = @("mutants", "--list", "--file", $File)
        if ($Filter) {
            $arguments += @("--re", $Filter)
        }
        Invoke-CargoDiagnostic $arguments
    }

    "Mutants" {
        if (-not $File) {
            throw "Mutants requires -File <owner.rs>."
        }
        if (-not $Filter) {
            throw "Mutants requires a narrow -Filter regex; broad mutation sweeps are not this project's default workflow."
        }

        $label = if ($Name) { $Name } else { [IO.Path]::GetFileNameWithoutExtension($File) }
        $label = ($label -replace '[^A-Za-z0-9._-]', '-').Trim('-')
        if (-not $label) { $label = "mutation" }
        $stamp = Get-Date -Format "yyyyMMdd-HHmmssfff"
        $output = "target/agent-output/mutants/$label-$stamp-$PID"
        New-Item -ItemType Directory -Force -Path $output | Out-Null

        Write-Host "cargo-mutants output: $output/mutants.out" -ForegroundColor DarkGray
        $arguments = @(
            "mutants", "--file", $File, "--re", $Filter, "--jobs", "$Jobs",
            "--output", $output
        )
        if ($TestFilter) {
            $arguments += @("--", $TestFilter)
        }
        Invoke-CargoDiagnostic $arguments
    }

    "Expand" {
        if (-not $Item) {
            throw "Expand requires -Item <module::item>."
        }
        $expanded = @(Get-CargoDiagnosticOutput @("expand", "--lib", $Item))
        Write-BoundedExpandedOutput -Lines $expanded -Pattern $Filter -SurroundingLines $Context -LineLimit $MaxLines
    }
}

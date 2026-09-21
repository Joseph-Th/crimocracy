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

    [string]$Item = ""
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
        Invoke-CargoDiagnostic @("expand", "--lib", $Item)
    }
}

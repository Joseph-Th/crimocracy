# check-docs.ps1 -- compile-free documentation and command-surface contracts.

$ErrorActionPreference = "Stop"
Set-Location (Split-Path -Parent $PSScriptRoot)

function Fail-Docs([string]$Message) {
    Write-Host "DOCS FAIL  $Message" -ForegroundColor Red
    exit 1
}

$requiredDocuments = @(
    "README.md",
    "AGENTS.md",
    "ARCHITECTURE.md",
    "STATUS.md",
    "TESTING.md",
    "GAME_DESIGN.md"
)
foreach ($relative in $requiredDocuments) {
    if (-not (Test-Path -LiteralPath $relative -PathType Leaf)) {
        Fail-Docs "required current document '$relative' does not exist"
    }
}

$documents = [ordered]@{
    "README.md"       = Get-Content README.md -Raw
    "AGENTS.md"       = Get-Content AGENTS.md -Raw
    "ARCHITECTURE.md" = Get-Content ARCHITECTURE.md -Raw
    "STATUS.md"       = Get-Content STATUS.md -Raw
    "TESTING.md"      = Get-Content TESTING.md -Raw
    "GAME_DESIGN.md"  = Get-Content GAME_DESIGN.md -Raw
}
$root = [IO.Path]::GetFullPath((Get-Location).Path)
$rootPrefix = $root.TrimEnd('\', '/') + [IO.Path]::DirectorySeparatorChar

foreach ($relative in $documents.Keys) {
    if ($relative -ne "README.md" -and -not $documents["README.md"].Contains($relative)) {
        Fail-Docs "README.md does not route cold readers to '$relative'"
    }
}

foreach ($entry in $documents.GetEnumerator()) {
    $relative = $entry.Key
    $parent = Split-Path -Parent ([IO.Path]::GetFullPath((Join-Path $root $relative)))
    foreach ($match in [regex]::Matches($entry.Value, '\]\(([^)]+)\)')) {
        $target = ($match.Groups[1].Value -split '\s+')[0].Trim('<', '>')
        if (-not $target -or $target.StartsWith('#') -or $target.StartsWith('/') -or
            $target.StartsWith('../') -or $target.Contains('://') -or $target.StartsWith('mailto:')) {
            continue
        }
        $target = ($target -split '#')[0]
        if (-not $target) { continue }
        $resolved = [IO.Path]::GetFullPath((Join-Path $parent $target))
        if ($resolved -ne $root -and
            -not $resolved.StartsWith($rootPrefix, [StringComparison]::OrdinalIgnoreCase)) {
            Fail-Docs "$relative local link escapes the repository: '$target'"
        }
        if (-not (Test-Path -LiteralPath $resolved)) {
            Fail-Docs "$relative links to missing local path '$target'"
        }
    }
}

$routeDocuments = @("README.md", "AGENTS.md", "ARCHITECTURE.md", "TESTING.md")
foreach ($relative in $routeDocuments) {
    foreach ($match in [regex]::Matches($documents[$relative], '\x60([^\x60]+)\x60')) {
        $value = $match.Groups[1].Value
        $isRoute = $value.StartsWith('src/') -or $value.StartsWith('scripts/') -or
            $value.StartsWith('examples/') -or $value.StartsWith('tests/') -or
            $value.StartsWith('.cargo/')
        if ($isRoute -and $value -notmatch '[\s*<>]' -and -not (Test-Path -LiteralPath $value)) {
            Fail-Docs "$relative advertises missing repository route '$value'"
        }
    }
}

$aliases = [Collections.Generic.HashSet[string]]::new([StringComparer]::Ordinal)
$inAliases = $false
foreach ($line in Get-Content .cargo/config.toml) {
    $trimmed = $line.Trim()
    if ($trimmed -eq '[alias]') {
        $inAliases = $true
        continue
    }
    if ($trimmed.StartsWith('[')) { $inAliases = $false }
    if ($inAliases -and $trimmed -match '^([A-Za-z0-9_-]+)\s*=') {
        [void]$aliases.Add($Matches[1])
    }
}
$builtins = @('build', 'check', 'clippy', 'doc', 'fmt', 'run', 'test')
$externals = @('expand', 'modules', 'mutants')
foreach ($entry in $documents.GetEnumerator()) {
    foreach ($match in [regex]::Matches($entry.Value, '\x60cargo\s+([^\s\x60]+)[^\x60]*\x60')) {
        $command = $match.Groups[1].Value
        if ($command.StartsWith('-') -or $command -in $builtins -or $command -in $externals) {
            continue
        }
        if (-not $aliases.Contains($command)) {
            Fail-Docs "$($entry.Key) advertises 'cargo $command' but .cargo/config.toml has no such alias"
        }
    }
}

$stateSource = Get-Content src/core/state.rs -Raw
$contentSource = Get-Content src/content/mod.rs -Raw
$status = $documents["STATUS.md"]
if ($stateSource -notmatch 'CURRENT_STATE_SCHEMA_VERSION:\s*u16\s*=\s*(\d+)') {
    Fail-Docs "could not read CURRENT_STATE_SCHEMA_VERSION from src/core/state.rs"
}
$stateVersion = [int]$Matches[1]
if ($contentSource -notmatch 'CURRENT_CONTENT_REVISION:\s*u32\s*=\s*(\d+)') {
    Fail-Docs "could not read CURRENT_CONTENT_REVISION from src/content/mod.rs"
}
$contentRevision = [int]$Matches[1]
if ($status -notmatch 'The current in-memory state schema version is\s+(\d+)\.') {
    Fail-Docs "STATUS.md must publish the current in-memory state schema version"
}
$publishedStateVersion = [int]$Matches[1]
if ($status -notmatch 'The current authored content revision is\s+(\d+)\.') {
    Fail-Docs "STATUS.md must publish the current authored content revision"
}
$publishedContentRevision = [int]$Matches[1]
if ($stateVersion -ne $publishedStateVersion) {
    Fail-Docs "STATUS.md state schema version is stale ($publishedStateVersion != $stateVersion)"
}
if ($contentRevision -ne $publishedContentRevision) {
    Fail-Docs "STATUS.md authored content revision is stale ($publishedContentRevision != $contentRevision)"
}

$versionMarkers = @(
    'CURRENT_CONTENT_REVISION',
    'CURRENT_STATE_SCHEMA_VERSION',
    'content_revision:',
    'schema check',
    'revision check'
)
foreach ($entry in $documents.GetEnumerator()) {
    if ($entry.Key -eq 'STATUS.md') { continue }
    foreach ($line in ($entry.Value -split "\r?\n")) {
        foreach ($marker in $versionMarkers) {
            $index = $line.IndexOf($marker, [StringComparison]::Ordinal)
            if ($index -lt 0) { continue }
            $suffix = $line.Substring($index + $marker.Length).TrimStart()
            $suffix = $suffix.TrimStart('=', ':', '(', '{', '[').TrimStart()
            if ($suffix -match '^\d') {
                Fail-Docs "$($entry.Key) duplicates a mutable persistence version after '$marker': $line"
            }
        }
    }
}

Write-Output "DOCS PASS  $($documents.Count) authorities; links, routes, Cargo aliases, and published versions are current"

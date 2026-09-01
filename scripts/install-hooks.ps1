# install-hooks.ps1 -- install local git hooks for solo verification.
#
# These hooks are local-only (not committed as active hooks) and advisory:
# a solo developer can always --no-verify if a hook is stale or slow.
#
#   pre-commit: fast lane (fmt + lib tests --skip soak)  ~0.7s warm
#   pre-push:   full gate (fmt -> tests -> harness -> clippy)  ~2-3s warm
#
# Install:   powershell -File scripts/install-hooks.ps1
# Remove:    powershell -File scripts/install-hooks.ps1 -Remove
# The hooks use pwsh if available, falling back to powershell.

[CmdletBinding()]
param([switch]$Remove)

$ErrorActionPreference = "Stop"
$hooksDir = Join-Path (Split-Path -Parent $PSScriptRoot) ".git/hooks"

if ($Remove) {
    foreach ($hook in @("pre-commit", "pre-push")) {
        $path = Join-Path $hooksDir $hook
        if (Test-Path $path) {
            Remove-Item $path -Force
            Write-Host "removed $hook" -ForegroundColor Yellow
        }
    }
    Write-Host "hooks removed" -ForegroundColor Green
    exit 0
}

if (-not (Test-Path $hooksDir)) {
    Write-Host "[FAIL] .git/hooks not found -- is this a git repo?" -ForegroundColor Red
    exit 1
}

# ── pre-commit: fast lane ──────────────────────────────────────────────────

$preCommit = @'
#!/bin/sh
# Local pre-commit hook: fast lane (fmt + lib tests --skip soak).
# Bypass with: git commit --no-verify
# Reinstall: powershell -File scripts/install-hooks.ps1

echo "pre-commit: fast lane (fmt + lib --skip soak)..."
if command -v pwsh >/dev/null 2>&1; then
    pwsh -NoProfile -NoLogo -ExecutionPolicy Bypass -File scripts/verify.ps1 -Fast
else
    powershell -NoProfile -NoLogo -ExecutionPolicy Bypass -File scripts/verify.ps1 -Fast
fi
code=$?
if [ $code -ne 0 ]; then
    echo "pre-commit hook failed (exit $code) -- fix or use --no-verify to bypass"
    exit $code
fi
'@

# ── pre-push: full gate ────────────────────────────────────────────────────

$prePush = @'
#!/bin/sh
# Local pre-push hook: full gate (fmt -> tests -> harness -> clippy).
# Bypass with: git push --no-verify
# Reinstall: powershell -File scripts/install-hooks.ps1

echo "pre-push: full gate (fmt -> tests -> harness -> clippy)..."
if command -v pwsh >/dev/null 2>&1; then
    pwsh -NoProfile -NoLogo -ExecutionPolicy Bypass -File scripts/verify.ps1
else
    powershell -NoProfile -NoLogo -ExecutionPolicy Bypass -File scripts/verify.ps1
fi
code=$?
if [ $code -ne 0 ]; then
    echo "pre-push hook failed (exit $code) -- fix or use --no-verify to bypass"
    exit $code
fi
'@

Set-Content -Path (Join-Path $hooksDir "pre-commit") -Value $preCommit -NoNewline
Set-Content -Path (Join-Path $hooksDir "pre-push") -Value $prePush -NoNewline

Write-Host "installed pre-commit (fast lane) and pre-push (full gate)" -ForegroundColor Green
Write-Host "  bypass any hook with --no-verify" -ForegroundColor DarkGray
Write-Host "  remove with: powershell -File scripts/install-hooks.ps1 -Remove" -ForegroundColor DarkGray

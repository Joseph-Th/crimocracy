@echo off
where pwsh >nul 2>nul && (pwsh -NoProfile -NoLogo -ExecutionPolicy Bypass -File "%~dp0verify.ps1" %*) || (powershell -NoProfile -NoLogo -ExecutionPolicy Bypass -File "%~dp0verify.ps1" %*)
exit /b %ERRORLEVEL%

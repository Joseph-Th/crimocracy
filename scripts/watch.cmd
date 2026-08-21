@echo off
where pwsh >nul 2>nul && (pwsh -NoProfile -NoLogo -ExecutionPolicy Bypass -File "%~dp0watch.ps1" %*) || (powershell -NoProfile -NoLogo -ExecutionPolicy Bypass -File "%~dp0watch.ps1" %*)
exit /b %ERRORLEVEL%

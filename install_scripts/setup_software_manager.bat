@echo off
setlocal
rem Double-click launcher for setup_software_manager.ps1. Runs it without requiring a change to the
rem PowerShell execution policy, which otherwise blocks scripts extracted from a downloaded zip.
powershell -NoProfile -ExecutionPolicy Bypass -File "%~dp0setup_software_manager.ps1" %*
set "setup_exit_code=%errorlevel%"
pause
exit /b %setup_exit_code%

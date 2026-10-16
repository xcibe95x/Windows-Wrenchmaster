@echo off
setlocal
cd /d "%~dp0"

where powershell.exe >nul 2>nul
if errorlevel 1 (
    echo PowerShell was not found.
    pause
    exit /b 1
)

set "CARGO_TARGET_DIR=%~dp0target"
set "WRENCHMASTER_DEV=1"
set "WRENCHMASTER_NO_UAC=1"
powershell.exe -NoProfile -ExecutionPolicy Bypass -File "%~dp0Start Wrenchmaster Dev.ps1"
if errorlevel 1 pause

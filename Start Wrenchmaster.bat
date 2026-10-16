@echo off
setlocal
cd /d "%~dp0"

where cargo >nul 2>nul
if errorlevel 1 (
    echo Cargo was not found. Install Rust and Cargo, then try again.
    pause
    exit /b 1
)

where dotnet >nul 2>nul
if errorlevel 1 (
    echo .NET 8 SDK was not found. Install the .NET 8 SDK, then try again.
    pause
    exit /b 1
)

set "CARGO_TARGET_DIR=%~dp0target"
dotnet publish "%~dp0sensor-helper\Wrenchmaster.Sensors.csproj" --configuration Release --runtime win-x64 --self-contained true --output "%~dp0target\release\sensors"
if errorlevel 1 (
    echo Wrenchmaster sensor helper could not be published. Review the error above.
    pause
    exit /b 1
)

cargo build --release
if errorlevel 1 (
    echo Wrenchmaster could not be built. Review the error above.
    pause
    exit /b 1
)

set "WRENCHMASTER_EXE=%~dp0target\release\wrenchmaster.exe"
set "WRENCHMASTER_DIR=%~dp0"
rem The flag tells the app this is already a UAC handoff, so a declined prompt
rem does not send it round the loop again; NO_UAC keeps it from re-prompting.
set "WRENCHMASTER_NO_UAC=1"
powershell.exe -NoProfile -Command "$ErrorActionPreference = 'Stop'; try { Start-Process -FilePath $env:WRENCHMASTER_EXE -WorkingDirectory $env:WRENCHMASTER_DIR -ArgumentList '--wrenchmaster-uac-relaunch' -WindowStyle Normal -Verb RunAs; exit 0 } catch { Write-Error $_; exit 1 }"
if errorlevel 1 (
    echo Windows could not start Wrenchmaster. Check that the administrator prompt was accepted.
    pause
    exit /b 1
)

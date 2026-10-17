# Wrenchmaster hot reload.
#
# Build, run, and restart the app whenever a source file is saved. Run it with
#   .\dev.bat          (double-click friendly)
#   pwsh .\dev.ps1     (or powershell -ExecutionPolicy Bypass -File .\dev.ps1)
#
# WRENCHMASTER_DEV is what keeps this loop working: it turns off the
# single-instance guard, so a rebuilt copy starting while the previous one is
# still closing replaces it instead of raising the old window and exiting.
# WRENCHMASTER_NO_UAC keeps the app unelevated, so every restart is silent and
# nothing holds a lock on the exe that this script cannot release.

$ErrorActionPreference = 'Stop'
Set-Location -LiteralPath $PSScriptRoot
$env:CARGO_TARGET_DIR = Join-Path $PSScriptRoot 'target'
$env:WRENCHMASTER_DEV = '1'
$env:WRENCHMASTER_NO_UAC = '1'

$exe = Join-Path $PSScriptRoot 'target\debug\wrenchmaster.exe'
$watched = @('src', 'assets', 'Cargo.toml', 'Cargo.lock')
$app = $null

# One loop at a time, and this is not a nicety. Two copies of this script are
# actively destructive to each other: each one sweeps up stray builds of the
# exe before it starts its own, so loop A starts the app and loop B kills it a
# moment later. The symptom is a reload that reports success and leaves no
# window open, which is maddening to diagnose from the inside.
$loopLock = New-Object System.Threading.Mutex($false, 'Local\WrenchmasterDevLoop')
if (-not $loopLock.WaitOne(0)) {
    Write-Host 'Another hot reload loop is already running in this session.' -ForegroundColor Yellow
    Write-Host 'Close that window, or use it instead of starting a second one.' -ForegroundColor Yellow
    exit 1
}

# The sensor helper is a separate .NET build and almost never changes, so it is
# published once here rather than on every reload.
$sensors = Join-Path $PSScriptRoot 'target\debug\sensors'
if (-not (Test-Path -LiteralPath $sensors)) {
    Write-Host 'Publishing the sensor helper (first run only)...' -ForegroundColor Cyan
    & dotnet publish (Join-Path $PSScriptRoot 'sensor-helper\Wrenchmaster.Sensors.csproj') `
        --configuration Debug --runtime win-x64 --self-contained true --output $sensors
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'Sensor helper publish failed. Temperatures will be unavailable.' -ForegroundColor Yellow
    }
}

function Get-Stamp {
    # One string that changes whenever any watched file does. Cheaper and more
    # predictable than a FileSystemWatcher, which fires several times per save
    # and needs its own debounce anyway.
    $items = foreach ($path in $watched) {
        $full = Join-Path $PSScriptRoot $path
        if (Test-Path -LiteralPath $full -PathType Container) {
            Get-ChildItem -LiteralPath $full -File -Recurse
        } elseif (Test-Path -LiteralPath $full -PathType Leaf) {
            Get-Item -LiteralPath $full
        }
    }
    ($items | Sort-Object FullName | ForEach-Object {
        '{0}|{1}' -f $_.FullName, $_.LastWriteTimeUtc.Ticks
    }) -join "`n"
}

function Stop-App {
    if ($null -ne $app -and -not $app.HasExited) {
        # Ask first, insist after a second: the window has to be gone before
        # cargo can overwrite the exe it is running from.
        [void]$app.CloseMainWindow()
        if (-not $app.WaitForExit(1000)) { $app.Kill(); [void]$app.WaitForExit() }
    }
    $script:app = $null

    # Then anything else running from the same exe. One missed restart leaves
    # a copy behind, and from then on every reload adds another window while
    # the stale one keeps holding the exe so the build fails -- which is how
    # a reload loop turns into a screen full of Wrenchmasters.
    #
    # Matched on Path, so this only ever touches builds from this folder and
    # never a copy the user installed or started themselves.
    foreach ($stray in @(Get-Process -Name 'wrenchmaster' -ErrorAction SilentlyContinue)) {
        try {
            if ($stray.Path -eq $exe) {
                $stray.Kill()
                [void]$stray.WaitForExit(2000)
            }
        } catch {
            # An elevated copy cannot be killed from here. Say so once rather
            # than looping on a build that is about to fail for this reason.
            Write-Host ('Could not close pid {0}; close it yourself.' -f $stray.Id) -ForegroundColor Yellow
        }
    }
}

function Start-App {
    Write-Host 'Building...' -ForegroundColor Cyan
    & cargo build
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'Build failed. Fix it and save again.' -ForegroundColor Red
        return
    }
    # -NoNewWindow, NOT -WindowStyle Hidden.
    #
    # A debug build is a console-subsystem binary (release sets
    # windows_subsystem, debug deliberately does not, so panics have somewhere
    # to print), so launching it plainly throws another console window on
    # screen at every reload. Hiding it looked like the fix and was not:
    # -WindowStyle Hidden puts SW_HIDE into the STARTUPINFO handed to the
    # process, and a window created with SW_SHOWDEFAULT -- which is what the
    # windowing layer under egui asks for -- is documented to take its show
    # state from exactly that field. So the app started, logged happily, and
    # never showed its window: a running pid and nothing on screen.
    #
    # -NoNewWindow instead lets the app share this console, so no second
    # window appears and the app's own window opens normally.
    $script:app = Start-Process -FilePath $exe -WorkingDirectory $PSScriptRoot -PassThru -NoNewWindow
    Write-Host ('Running (pid {0}). Watching {1}.' -f $app.Id, ($watched -join ', ')) -ForegroundColor Green
}

try {
    # Clear anything left over from a previous session before the first build,
    # or that copy holds the exe and the build fails before it starts.
    Stop-App
    Start-App
    $stamp = Get-Stamp
    $retryAt = $null
    $interactive = -not [Console]::IsInputRedirected
    if ($interactive) {
        Write-Host 'Press R to rebuild now, Q to quit.' -ForegroundColor DarkGray
    }
    while ($true) {
        Start-Sleep -Milliseconds 400

        # A manual reload, for when a build failed on a locked exe or the app
        # died on its own: the watcher only fires on a file change, so without
        # this the way to get a window back is to touch a file.
        if ($interactive -and [Console]::KeyAvailable) {
            $key = [Console]::ReadKey($true)
            if ($key.Key -eq 'Q') {
                Write-Host 'Stopping.' -ForegroundColor Cyan
                break
            }
            if ($key.Key -eq 'R') {
                Write-Host ''
                Write-Host 'Forced reload.' -ForegroundColor Cyan
                Stop-App
                Start-App
                $retryAt = if ($null -eq $app) { (Get-Date).AddSeconds(5) } else { $null }
                $stamp = Get-Stamp
                continue
            }
        }

        # A build can fail for a reason that has nothing to do with the code --
        # most often the exe still being held by a copy that had not finished
        # closing. Retrying a few seconds later costs nothing and saves the
        # user from having to touch a file to get their app back.
        if ($null -eq $app -and $null -ne $retryAt -and (Get-Date) -ge $retryAt) {
            $retryAt = $null
            Start-App
            if ($null -eq $app) { $retryAt = (Get-Date).AddSeconds(5) }
            $stamp = Get-Stamp
            continue
        }

        $now = Get-Stamp
        if ($now -ceq $stamp) { continue }

        # Editors write and then reformat; wait for the file to settle rather
        # than building against a half-written save.
        Start-Sleep -Milliseconds 600
        $stamp = Get-Stamp

        Write-Host ''
        Write-Host 'Change detected.' -ForegroundColor Cyan
        Stop-App
        Start-App
        # Start-App leaves $app null when the build failed. Arm the retry so a
        # transient failure does not leave the loop sitting there with nothing
        # running until the next save.
        $retryAt = if ($null -eq $app) { (Get-Date).AddSeconds(5) } else { $null }
        $stamp = Get-Stamp
    }
} finally {
    Stop-App
    $loopLock.ReleaseMutex()
    $loopLock.Dispose()
}

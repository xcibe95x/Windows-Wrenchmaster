$ErrorActionPreference = 'Stop'
$root = $PSScriptRoot
$exe = Join-Path $root 'target\debug\wrenchmaster.exe'
$env:CARGO_TARGET_DIR = Join-Path $root 'target'
$env:WRENCHMASTER_DEV = '1'
$env:WRENCHMASTER_NO_UAC = '1'
Set-Location -LiteralPath $root

function Get-SourceSnapshot {
    $files = @()
    foreach ($path in @((Join-Path $root 'Cargo.toml'), (Join-Path $root 'Cargo.lock'))) {
        if (Test-Path -LiteralPath $path -PathType Leaf) {
            $files += Get-Item -LiteralPath $path
        }
    }
    foreach ($directory in @((Join-Path $root 'src'), (Join-Path $root 'assets'), (Join-Path $root 'sensor-helper'))) {
        if (Test-Path -LiteralPath $directory -PathType Container) {
            $files += Get-ChildItem -LiteralPath $directory -File -Recurse |
                Where-Object { $_.FullName -notmatch '[\\/](bin|obj)[\\/]' }
        }
    }
    return (($files | Sort-Object FullName | ForEach-Object {
        '{0}|{1}|{2}' -f $_.FullName, $_.Length, $_.LastWriteTimeUtc.Ticks
    }) -join "`n")
}

function Build-DevApp {
    Write-Host 'Publishing Wrenchmaster sensor helper...'
    & dotnet publish (Join-Path $root 'sensor-helper\Wrenchmaster.Sensors.csproj') --configuration Debug --runtime win-x64 --self-contained true --output (Join-Path $root 'target\debug\sensors')
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'Sensor helper publish failed. Restore .NET packages and try again.' -ForegroundColor Red
        return $false
    }

    Write-Host 'Building Wrenchmaster dev version...'
    & cargo build
    if ($LASTEXITCODE -ne 0) {
        Write-Host 'Build failed. Fix the errors and save a source file to try again.' -ForegroundColor Red
        return $false
    }
    return $true
}

function Stop-DevApp($process) {
    if ($null -ne $process -and -not $process.HasExited) {
        Write-Host 'Closing the running dev window for the rebuild...'
        $process.CloseMainWindow() | Out-Null
        if (-not $process.WaitForExit(2000)) {
            $process.Kill()
            $process.WaitForExit()
        }
    }
}

$lastSnapshot = Get-SourceSnapshot
$process = $null
if ((Build-DevApp) -and (Test-Path -LiteralPath $exe)) {
    $process = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru
    Write-Host 'Wrenchmaster dev is running. Save a file under src or assets to rebuild and restart.' -ForegroundColor Cyan
}

while ($true) {
    if ($null -ne $process -and -not $process.HasExited) {
        [void]$process.WaitForExit(400)
    } else {
        Start-Sleep -Milliseconds 400
        $process = $null
    }

    $snapshot = Get-SourceSnapshot
    if ($snapshot -ceq $lastSnapshot) { continue }

    # Wait for an editor's save/format operation to finish before compiling.
    Start-Sleep -Milliseconds 700
    $snapshot = Get-SourceSnapshot
    if ($snapshot -ceq $lastSnapshot) { continue }
    $lastSnapshot = $snapshot

    Stop-DevApp $process
    $process = $null
    if (Build-DevApp) {
        $lastSnapshot = Get-SourceSnapshot
        if (Test-Path -LiteralPath $exe) {
            $process = Start-Process -FilePath $exe -WorkingDirectory $root -PassThru
            Write-Host 'Wrenchmaster restarted with your latest changes.' -ForegroundColor Green
        }
    }
}

<#
.SYNOPSIS
    Remove the app installed by install.ps1 (keeps settings and logs by default).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\uninstall.ps1
    powershell -ExecutionPolicy Bypass -File scripts\uninstall.ps1 -RemoveSettings
#>
[CmdletBinding()]
param(
    [switch]$RemoveSettings
)

$ErrorActionPreference = 'Stop'

Get-Process -Name 'win-island-plus' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400

$targetDir = Join-Path $env:LOCALAPPDATA 'Programs\WinIslandPlus'
if (Test-Path $targetDir) {
    Remove-Item $targetDir -Recurse -Force
    Write-Host "Removed: $targetDir"
}

$shortcutPath = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs\WinIslandPlus.lnk'
if (Test-Path $shortcutPath) {
    Remove-Item $shortcutPath -Force
    Write-Host "Removed shortcut"
}

$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if (Get-ItemProperty -Path $runKey -Name 'WinIslandPlus' -ErrorAction SilentlyContinue) {
    Remove-ItemProperty -Path $runKey -Name 'WinIslandPlus'
    Write-Host 'Autostart: removed'
}

if ($RemoveSettings) {
    $dataDir = Join-Path $env:LOCALAPPDATA 'WinIslandPlus'
    if (Test-Path $dataDir) {
        Remove-Item $dataDir -Recurse -Force
        Write-Host "Removed data: $dataDir"
    }
}
else {
    Write-Host "Settings kept at $env:LOCALAPPDATA\WinIslandPlus (use -RemoveSettings to delete)"
}

Write-Host 'Uninstall done.'

<#
.SYNOPSIS
    Install the built app into %LOCALAPPDATA%\Programs\WinIslandPlus without admin rights.

.DESCRIPTION
    Copies the executable produced by `pnpm tauri build`, creates a Start Menu shortcut,
    optionally registers autostart (HKCU Run) and launches the app.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1
    powershell -ExecutionPolicy Bypass -File scripts\install.ps1 -AutoStart
#>
[CmdletBinding()]
param(
    [switch]$AutoStart,
    [switch]$NoLaunch
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSCommandPath | Split-Path -Parent

$source = Join-Path $root 'src-tauri\target\release\win-island-plus.exe'
if (-not (Test-Path $source)) {
    $source = Join-Path $root 'src-tauri\target\debug\win-island-plus.exe'
}
if (-not (Test-Path $source)) {
    throw "Build the app first: powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1"
}

$targetDir = Join-Path $env:LOCALAPPDATA 'Programs\WinIslandPlus'
$targetExe = Join-Path $targetDir 'WinIslandPlus.exe'

Get-Process -Name 'win-island-plus' -ErrorAction SilentlyContinue | Stop-Process -Force
Start-Sleep -Milliseconds 400

if (-not (Test-Path $targetDir)) {
    New-Item -ItemType Directory -Path $targetDir -Force | Out-Null
}

Copy-Item $source $targetExe -Force
Write-Host "Installed: $targetExe"

# Start Menu shortcut
$startMenu = Join-Path $env:APPDATA 'Microsoft\Windows\Start Menu\Programs'
$shortcutPath = Join-Path $startMenu 'WinIslandPlus.lnk'
$shell = New-Object -ComObject WScript.Shell
$shortcut = $shell.CreateShortcut($shortcutPath)
$shortcut.TargetPath = $targetExe
$shortcut.WorkingDirectory = $targetDir
$shortcut.Description = 'Windows 11 Dynamic Island'
$shortcut.Save()
Write-Host "Shortcut: $shortcutPath"

# Optional autostart
$runKey = 'HKCU:\Software\Microsoft\Windows\CurrentVersion\Run'
if ($AutoStart) {
    Set-ItemProperty -Path $runKey -Name 'WinIslandPlus' -Value "`"$targetExe`" --autostart"
    Write-Host 'Autostart: enabled'
}
else {
    Write-Host 'Autostart: unchanged (use -AutoStart to enable, or toggle it in settings)'
}

if (-not $NoLaunch) {
    Start-Process -FilePath $targetExe
    Write-Host 'Launched.'
}

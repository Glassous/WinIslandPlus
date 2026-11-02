<#
.SYNOPSIS
    Build the app and the NSIS installer, then copy the artifacts into .\release.

.DESCRIPTION
    Version comes from src-tauri/tauri.conf.json (and Cargo.toml) - bump it there.

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1
    powershell -ExecutionPolicy Bypass -File scripts\build-release.ps1 -DevBuild
#>
[CmdletBinding()]
param(
    [switch]$DevBuild,
    [switch]$NoBundle
)

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSCommandPath | Split-Path -Parent
$release = Join-Path $root 'release'
$profile = if ($DevBuild) { 'debug' } else { 'release' }

Push-Location $root
try {
    if (-not (Test-Path (Join-Path $root 'node_modules'))) {
        Write-Host 'Installing frontend dependencies...'
        pnpm install
    }

    $arguments = @('tauri', 'build')
    if ($DevBuild) { $arguments += '--debug' }
    if ($NoBundle) { $arguments += '--no-bundle' }

    Write-Host ("Building: pnpm " + ($arguments -join ' '))
    & pnpm @arguments
    if ($LASTEXITCODE -ne 0) {
        throw "tauri build failed with exit code $LASTEXITCODE"
    }

    if (-not (Test-Path $release)) {
        New-Item -ItemType Directory -Path $release -Force | Out-Null
    }

    $exe = Join-Path $root "src-tauri\target\$profile\win-island-plus.exe"
    if (Test-Path $exe) {
        Copy-Item $exe (Join-Path $release 'win-island-plus.exe') -Force
        Write-Host "Copied: release\win-island-plus.exe"
    }

    $bundle = Join-Path $root "src-tauri\target\$profile\bundle\nsis"
    if (Test-Path $bundle) {
        Get-ChildItem $bundle -Filter '*.exe' | ForEach-Object {
            Copy-Item $_.FullName (Join-Path $release $_.Name) -Force
            Write-Host "Copied: release\$($_.Name)"
        }
    }

    Write-Host ''
    Write-Host "Done. Artifacts in: $release"
}
finally {
    Pop-Location
}

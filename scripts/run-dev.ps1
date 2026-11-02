<#
.SYNOPSIS
    Start the development build (Vite dev server + Tauri window with hot reload).

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\run-dev.ps1
#>
[CmdletBinding()]
param()

$ErrorActionPreference = 'Stop'
$root = Split-Path -Parent $PSCommandPath | Split-Path -Parent

Push-Location $root
try {
    if (-not (Test-Path (Join-Path $root 'node_modules'))) {
        Write-Host 'Installing frontend dependencies...'
        pnpm install
    }

    pnpm tauri dev
}
finally {
    Pop-Location
}

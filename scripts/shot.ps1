<#
.SYNOPSIS
    截取主屏（或指定区域）到 PNG，便于对照验收视觉效果。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\shot.ps1
    powershell -ExecutionPolicy Bypass -File scripts\shot.ps1 -Out .\release\shot.png -Crop 0,0,1200,400
#>
[CmdletBinding()]
param(
    [string]$Out,
    [int[]]$Crop
)

$ErrorActionPreference = 'Stop'

if ([string]::IsNullOrWhiteSpace($Out)) {
    $Out = Join-Path (Split-Path -Parent $PSCommandPath) '..\release\shot.png'
}

Add-Type -AssemblyName System.Windows.Forms
Add-Type -AssemblyName System.Drawing

$bounds = [System.Windows.Forms.SystemInformation]::VirtualScreen
$x = $bounds.X
$y = $bounds.Y
$width = $bounds.Width
$height = $bounds.Height

if ($Crop -and $Crop.Count -eq 4) {
    $x = $Crop[0]
    $y = $Crop[1]
    $width = $Crop[2]
    $height = $Crop[3]
}

$directory = Split-Path -Parent $Out
if (-not (Test-Path $directory)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}

$bitmap = New-Object System.Drawing.Bitmap($width, $height)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.CopyFromScreen($x, $y, 0, 0, $bitmap.Size)
$bitmap.Save($Out, [System.Drawing.Imaging.ImageFormat]::Png)
$graphics.Dispose()
$bitmap.Dispose()

Write-Output "saved: $Out ($width x $height)"

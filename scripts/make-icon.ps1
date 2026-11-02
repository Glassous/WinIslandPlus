<#
.SYNOPSIS
    生成应用图标源文件（1024×1024 PNG），再交给 `pnpm tauri icon` 派生全套尺寸。

.DESCRIPTION
    图案与托盘图标一致：深色胶囊 + 半透明白描边 + 绿色状态点。
    不提交任何二进制资源，图标始终由脚本重新生成。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\make-icon.ps1
#>
[CmdletBinding()]
param(
    [string]$Output
)

$ErrorActionPreference = 'Stop'

# $PSScriptRoot 在 param 默认值里不可靠（Windows PowerShell 5.1），这里在函数体内补齐
if ([string]::IsNullOrWhiteSpace($Output)) {
    $Output = Join-Path (Split-Path -Parent $PSCommandPath) '..\assets\app-icon.png'
}

Add-Type -AssemblyName System.Drawing

$size = 1024
$bitmap = New-Object System.Drawing.Bitmap($size, $size, [System.Drawing.Imaging.PixelFormat]::Format32bppArgb)
$graphics = [System.Drawing.Graphics]::FromImage($bitmap)
$graphics.SmoothingMode = [System.Drawing.Drawing2D.SmoothingMode]::AntiAlias
$graphics.Clear([System.Drawing.Color]::Transparent)

# 胶囊：768×448，圆角半径 224（与云岛 2:1 的比例一致）
$capsule = New-Object System.Drawing.Rectangle(128, 288, 768, 448)
$radius = 224
$diameter = $radius * 2

$path = New-Object System.Drawing.Drawing2D.GraphicsPath
$path.AddArc($capsule.X, $capsule.Y, $diameter, $diameter, 180, 90)
$path.AddArc($capsule.Right - $diameter, $capsule.Y, $diameter, $diameter, 270, 90)
$path.AddArc($capsule.Right - $diameter, $capsule.Bottom - $diameter, $diameter, $diameter, 0, 90)
$path.AddArc($capsule.X, $capsule.Bottom - $diameter, $diameter, $diameter, 90, 90)
$path.CloseFigure()

$fill = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 22, 22, 24))
$stroke = New-Object System.Drawing.Pen([System.Drawing.Color]::FromArgb(60, 255, 255, 255), 10)
$graphics.FillPath($fill, $path)
$graphics.DrawPath($stroke, $path)

# 播放指示：绿色圆点，居中偏右
$dot = New-Object System.Drawing.SolidBrush([System.Drawing.Color]::FromArgb(255, 61, 220, 132))
$graphics.FillEllipse($dot, 512 - 80, 512 - 80, 160, 160)

$directory = Split-Path -Parent $Output
if (-not (Test-Path $directory)) {
    New-Item -ItemType Directory -Path $directory -Force | Out-Null
}

$bitmap.Save($Output, [System.Drawing.Imaging.ImageFormat]::Png)

$graphics.Dispose()
$bitmap.Dispose()
$fill.Dispose()
$stroke.Dispose()
$dot.Dispose()
$path.Dispose()

Write-Host "图标源已生成: $Output"

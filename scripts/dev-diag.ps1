<#
.SYNOPSIS
    调试探针：按进程枚举顶层窗口，输出可见性 / 位置 / 扩展样式，并可选截取岛窗区域。

.DESCRIPTION
    仅用于开发期定位"窗口到底在哪里、有没有显示"这类问题，不参与安装。

.EXAMPLE
    powershell -ExecutionPolicy Bypass -File scripts\dev-diag.ps1
    powershell -ExecutionPolicy Bypass -File scripts\dev-diag.ps1 -Shot .\release\island-area.png
#>
[CmdletBinding()]
param(
    [string]$ProcessName = 'win-island-plus',
    [string]$Shot
)

$ErrorActionPreference = 'Stop'

Add-Type -Namespace Diag -Name Native -MemberDefinition @'
[DllImport("user32.dll")] public static extern bool SetProcessDPIAware();
[DllImport("user32.dll")] public static extern bool EnumWindows(EnumProc cb, IntPtr l);
public delegate bool EnumProc(IntPtr h, IntPtr l);
[DllImport("user32.dll")] public static extern uint GetWindowThreadProcessId(IntPtr h, out uint pid);
[DllImport("user32.dll")] public static extern bool GetWindowRect(IntPtr h, out RECT r);
[DllImport("user32.dll")] public static extern bool IsWindowVisible(IntPtr h);
[DllImport("user32.dll", CharSet = CharSet.Unicode)] public static extern int GetWindowTextW(IntPtr h, System.Text.StringBuilder s, int n);
[DllImport("user32.dll")] public static extern IntPtr GetWindowLongPtrW(IntPtr h, int i);
[StructLayout(LayoutKind.Sequential)] public struct RECT { public int L; public int T; public int R; public int B; }
'@

Add-Type -AssemblyName System.Drawing

[void][Diag.Native]::SetProcessDPIAware()

$processes = Get-Process -Name $ProcessName -ErrorAction SilentlyContinue
if (-not $processes) {
    Write-Output "process '$ProcessName' not running"
    exit 1
}

$ids = @($processes | ForEach-Object { $_.Id })
$script:found = New-Object System.Collections.ArrayList

$callback = [Diag.Native+EnumProc] {
    param($h, $l)
    $owner = 0
    [void][Diag.Native]::GetWindowThreadProcessId($h, [ref]$owner)
    if ($ids -contains $owner) {
        $title = New-Object System.Text.StringBuilder 256
        [void][Diag.Native]::GetWindowTextW($h, $title, 256)
        $rect = New-Object Diag.Native+RECT
        [void][Diag.Native]::GetWindowRect($h, [ref]$rect)
        $ex = [Diag.Native]::GetWindowLongPtrW($h, -20)
        [void]$script:found.Add([pscustomobject]@{
            Hwnd    = $h
            Visible = [Diag.Native]::IsWindowVisible($h)
            ExStyle = ('0x{0:X}' -f $ex)
            Rect    = "$($rect.L),$($rect.T) - $($rect.R),$($rect.B)"
            Title   = $title.ToString()
        })
    }
    return $true
}

[void][Diag.Native]::EnumWindows($callback, [IntPtr]::Zero)
$script:found | Format-Table -AutoSize | Out-String | Write-Output

if (-not [string]::IsNullOrWhiteSpace($Shot)) {
    $target = $script:found | Where-Object { $_.Visible -and $_.Title -eq 'WinIslandPlus' } | Select-Object -First 1
    if (-not $target) {
        $target = $script:found | Where-Object { $_.Visible } | Sort-Object { $_.Rect.Length } | Select-Object -Last 1
    }

    if ($target) {
        $parts = ($target.Rect -split ' - ')
        $left, $top = $parts[0] -split ','
        $right, $bottom = $parts[1] -split ','
        $width = [int]$right - [int]$left
        $height = [int]$bottom - [int]$top

        $bitmap = New-Object System.Drawing.Bitmap($width, $height)
        $graphics = [System.Drawing.Graphics]::FromImage($bitmap)
        $graphics.CopyFromScreen([int]$left, [int]$top, 0, 0, $bitmap.Size)
        $bitmap.Save($Shot, [System.Drawing.Imaging.ImageFormat]::Png)
        $graphics.Dispose()
        $bitmap.Dispose()
        Write-Output "shot: $Shot ($width x $height)"
    }
}

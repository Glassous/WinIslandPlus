<#
.SYNOPSIS
    为 WinIslandPlus 写入 DeepSeek Harness（dsh）状态文件。

.DESCRIPTION
    岛体默认会自己从 dsh 的会话事件流里探测阶段与用量，**不需要本脚本**。
    这个写入器是可选的补充：当你想让岛体显示「会话成本」、自定义阶段文案，
    或不想让岛体自己查询余额时，再把它挂到 dsh 的阶段 hook 上。

    脚本把「当前阶段 / 会话用量 / 余额」聚合成一个原子写入的 JSON：
    默认写入 %LOCALAPPDATA%\WinIslandPlus\dsh-status.json（岛体的首选路径，
    存在且新鲜时会覆盖自动探测结果）。

.PARAMETER Stage
    idle | thinking | tool | waiting | done

.PARAMETER Label
    展示用短词；留空时由岛体按 Stage 兜底（思考中 / 调用工具 / ...）。

.PARAMETER FromLedger
    从 ~/.dsh/storages/cost-meter/ledger.json 读取指定会话（或最新会话）
    的 input / output / cacheRead / cost。

.PARAMETER FetchBalance
    用 DEEPSEEK_API_KEY 调用官方 GET /user/balance 取余额。
    仅在 AuthMode 为 official / web 时写入。

.PARAMETER Watch
    Watch 模式：每隔 N 秒刷新一次 UpdatedAtMs 心跳，并在给了 -FromLedger
    时重新汇总用量。按 Ctrl+C 结束。

.EXAMPLE
    # 每次阶段变化调用一次
    .\scripts\dsh-status.ps1 -Stage thinking -FromLedger -SessionId session-xxxx -FetchBalance

.EXAMPLE
    # 常驻心跳（dsh 运行期间保持在后台）
    .\scripts\dsh-status.ps1 -Stage tool -FromLedger -Watch 20

.EXAMPLE
    # dsh 退出
    .\scripts\dsh-status.ps1 -Off
#>
[CmdletBinding()]
param(
    [ValidateSet("idle", "thinking", "tool", "waiting", "done")]
    [string]$Stage = "idle",

    [string]$Label = "",
    [string]$Model = "",
    [string]$Provider = "deepseek-official",
    [ValidateSet("official", "web", "api")]
    [string]$AuthMode = "official",
    [string]$SessionId = "",

    [double]$InputTokens = -1,
    [double]$OutputTokens = -1,
    [double]$CacheReadTokens = -1,
    [double]$Cost = -1,

    [double]$Balance = [double]::NaN,
    [string]$BalanceCurrency = "CNY",
    [string]$Currency = "CNY",

    [switch]$FromLedger,
    [switch]$FetchBalance,

    [int]$Watch = 0,
    [switch]$Off,

    [string]$Path = ""
)

$ErrorActionPreference = "Stop"

$stageLabels = @{
    idle     = "空闲"
    thinking = "思考中"
    tool     = "调用工具"
    waiting  = "等待确认"
    done     = "已完成"
}

function Get-StatusPath {
    param([string]$Override)
    if ($Override) { return $Override }
    $root = if ($env:LOCALAPPDATA) { Join-Path $env:LOCALAPPDATA "WinIslandPlus" } else { $PWD.Path }
    return (Join-Path $root "dsh-status.json")
}

function Get-LedgerPath {
    param([string]$UserProfile)
    return (Join-Path $UserProfile ".dsh/storages/cost-meter/ledger.json")
}

<#
    ledger.json 里有庞大的价格表，PowerShell 的 ConvertFrom-Json 在这种体量下
    会失败，这里用正则只抓 sessions 数组里每条会话的汇总字段。
#>
function Read-LedgerSession {
    param(
        [string]$LedgerPath,
        [string]$Wanted
    )

    if (-not (Test-Path -LiteralPath $LedgerPath)) { return $null }

    $raw = [System.IO.File]::ReadAllText($LedgerPath)
    $pattern = '"id":"(session-[0-9a-fA-F\-]+)","input":(\d+),"output":(\d+),"cacheRead":(\d+),"cacheWrite":(\d+),"reasoning":(\d+),"calls":(\d+),"cost":([0-9.eE\+\-]+),"apiCost":([0-9.eE\+\-]+)'
    $found = [regex]::Matches($raw, $pattern)
    if ($found.Count -eq 0) { return $null }

    $target = $found[$found.Count - 1]
    if ($Wanted) {
        $match = $found | Where-Object { $_.Groups[1].Value -eq $Wanted } | Select-Object -Last 1
        if ($match) { $target = $match }
    }

    return [pscustomobject]@{
        SessionId   = $target.Groups[1].Value
        Input       = [double]$target.Groups[2].Value
        Output      = [double]$target.Groups[3].Value
        CacheRead   = [double]$target.Groups[4].Value
        Cost        = [double]$target.Groups[8].Value
    }
}

function Get-OfficialBalance {
    param([string]$Key)

    if (-not $Key) { return [double]::NaN }
    try {
        $response = Invoke-RestMethod `
            -Uri "https://api.deepseek.com/user/balance" `
            -Headers @{ Authorization = "Bearer $Key" } `
            -TimeoutSec 8
        if ($response -and $response.balance_infos -and $response.balance_infos.Count -gt 0) {
            $info = $response.balance_infos[0]
            if ($null -ne $info.total_balance) { return [double]$info.total_balance }
        }
    } catch {
        Write-Verbose "取余额失败：$($_.Exception.Message)"
    }
    return [double]::NaN
}

function Write-StatusFile {
    param(
        [string]$TargetPath,
        [hashtable]$Payload
    )

    $directory = Split-Path -Parent $TargetPath
    if ($directory -and -not (Test-Path -LiteralPath $directory)) {
        New-Item -ItemType Directory -Path $directory -Force | Out-Null
    }

    $json = $Payload | ConvertTo-Json -Depth 4
    $temp = "$TargetPath.tmp"
    $utf8 = New-Object System.Text.UTF8Encoding($false)
    [System.IO.File]::WriteAllText($temp, $json, $utf8)

    # 原子替换：岛体永不读到写了一半的 JSON
    Move-Item -LiteralPath $temp -Destination $TargetPath -Force
}

function Build-Payload {
    $ledgerPath = Get-LedgerPath -UserProfile $env:USERPROFILE
    $ledger = $null

    if ($FromLedger) {
        $ledger = Read-LedgerSession -LedgerPath $ledgerPath -Wanted $SessionId
    }

    $inputTokens = if ($InputTokens -ge 0) { $InputTokens } elseif ($ledger) { $ledger.Input } else { 0 }
    $outputTokens = if ($OutputTokens -ge 0) { $OutputTokens } elseif ($ledger) { $ledger.Output } else { 0 }
    $cacheRead = if ($CacheReadTokens -ge 0) { $CacheReadTokens } elseif ($ledger) { $ledger.CacheRead } else { 0 }
    $costValue = if ($Cost -ge 0) { $Cost } elseif ($ledger) { $ledger.Cost } else { 0 }

    $resolvedSession = if ($SessionId) { $SessionId } elseif ($ledger) { $ledger.SessionId } else { "" }

    $balanceValue = $Balance
    if ($Off -or -not ($AuthMode -eq "official" -or $AuthMode -eq "web")) {
        $balanceValue = [double]::NaN
    } elseif ([double]::IsNaN($balanceValue) -and $FetchBalance) {
        $balanceValue = Get-OfficialBalance -Key $env:DEEPSEEK_API_KEY
    }

    $payload = [ordered]@{
        Version         = 1
        Running         = (-not $Off)
        Stage           = if ($Off) { "idle" } else { $Stage }
        StageLabel      = if ($Off) { "" } elseif ($Label) { $Label } else { $stageLabels[$Stage] }
        Model           = $Model
        Provider        = $Provider
        AuthMode        = $AuthMode
        SessionId       = $resolvedSession
        InputTokens     = $inputTokens
        OutputTokens    = $outputTokens
        CacheReadTokens = $cacheRead
        SessionCost     = $costValue
        Currency        = $Currency
        Balance         = if ([double]::IsNaN($balanceValue)) { $null } else { $balanceValue }
        BalanceCurrency = $BalanceCurrency
        UpdatedAtMs     = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
    }

    return $payload
}

$target = Get-StatusPath -Override $Path

if ($Watch -le 0) {
    Write-StatusFile -TargetPath $target -Payload (Build-Payload)
    Write-Verbose "已写入 $target"
    return
}

Write-Host "dsh 状态心跳中（每 $Watch 秒）。Ctrl+C 结束。"
try {
    while ($true) {
        Write-StatusFile -TargetPath $target -Payload (Build-Payload)
        Start-Sleep -Seconds $Watch
    }
} finally {
    # Watch 退出即视为 dsh 结束，避免岛体长时间显示过期状态
    if (-not $Off) {
        $final = Build-Payload
        $final.Running = $false
        $final.UpdatedAtMs = [DateTimeOffset]::UtcNow.ToUnixTimeMilliseconds()
        Write-StatusFile -TargetPath $target -Payload $final
    }
}

# DeepSeek Harness（dsh）接入说明

岛体有两条获取 dsh 状态的通道，**自动探测优先，状态文件可选**：

| 通道 | 需要 dsh 侧配合 | 能提供 |
| --- | --- | --- |
| **自动探测**（默认） | 不需要，开箱即用 | 阶段、会话 token、模型、当前工具名、官方余额 |
| 状态文件（可选） | 插件 / hook / 脚本写 JSON | 上面全部 + 会话成本、自定义阶段文案 |

大多数情况下你什么都不用做：只要 dsh 桌面端（或 `dsh` CLI）在运行，岛体就会显示它的状态。

## 自动探测（无需配置）

1. **是否运行**：遍历进程，命中 `DeepSeek Harness.exe` / `dsh.exe` / `dsh` 即视为活跃
   （2 秒探测一次）。
2. **当前阶段**：读 `~/.dsh/sessions/<项目>/session-<id>/session.v4.jsonl.zstd` 的**尾部帧**。
   dsh 每追加一条事件就追加一个独立 zstd 帧，所以只要定位最后一个帧边界就能只解几 KB，
   不需要解整个会话文件。事件到阶段的映射：

   | 事件 | 阶段 |
   | --- | --- |
   | `tool/call` | 调用工具（同时取工具名，展开态显示） |
   | `approval/asked` | 等待确认 |
   | `assistant/message`、`tool/result`、`step/*`、`user/message` | 思考中 |
   | `turn/end` | 已完成（120 秒后回落为空闲） |

3. **用量**：会话缓存 `~/.dsh/storages/session_projcache/sessions/<id>.json` 给出历史累计，
   之后逐条累加新事件的 `usage`（input + output + cacheRead），所以数字是实时的。
4. **余额**：仅当 provider 是官方（`deepseek-official` / `deepseek-account`）时，
   每 5 分钟调用一次官方 `GET https://api.deepseek.com/user/balance`。

### 只读取这些内容

导出的快照里只有：阶段、模型名、工具名、三个 token 计数、余额数字。
**不读取、不记录、不上传任何对话正文**；日志里只出现「接入 / 断开 / 阶段切换」。

### 关于余额与凭据

余额查询需要官方 API Key，从 dsh 自己的凭据文件 `~/.dsh/.credentials.yaml` 的
`DEEPSEEK_API_KEY` 一行读取，**仅用于这一次 HTTPS 请求**，不写日志、不进快照、不落盘。
请求走系统代理（若开启）。不想让它联网，把设置里的 **DeepSeek Harness** 开关关掉即可——
关掉后既不会读会话文件，也不会发任何请求。

## 状态文件（可选，用于补充成本 / 自定义文案）

如果希望岛体显示**会话成本**，或想让阶段文案由 dsh 侧决定，就写一个本地 JSON 文件。
dsh 侧（插件 / hook / 脚本）在每次阶段变化时重写它即可。

## 文件位置

按顺序探测，命中即用：

1. `%LOCALAPPDATA%\WinIslandPlus\dsh-status.json`（首选）
2. `%USERPROFILE%\.dsh\winisland-status.json`（备选，便于 dsh 插件就近写入）

文件不存在 / 判定不活跃时，岛体立即回到纯媒体行为。

## 字段

| 字段 | 类型 | 说明 |
| --- | --- | --- |
| `Version` | number | 契约版本，当前为 `1`（保留字段，岛体暂不校验） |
| `Running` | boolean | `false` 表示 dsh 已退出，岛体立刻隐藏 dsh 信息；缺省视为运行中 |
| `Stage` | string | `idle` / `thinking` / `tool` / `waiting` / `done`；未知值按 `idle` 处理 |
| `StageLabel` | string | 展示用短词（中文）；留空时岛体按 `Stage` 兜底 |
| `Model` | string | 模型名，展开卡片里显示 |
| `Provider` | string | 例如 `deepseek-official` |
| `AuthMode` | string | `official` / `web` / `api`；**只有 `official` / `web` 才显示余额** |
| `SessionId` | string | 会话 ID（只用于展示与脚本汇总） |
| `InputTokens` | number | 会话累计输入 token |
| `OutputTokens` | number | 会话累计输出 token |
| `CacheReadTokens` | number | 会话累计缓存命中 token |
| `SessionCost` | number | 会话累计成本 |
| `Currency` | string | 成本币种，缺省 `CNY` |
| `Balance` | number \| null | 账户余额；`null` 时余额整段不占位 |
| `BalanceCurrency` | string | 余额币种，缺省 `CNY` |
| `UpdatedAtMs` | number | 写入时刻（Unix 毫秒，UTC）。缺省时按写入当下的时刻处理 |

`TotalTokens` 由岛体按 `InputTokens + OutputTokens + CacheReadTokens` 计算，文件里不需要提供。

## 示例

```json
{
  "Version": 1,
  "Running": true,
  "Stage": "tool",
  "StageLabel": "调用工具",
  "Model": "deepseek-v4-flash",
  "Provider": "deepseek-official",
  "AuthMode": "official",
  "SessionId": "session-4a22622d-a8bd-4d66-bf97-b4bc95c309fb",
  "InputTokens": 39798,
  "OutputTokens": 8558,
  "CacheReadTokens": 1057280,
  "SessionCost": 0.0436,
  "Currency": "CNY",
  "Balance": 12.3,
  "BalanceCurrency": "CNY",
  "UpdatedAtMs": 1788922696231
}
```

## 活跃判定

- `Running` 为 `false` → 立刻视为不活跃；
- 超过 **120 秒**没更新 `UpdatedAtMs` → 视为不活跃（dsh 已退出）；
- `Stage` 为 `waiting`（等待确认）时**不超时**：人在思考时岛体不该先消失；
- 文件写坏（写了一半）时沿用上一份快照，连续失败 3 次才清空，避免胶囊忽隐忽现。

## 写入器脚本

仓库里带了一个通用写入器 `scripts/dsh-status.ps1`（自动探测已经够用，这个脚本是可选的补充）：

```powershell
# 单次写入（适合挂在 dsh 的阶段 hook 上）
.\scripts\dsh-status.ps1 -Stage thinking -Model deepseek-v4-flash

# 从 cost-meter 账本汇总当前会话的 token / 成本
.\scripts\dsh-status.ps1 -Stage tool -FromLedger -SessionId session-xxxx

# 顺带取官方余额（需要 DEEPSEEK_API_KEY）
.\scripts\dsh-status.ps1 -Stage done -FromLedger -FetchBalance

# 心跳模式：每隔 20 秒刷新时间戳，退出时自动置 Running=false
.\scripts\dsh-status.ps1 -Stage thinking -FromLedger -Watch 20

# dsh 退出
.\scripts\dsh-status.ps1 -Off
```

写入器用「临时文件 + 原子替换」，岛体不会读到半截 JSON。
脚本文件以 **UTF-8 with BOM** 保存——Windows PowerShell 5.1 只有带 BOM 才能正确解析
文件里的中文注释（改用 PowerShell 7 或其它语言实现则无此限制）。

若 dsh 插件框架允许写本地文件，直接按上面的字段在阶段变化时写文件即可，
无需安装任何依赖。

## 余额

只有 `AuthMode` 为 `official` / `web` 时岛体才显示余额。余额来源是 DeepSeek 官方接口：

```
GET https://api.deepseek.com/user/balance
Authorization: Bearer <API_KEY>
```

两种取数方式：

- **自动探测**：岛体自己调用该接口（每 5 分钟一次），密钥取 dsh 凭据文件里的
  `DEEPSEEK_API_KEY`，仅用于本次请求；走系统代理；失败静默重试。
- **状态文件**：由写入方（`scripts/dsh-status.ps1 -FetchBalance` 或 dsh 插件）调用后
  把结果写进 `Balance` / `BalanceCurrency`，此时状态文件里的值优先。

## 展示规则

媒体与 dsh 是**两个彼此独立的岛**，同一行内并排（不共用一块形状）：

| 场景 | 胶囊态 | 展开态 |
| --- | --- | --- |
| 无媒体 + 有 dsh | 只有 dsh 岛：状态点 + 阶段 + 用量/余额（窄的时候依次舍去余额、成本），居中 | dsh 岛展开为信息卡，长在胶囊行的下一行 |
| 有媒体 + 有 dsh | dsh 胶囊在左、纯黑正圆在右（直径 = 胶囊高度；圆内是圆角矩形专辑图，四周留黑边；没有封面时圆内显示频谱），整组居中 | 一次只展开一个：被展开者变成卡片（媒体卡 / dsh 卡），另一个留在胶囊行居中；只剩媒体时它变回普通胶囊 |
| 只有媒体 | 与原行为完全一致（没有封面时封面让位、频谱居中） | 与原行为完全一致 |
| 无媒体 + 无 dsh | 常驻时显示一枚静止空胶囊，点击不展开 | — |

dsh 展开卡分两行，第二行允许换行：

```
[图标] ● 调用工具  pwsh
       43.9k tok   余 ¥4.61   deepseek-v4-flash
```

dsh 胶囊宽度按内容自适应，夹在 96–132 DIP 之间。切换展开对象时两个岛在同一帧里
各自补间（300 ms 缓出）：一个从卡片收回胶囊行、另一个从胶囊行展开成卡片，只做上下换位。

### 什么时候显示用量

用量只在**这个会话刚刚还在活动**时显示（最近事件或刚被切换/打开，2 分钟内）：

- 新开一个空对话、或者会话早就结束 → 只显示「空闲 + 余额」，不会挂着上一次的 token 数；
- 一轮对话刚结束 → 保留 120 秒的「已完成 + 最终用量」，然后回到空闲并隐藏用量；
- 「等待确认」不会超时消失；
- 「思考中 / 调用工具」超过 5 分钟没有新事件 → 视为已结束，回落成空闲。

## 关闭

设置窗口 → 行为 → **DeepSeek Harness** 开关（对应 `settings.json` 的 `DshEnabled`）。
关闭后岛体不再读取状态文件、不再查询余额，dsh 岛整体消失（媒体岛回到单独的居中胶囊）。

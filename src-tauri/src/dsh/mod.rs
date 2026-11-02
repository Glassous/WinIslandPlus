//! DeepSeek Harness（dsh）状态接入。
//!
//! 只读取**本地状态文件**，不做任何网络请求、不读取任何凭据：
//!
//! 1. `%LOCALAPPDATA%\WinIslandPlus\dsh-status.json`（首选，与 settings.json 同目录）
//! 2. `%USERPROFILE%\.dsh\winisland-status.json`（备选，便于 dsh 插件就近写入）
//!
//! 与 `media` 模块同构：独立线程 + `recv_timeout` 轮询 + 仅在变化时 `emit`。
//! 文件 mtime 未变化时不重新解析，空闲态开销只有一次 `metadata`。

mod balance;
mod process;
mod session;

use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use tauri::{AppHandle, Emitter, Manager};

use crate::dsh::balance::BalanceCache;
use crate::dsh::session::{SessionProbe, SESSION_SCAN_INTERVAL_MS};
use crate::log;
use crate::state::SharedState;

pub const EVENT_DSH_CHANGED: &str = "dsh://changed";

/// 有 dsh 时的轮询间隔。
const POLL_ACTIVE_MS: u64 = 300;
/// 没有 dsh 时的轮询间隔（只做一次 metadata 检查）。
const POLL_IDLE_MS: u64 = 1200;
/// 状态文件超过这么久没更新即视为 dsh 已退出；「等待确认」阶段不超时。
const STALE_MS: f64 = 120_000.0;
/// 文件半写入（原子写被打断）时的最大连续失败次数，超过才清空状态。
const MAX_PARSE_FAILURES: i32 = 3;

/// dsh 执行阶段（与前端 `DshStage` 一致）。
pub const STAGE_IDLE: &str = "idle";
pub const STAGE_THINKING: &str = "thinking";
pub const STAGE_TOOL: &str = "tool";
pub const STAGE_WAITING: &str = "waiting";
pub const STAGE_DONE: &str = "done";

/// 某一时刻的 dsh 状态快照（不可变）。
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct DshSnapshot {
    pub stage: String,
    /// 展示用短词；状态文件没给时按 stage 兜底。
    pub stage_label: String,
    pub model: String,
    /// `deepseek-official` / `openrouter` / ...
    pub provider: String,
    /// 最近一次工具调用的名字（由自动探测给出；展开态展示）。
    pub tool: String,
    /// `official` / `web` / `api`；仅 official / web 才显示余额。
    pub auth_mode: String,
    pub session_id: String,
    pub input_tokens: f64,
    pub output_tokens: f64,
    pub cache_read_tokens: f64,
    /// 会话累计 token（input + output + cacheRead）。
    pub total_tokens: f64,
    pub session_cost: f64,
    pub currency: String,
    pub balance: Option<f64>,
    pub balance_currency: String,
    pub updated_at_ms: f64,
}

impl DshSnapshot {
    /// 是否处于「等待确认」：这一类阶段不参与超时判定。
    fn holds_open(&self) -> bool {
        self.stage == STAGE_WAITING
    }
}

/// 状态文件的原始结构：全部字段可选，缺省即忽略。
#[derive(Deserialize, Default)]
#[serde(rename_all = "PascalCase", default)]
struct StatusFile {
    /// 写入方显式声明 dsh 是否仍在运行；缺省按「运行中」处理。
    running: Option<bool>,
    stage: String,
    stage_label: String,
    model: String,
    provider: String,
    tool: String,
    auth_mode: String,
    session_id: String,
    input_tokens: f64,
    output_tokens: f64,
    cache_read_tokens: f64,
    total_tokens: f64,
    session_cost: f64,
    currency: String,
    balance: Option<f64>,
    balance_currency: String,
    updated_at_ms: f64,
}

struct Inner {
    current: Arc<Mutex<Option<DshSnapshot>>>,
    stop: Mutex<Option<Sender<()>>>,
}

/// dsh 服务句柄（克隆成本极低）。
#[derive(Clone)]
pub struct DshService {
    inner: Arc<Inner>,
}

impl DshService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                current: Arc::new(Mutex::new(None)),
                stop: Mutex::new(None),
            }),
        }
    }

    /// 当前活跃的 dsh 快照；未检测到 dsh 时为 `None`。
    pub fn current(&self) -> Option<DshSnapshot> {
        self.inner
            .current
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    pub fn is_active(&self) -> bool {
        self.inner
            .current
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false)
    }

    /// 启动轮询线程（应用装配时调用一次）。
    pub fn start(&self, app: AppHandle) {
        let (tx, rx) = channel::<()>();
        if let Ok(mut guard) = self.inner.stop.lock() {
            *guard = Some(tx);
        }

        let current = self.inner.current.clone();
        std::thread::Builder::new()
            .name("winisland-dsh".into())
            .spawn(move || run(app, current, rx))
            .ok();
    }

    /// 通知线程退出（应用关闭时调用）。
    pub fn stop(&self) {
        if let Ok(mut guard) = self.inner.stop.lock() {
            if let Some(tx) = guard.take() {
                let _ = tx.send(());
            }
        }
    }
}

impl Default for DshService {
    fn default() -> Self {
        Self::new()
    }
}

fn run(
    app: AppHandle,
    current: Arc<Mutex<Option<DshSnapshot>>>,
    rx: Receiver<()>,
) {
    let mut watcher = Watcher::new();
    let mut auto = AutoProbe::new();
    let mut last_emitted: Option<Option<DshSnapshot>> = None;

    loop {
        let active = current
            .lock()
            .map(|guard| guard.is_some())
            .unwrap_or(false);

        let interval = Duration::from_millis(if active {
            POLL_ACTIVE_MS
        } else {
            POLL_IDLE_MS
        });

        match rx.recv_timeout(interval) {
            Ok(()) => break,
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        let now = now_ms();
        // 首选显式状态文件（可由 dsh 插件 / 脚本写入，能附带余额）；
        // 没有可用状态文件时，直接从 dsh 自己的会话数据里探测。
        let next = watcher.probe(now).or_else(|| auto.probe(now));

        if let Ok(mut guard) = current.lock() {
            *guard = next.clone();
        }

        if last_emitted.as_ref() == Some(&next) {
            continue;
        }

        let previous = last_emitted.clone().flatten();
        last_emitted = Some(next.clone());

        report_transition(previous.as_ref(), next.as_ref());

        let _ = app.emit(EVENT_DSH_CHANGED, next.clone());

        // 活跃状态翻转会改变「岛体是否应该显示」，交给 state 统一处理
        let was_active = previous.is_some();
        let now_active = next.is_some();
        if was_active != now_active {
            let state = app.state::<SharedState>();
            state.sync_visibility(&app);
        }
    }

    log::info("dsh: 轮询线程已退出");
}

/// 只在阶段 / 接入状态变化时写日志，避免每轮刷屏；不记录任何数值。
fn report_transition(previous: Option<&DshSnapshot>, next: Option<&DshSnapshot>) {
    match (previous, next) {
        (None, Some(snapshot)) => log::info(&format!("dsh: 已接入 stage={}", snapshot.stage)),
        (Some(_), None) => log::info("dsh: 已断开"),
        (Some(old), Some(new)) => {
            if old.stage != new.stage {
                log::info(&format!("dsh: 阶段 {} -> {}", old.stage, new.stage));
            }
        }
        (None, None) => {}
    }
}

// ---------------- 自动探测（无需 dsh 侧配合） ----------------

/// 进程探测的节流间隔。
const PROCESS_CHECK_MS: f64 = 2000.0;

/// 没有状态文件时，直接从 dsh 进程 + 会话事件流推导状态。
///
/// 判定顺序：
/// 1. 有 `DeepSeek Harness.exe` / `dsh` 进程在跑 → dsh 活跃；
/// 2. 取最近写入的会话，从 zstd 帧尾部读出最近事件 → 阶段；
/// 3. 用量由「会话缓存的历史总量 + 尾部新增事件的 usage」累计。
struct AutoProbe {
    running: bool,
    checked_at: f64,
    probe: Option<SessionProbe>,
    next_scan_at: f64,
    balance: BalanceCache,
}

impl AutoProbe {
    fn new() -> Self {
        Self {
            running: false,
            checked_at: 0.0,
            probe: None,
            next_scan_at: 0.0,
            balance: BalanceCache::new(),
        }
    }

    fn probe(&mut self, now: f64) -> Option<DshSnapshot> {
        if now - self.checked_at >= PROCESS_CHECK_MS {
            self.checked_at = now;
            self.running = process::dsh_running();
            if !self.running {
                self.probe = None;
            }
        }

        if !self.running {
            return None;
        }

        // 余额 5 分钟刷新一次，失败静默
        self.refresh_session(now);
        let mut snapshot = match self.probe.as_mut() {
            Some(probe) => {
                probe.pump();
                snapshot_from_session(probe, now)
            }
            // dsh 在跑但还没打开会话：显示空闲
            None => idle_snapshot(),
        };

        self.balance.refresh(now);
        if snapshot.auth_mode.eq_ignore_ascii_case("official") {
            if let Some((amount, currency)) = self.balance.value() {
                snapshot.balance = Some(amount);
                snapshot.balance_currency = currency;
            }
        }

        Some(snapshot)
    }

    /// 只在「当前会话」变化时重新接入（扫描本身有节流）。
    fn refresh_session(&mut self, now: f64) {
        if now < self.next_scan_at {
            return;
        }
        self.next_scan_at = now + SESSION_SCAN_INTERVAL_MS;

        let newest = session::current_session().map(|(path, _)| path);
        let current = self.probe.as_ref().map(|probe| probe.path().to_path_buf());

        // 接入时文件还不存在 / 还是空的新对话：等它真正有内容后重新接入一次
        let waiting_for_content = self
            .probe
            .as_ref()
            .map(|probe| {
                probe.seen_len() == 0
                    && std::fs::metadata(probe.path())
                        .map(|meta| meta.len() > 0)
                        .unwrap_or(false)
            })
            .unwrap_or(false);

        if newest != current || waiting_for_content {
            self.probe = SessionProbe::attach();
        }
    }
}

fn snapshot_from_session(probe: &SessionProbe, now: f64) -> DshSnapshot {
    let stage = probe.stage(now);
    let provider = probe.provider.clone();

    // 只有「这个会话刚刚还在活动」时才显示用量：
    // 否则沿用下来的历史数字会让人以为空对话也在烧 token。
    let fresh = probe.is_fresh(now);
    let (input, output, cache_read, total) = if fresh {
        (
            probe.totals.input,
            probe.totals.output,
            probe.totals.cache_read,
            probe.totals.total(),
        )
    } else {
        (0.0, 0.0, 0.0, 0.0)
    };

    DshSnapshot {
        stage: stage.as_str().to_string(),
        stage_label: default_label(stage.as_str()).to_string(),
        model: probe.model.clone(),
        // `deepseek-official` / `deepseek-account` 都算官方登录
        auth_mode: if provider.to_ascii_lowercase().contains("deepseek") {
            "official".to_string()
        } else if provider.is_empty() {
            "official".to_string()
        } else {
            "api".to_string()
        },
        provider,
        tool: probe.tool.clone(),
        session_id: format!("session-{}", probe.id),
        input_tokens: input,
        output_tokens: output,
        cache_read_tokens: cache_read,
        total_tokens: total,
        // 成本依赖 cost-meter 插件账本，自动探测不猜测价格；余额需要密钥，同理不自行获取
        session_cost: 0.0,
        currency: "CNY".to_string(),
        balance: None,
        balance_currency: "CNY".to_string(),
        updated_at_ms: if probe.stage_ms() > 0.0 {
            probe.stage_ms()
        } else {
            now
        },
    }
}

fn idle_snapshot() -> DshSnapshot {
    DshSnapshot {
        stage: STAGE_IDLE.to_string(),
        stage_label: default_label(STAGE_IDLE).to_string(),
        provider: "deepseek-official".to_string(),
        auth_mode: "official".to_string(),
        currency: "CNY".to_string(),
        balance_currency: "CNY".to_string(),
        updated_at_ms: 0.0,
        ..DshSnapshot::default()
    }
}

// ---------------- 状态文件读取 ----------------

/// 缓存文件 mtime，未变化时跳过解析。
struct Watcher {
    mtime: Option<SystemTime>,
    failures: i32,
    last_good: Option<DshSnapshot>,
}

impl Watcher {
    fn new() -> Self {
        Self {
            mtime: None,
            failures: 0,
            last_good: None,
        }
    }

    /// 返回当前应生效的快照（`None` 表示 dsh 不活跃）。
    fn probe(&mut self, now: f64) -> Option<DshSnapshot> {
        let path = status_path();

        let mtime = std::fs::metadata(&path).and_then(|meta| meta.modified()).ok();

        let Some(mtime) = mtime else {
            // 文件消失：dsh 已退出
            if self.mtime.is_some() {
                self.mtime = None;
                self.last_good = None;
                self.failures = 0;
            }
            return None;
        };

        if self.mtime == Some(mtime) {
            // 内容没变：沿用上次解析结果，但仍要判断是否超时
            return self.apply_staleness(self.last_good.clone(), now);
        }

        self.mtime = Some(mtime);

        let parsed = std::fs::read_to_string(&path)
            .map_err(|err| err.to_string())
            .and_then(|text| serde_json::from_str::<StatusFile>(&text).map_err(|err| err.to_string()));

        match parsed {
            Ok(file) => {
                self.failures = 0;
                let snapshot = normalize(file, now);
                self.last_good = Some(snapshot.clone());
                self.apply_staleness(Some(snapshot), now)
            }
            Err(err) => {
                self.failures += 1;
                if self.failures <= MAX_PARSE_FAILURES {
                    // 半写入：保留上一份快照，避免胶囊忽隐忽现
                    if self.failures == 1 {
                        log::warn(&format!("dsh: 状态文件解析失败，沿用上一份快照（{err}）"));
                    }
                    self.apply_staleness(self.last_good.clone(), now)
                } else {
                    self.last_good = None;
                    None
                }
            }
        }
    }

    fn apply_staleness(&self, snapshot: Option<DshSnapshot>, now: f64) -> Option<DshSnapshot> {
        let snapshot = snapshot?;
        if snapshot.holds_open() {
            return Some(snapshot);
        }
        if now - snapshot.updated_at_ms > STALE_MS {
            return None;
        }
        Some(snapshot)
    }
}

fn normalize(file: StatusFile, now: f64) -> DshSnapshot {
    let stage = normalize_stage(&file.stage);

    let total = if file.total_tokens > 0.0 {
        file.total_tokens
    } else {
        file.input_tokens + file.output_tokens + file.cache_read_tokens
    };

    let label = if file.stage_label.trim().is_empty() {
        default_label(&stage).to_string()
    } else {
        file.stage_label.trim().to_string()
    };

    DshSnapshot {
        stage,
        stage_label: label,
        model: file.model.trim().to_string(),
        provider: file.provider.trim().to_string(),
        tool: file.tool.trim().to_string(),
        auth_mode: file.auth_mode.trim().to_ascii_lowercase(),
        session_id: file.session_id.trim().to_string(),
        input_tokens: sanitize(file.input_tokens),
        output_tokens: sanitize(file.output_tokens),
        cache_read_tokens: sanitize(file.cache_read_tokens),
        total_tokens: sanitize(total),
        session_cost: sanitize(file.session_cost),
        currency: if file.currency.trim().is_empty() {
            "CNY".to_string()
        } else {
            file.currency.trim().to_ascii_uppercase()
        },
        balance: file.balance.filter(|value| value.is_finite()),
        balance_currency: if file.balance_currency.trim().is_empty() {
            "CNY".to_string()
        } else {
            file.balance_currency.trim().to_ascii_uppercase()
        },
        // 写入方没给时间戳时用当前时刻，避免立刻被判定为过期
        updated_at_ms: if file.updated_at_ms > 0.0 {
            file.updated_at_ms
        } else {
            now
        },
    }
    .with_running(file.running)
}

impl DshSnapshot {
    /// `Running: false` 是写入方显式声明「已退出」，用 `updated_at_ms = 0`
    /// 让超时判定直接把它判成不活跃。
    fn with_running(mut self, running: Option<bool>) -> Self {
        if running == Some(false) {
            self.updated_at_ms = 0.0;
        }
        self
    }
}

fn normalize_stage(raw: &str) -> String {
    match raw.trim().to_ascii_lowercase().as_str() {
        "thinking" | "reasoning" => STAGE_THINKING.to_string(),
        "tool" | "tool_call" | "toolcall" | "acting" => STAGE_TOOL.to_string(),
        "waiting" | "confirm" | "waiting_confirm" | "confirming" => STAGE_WAITING.to_string(),
        "done" | "completed" | "complete" | "finished" => STAGE_DONE.to_string(),
        "idle" | "" => STAGE_IDLE.to_string(),
        _ => STAGE_IDLE.to_string(),
    }
}

fn default_label(stage: &str) -> &'static str {
    match stage {
        STAGE_THINKING => "思考中",
        STAGE_TOOL => "调用工具",
        STAGE_WAITING => "等待确认",
        STAGE_DONE => "已完成",
        _ => "空闲",
    }
}

fn sanitize(value: f64) -> f64 {
    if value.is_finite() && value > 0.0 {
        value
    } else {
        0.0
    }
}

/// 状态文件路径：首选应用数据目录，其次 `~/.dsh`。
fn status_path() -> PathBuf {
    let primary = log::root_dir().join("dsh-status.json");
    if primary.exists() {
        return primary;
    }

    if let Ok(home) = std::env::var("USERPROFILE") {
        if !home.is_empty() {
            let secondary = PathBuf::from(home).join(".dsh").join("winisland-status.json");
            if secondary.exists() {
                return secondary;
            }
        }
    }

    // 都不存在时返回首选路径，交给调用方按「文件消失」处理
    primary
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

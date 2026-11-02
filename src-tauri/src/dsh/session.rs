//! dsh 会话事件流：只读解析，用于推导「当前阶段」与会话用量。
//!
//! dsh 把每个会话写成 `<DSH_HOME>/sessions/<项目>/session-<id>/session.v4.jsonl.zstd`：
//! **每追加一条事件就追加一个独立 zstd 帧**。因此只要找到帧边界，就能只解尾部帧，
//! 拿到最近的事件而不用解整个文件（会话可能几十 MB）。
//!
//! 这里只关心三类信息，绝不读取消息正文：
//! 1. 事件类型（`tool/call` / `tool/result` / `turn/end` / `approval/asked` ...）→ 阶段
//! 2. `data.usage` → 会话 token 用量
//! 3. `data.message.source` 的 provider / model → 展示用模型名

use std::fs::File;
use std::io::{Read, Seek, SeekFrom};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use ruzstd::decoding::StreamingDecoder;
use serde_json::Value;

/// zstd 帧魔数。
const ZSTD_MAGIC: [u8; 4] = [0x28, 0xB5, 0x2F, 0xFD];
/// 首次接入时回看的字节数（只为确定当前阶段，不追求精确历史用量）。
const ATTACH_WINDOW: u64 = 2 << 20;
/// 单次 pump 最多读取的新增字节，避免异常大文件拖垮轮询线程。
const PUMP_LIMIT: u64 = 8 << 20;
/// 单帧解压输出上限。
const MAX_FRAME_OUTPUT: usize = 32 << 20;
/// 「已完成」保持多久之后回落成「空闲」。
pub const DONE_HOLD_MS: f64 = 120_000.0;
/// 「思考中 / 调用工具」多久没有新事件就回落成「空闲」。
/// 生成一段长回答时 dsh 可能一两分钟不落事件，所以给足 5 分钟。
const THINKING_HOLD_MS: f64 = 300_000.0;
/// 探测「当前会话」的最小间隔（扫描目录有成本）。
const SCAN_INTERVAL_MS: f64 = 2_000.0;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Stage {
    Idle,
    Thinking,
    Tool,
    Waiting,
    Done,
}

impl Stage {
    pub fn as_str(self) -> &'static str {
        match self {
            Stage::Idle => "idle",
            Stage::Thinking => "thinking",
            Stage::Tool => "tool",
            Stage::Waiting => "waiting",
            Stage::Done => "done",
        }
    }
}

#[derive(Clone, Copy, Default)]
pub struct Totals {
    pub input: f64,
    pub output: f64,
    pub cache_read: f64,
}

impl Totals {
    pub fn total(self) -> f64 {
        self.input + self.output + self.cache_read
    }
}

/// 当前会话的实时观测结果。
pub struct SessionProbe {
    pub id: String,
    pub model: String,
    pub provider: String,
    /// 最近一次工具调用的名字（展开态展示用）。
    pub tool: String,
    pub totals: Totals,
    stage: Stage,
    stage_ms: f64,
    path: PathBuf,
    /// 已解析到的文件字节偏移（恒为帧边界）。
    offset: u64,
    /// 会话缓存里已计入的最后一个事件 seq，只统计更大的 seq，避免重复累计。
    base_seq: i64,
    seen_len: u64,
    /// 接入时刻（取事件流 / 缓存的最近写入时间）：刚切换过来的会话也算「新鲜」。
    attached_ms: f64,
}

impl SessionProbe {
    /// 接入最新会话；没有会话时返回 None。
    pub fn attach() -> Option<Self> {
        let (path, id) = current_session()?;
        let mut probe = Self {
            id: id.clone(),
            model: String::new(),
            provider: String::new(),
            tool: String::new(),
            totals: Totals::default(),
            stage: Stage::Idle,
            stage_ms: 0.0,
            path: path.clone(),
            offset: 0,
            base_seq: 0,
            seen_len: 0,
            attached_ms: 0.0,
        };

        let file_ms = mtime_ms(&path).unwrap_or(0.0);
        let cache_ms = cache_path(&id)
            .and_then(|path| mtime_ms(&path))
            .unwrap_or(0.0);
        probe.attached_ms = file_ms.max(cache_ms);

        probe.load_cache(&id);
        probe.attach_scan();
        Some(probe)
    }

    /// 轮询新增事件；返回是否有变化。
    pub fn pump(&mut self) -> bool {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return false;
        };

        let len = meta.len();
        if len < self.offset {
            // 会话被重写 / 截断：从头再来
            self.offset = 0;
        }
        if len == self.seen_len && self.seen_len > 0 {
            return false;
        }
        self.seen_len = len;

        if len <= self.offset {
            return false;
        }

        if let Some(bytes) = read_range(&self.path, self.offset, len.min(self.offset + PUMP_LIMIT)) {
            let consumed = self.apply_frames(&bytes, self.base_seq);
            self.offset += consumed as u64;
        } else {
            self.offset = len;
        }

        true
    }

    /// 展示用阶段：长时间没有新事件就回落成空闲，避免会话早就结束后还一直显示思考中。
    ///
    /// - 「等待确认」不超时：人在思考，岛体不该先消失；
    /// - 「已完成」保留 120 秒（让用户看到收尾），之后回到空闲；
    /// - 「思考中 / 调用工具」超过 5 分钟没有新事件视为已经结束。
    pub fn stage(&self, now: f64) -> Stage {
        match self.stage {
            Stage::Waiting => Stage::Waiting,
            Stage::Idle => Stage::Idle,
            Stage::Done => {
                if now - self.stage_ms > DONE_HOLD_MS {
                    Stage::Idle
                } else {
                    Stage::Done
                }
            }
            other => {
                if self.stage_ms <= 0.0 || now - self.stage_ms > THINKING_HOLD_MS {
                    Stage::Idle
                } else {
                    other
                }
            }
        }
    }

    pub fn stage_ms(&self) -> f64 {
        self.stage_ms
    }

    /// 已解析到的文件长度（0 表示接入时文件还不存在 / 还是空的）。
    pub fn seen_len(&self) -> u64 {
        self.seen_len
    }

    /// 会话是否「刚刚还在活动」（用于决定要不要显示用量数字）。
    ///
    /// 两个来源任一在 2 分钟内就算新鲜：最近的事件时间，或刚刚被切换 / 打开过。
    pub fn is_fresh(&self, now: f64) -> bool {
        let last = self.stage_ms.max(self.attached_ms);
        last > 0.0 && now - last <= DONE_HOLD_MS
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    // ---------------- 内部 ----------------

    /// 读会话缓存（纯 JSON，小文件）：拿历史和兜底阶段。
    fn load_cache(&mut self, id: &str) {
        let Some(path) = cache_path(id) else {
            return;
        };
        let Ok(text) = std::fs::read_to_string(&path) else {
            return;
        };
        let Ok(value) = serde_json::from_str::<Value>(&text) else {
            return;
        };

        let Some(rows) = value.pointer("/record/rows") else {
            return;
        };

        if let Some(usage) = rows.pointer("/tokenUsage/val/totals") {
            self.totals = Totals {
                input: num(usage, "uncachedInputTokens"),
                output: num(usage, "outputTokens"),
                cache_read: num(usage, "cacheReadTokens"),
            };
        }
        if let Some(seq) = rows.pointer("/tokenUsage/seq").and_then(Value::as_i64) {
            self.base_seq = seq;
        }

        if let Some(model) = rows
            .pointer("/modelSelection/val/lastUsed/model")
            .and_then(Value::as_str)
        {
            self.model = model.to_string();
        }

        // 缓存只在阶段边界落盘，这里只作首次接入的兜底判断
        if self.stage == Stage::Idle {
            let pending = rows
                .pointer("/sessionStats/val/pendingCalls")
                .and_then(Value::as_object)
                .map(|calls| !calls.is_empty())
                .unwrap_or(false);
            let asking = rows
                .pointer("/userQuestions/val/questions/active")
                .and_then(Value::as_array)
                .map(|items| !items.is_empty())
                .unwrap_or(false);
            let open_turn = rows
                .pointer("/turnBoundary/val/openTurnStartSeq")
                .map(|value| !value.is_null())
                .unwrap_or(false);

            self.stage = if asking {
                Stage::Waiting
            } else if pending {
                Stage::Tool
            } else if open_turn {
                Stage::Thinking
            } else {
                Stage::Idle
            };

            // 缓存里的阶段没有时间戳，用文件 mtime 兜底，
            // 这样过期阶段会被 stage() 的 5 分钟阈值正常回落成空闲。
            if self.stage != Stage::Idle {
                self.stage_ms = mtime_ms(&self.path).unwrap_or(0.0);
            }
        }
    }

    /// 首帧：回看尾部窗口，把最近事件补进来（`seq > base_seq` 的用量才计入）。
    fn attach_scan(&mut self) {
        let Ok(meta) = std::fs::metadata(&self.path) else {
            return;
        };
        let len = meta.len();
        self.seen_len = len;
        if len == 0 {
            return;
        }

        let start = len.saturating_sub(ATTACH_WINDOW);
        let Some(bytes) = read_range(&self.path, start, len) else {
            self.offset = len;
            return;
        };

        let consumed = self.apply_frames(&bytes, self.base_seq);
        self.offset = start + consumed as u64;
    }

    /// 解析一段「从一个帧起点开始」的字节；返回已成功消费的字节数。
    fn apply_frames(&mut self, bytes: &[u8], min_seq: i64) -> usize {
        let mut starts = Vec::new();
        let mut index = 0usize;
        while index + 4 <= bytes.len() {
            if bytes[index..index + 4] == ZSTD_MAGIC {
                starts.push(index);
                index += 4;
            } else {
                index += 1;
            }
        }

        let mut consumed = 0usize;
        for (position, start) in starts.iter().enumerate() {
            let end = starts.get(position + 1).copied().unwrap_or(bytes.len());
            if end <= *start {
                continue;
            }

            let Some(text) = decode_frame(&bytes[*start..end]) else {
                // 最后一帧通常正在写入，留到下次再解
                break;
            };

            for line in text.lines() {
                let line = line.trim();
                if !line.is_empty() {
                    self.apply_line(line, min_seq);
                }
            }

            consumed = end;
        }

        consumed
    }

    fn apply_line(&mut self, line: &str, min_seq: i64) {
        let Ok(event) = serde_json::from_str::<Value>(line) else {
            return;
        };

        let Some(kind) = event.get("type").and_then(Value::as_str) else {
            return;
        };

        let seq = event.get("seq").and_then(Value::as_i64).unwrap_or(0);
        let time = event
            .get("time")
            .and_then(Value::as_f64)
            .unwrap_or_else(now_ms);

        let data = event.get("data");

        match kind {
            "assistant/message" => {
                if seq > min_seq {
                    if let Some(usage) = data.and_then(|d| d.get("usage")) {
                        self.totals.input += num(usage, "inputTokens");
                        self.totals.output += num(usage, "outputTokens");
                        self.totals.cache_read += num(usage, "cacheReadTokens");
                    }
                }
                if let Some(source) = data
                    .and_then(|d| d.get("message"))
                    .and_then(|m| m.get("source"))
                {
                    if let Some(model) = source.get("model").and_then(Value::as_str) {
                        self.model = model.to_string();
                    }
                    if let Some(provider) = source.get("provider").and_then(Value::as_str) {
                        self.provider = provider.to_string();
                    }
                }
                self.set_stage(Stage::Thinking, time);
            }
            "tool/call" => {
                if let Some(name) = data.and_then(|d| d.get("name")).and_then(Value::as_str) {
                    self.tool = name.to_string();
                }
                self.set_stage(Stage::Tool, time);
            }
            "tool/result" => self.set_stage(Stage::Thinking, time),
            "approval/asked" => self.set_stage(Stage::Waiting, time),
            "approval/decided" => self.set_stage(Stage::Thinking, time),
            "turn/end" => self.set_stage(Stage::Done, time),
            "turn/start" | "user/message" | "step/start" | "step/end" | "request/header"
            | "request/context" => self.set_stage(Stage::Thinking, time),
            _ => {}
        }
    }

    fn set_stage(&mut self, stage: Stage, time: f64) {
        self.stage = stage;
        self.stage_ms = time;
    }
}

// ---------------- 文件与路径 ----------------

fn num(value: &Value, key: &str) -> f64 {
    value.get(key).and_then(Value::as_f64).unwrap_or(0.0)
}

/// `$DSH_HOME` 优先，其次 `~/.dsh`。
pub fn dsh_home() -> Option<PathBuf> {
    if let Ok(home) = std::env::var("DSH_HOME") {
        if !home.trim().is_empty() {
            return Some(PathBuf::from(home));
        }
    }
    let profile = std::env::var("USERPROFILE").ok()?;
    if profile.trim().is_empty() {
        return None;
    }
    Some(PathBuf::from(profile).join(".dsh"))
}

fn cache_path(id: &str) -> Option<PathBuf> {
    let path = dsh_home()?
        .join("storages")
        .join("session_projcache")
        .join("sessions")
        .join(format!("{id}.json"));
    path.exists().then_some(path)
}

/// 取「当前会话」：综合事件流与会话缓存的最近写入时间。
///
/// 只有事件流 mtime 是不够的——桌面端切换对话并不会重写 `session.v4.jsonl.zstd`，
/// 但会刷新 `session_projcache/sessions/<id>.json`，所以两者取较大值才算「刚活动过」。
pub fn current_session() -> Option<(PathBuf, String)> {
    let home = dsh_home()?;
    let sessions = home.join("sessions");
    if !sessions.exists() {
        return None;
    }
    let cache_dir = home
        .join("storages")
        .join("session_projcache")
        .join("sessions");

    let mut best: Option<(SystemTime, PathBuf)> = None;
    walk_sessions(&sessions, &cache_dir, &mut best, 0);

    let (_, path) = best?;
    let id = path
        .parent()
        .and_then(|dir| dir.file_name())
        .and_then(|name| name.to_str())
        .map(|name| name.trim_start_matches("session-").to_string())?;

    Some((path, id))
}

fn walk_sessions(
    dir: &Path,
    cache_dir: &Path,
    best: &mut Option<(SystemTime, PathBuf)>,
    depth: usize,
) {
    if depth > 3 {
        return;
    }
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };

    for entry in entries.flatten() {
        let path = entry.path();
        let Ok(meta) = entry.metadata() else {
            continue;
        };

        if !meta.is_dir() {
            continue;
        }

        let name = path.file_name().and_then(|value| value.to_str()).unwrap_or("");
        if let Some(id) = name.strip_prefix("session-") {
            let jsonl = path.join("session.v4.jsonl.zstd");
            let mut activity = std::fs::metadata(&jsonl)
                .ok()
                .and_then(|meta| meta.modified().ok());

            if let Ok(cache) = std::fs::metadata(cache_dir.join(format!("{id}.json"))) {
                if let Ok(time) = cache.modified() {
                    activity = Some(match activity {
                        Some(current) if current > time => current,
                        _ => time,
                    });
                }
            }

            if let Some(activity) = activity {
                let replace = match best.as_ref() {
                    Some((current, _)) => activity > *current,
                    None => true,
                };
                if replace {
                    *best = Some((activity, jsonl));
                }
            }
        }

        walk_sessions(&path, cache_dir, best, depth + 1);
    }
}

fn mtime_ms(path: &Path) -> Option<f64> {
    let modified = std::fs::metadata(path).ok()?.modified().ok()?;
    let millis = modified.duration_since(UNIX_EPOCH).ok()?.as_millis() as f64;
    Some(millis)
}

fn read_range(path: &Path, from: u64, to: u64) -> Option<Vec<u8>> {
    if to <= from {
        return None;
    }
    let mut file = File::open(path).ok()?;
    file.seek(SeekFrom::Start(from)).ok()?;
    let mut buffer = vec![0u8; (to - from) as usize];
    let mut filled = 0usize;
    while filled < buffer.len() {
        match file.read(&mut buffer[filled..]) {
            Ok(0) => break,
            Ok(read) => filled += read,
            Err(_) => return None,
        }
    }
    buffer.truncate(filled);
    Some(buffer)
}

fn decode_frame(bytes: &[u8]) -> Option<String> {
    let mut decoder = StreamingDecoder::new(bytes).ok()?;
    let mut output = Vec::new();
    let mut chunk = [0u8; 16 * 1024];

    loop {
        match decoder.read(&mut chunk) {
            Ok(0) => break,
            Ok(read) => {
                if output.len() + read > MAX_FRAME_OUTPUT {
                    return None;
                }
                output.extend_from_slice(&chunk[..read]);
            }
            Err(_) => return None,
        }
    }

    String::from_utf8(output).ok()
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

/// 探测「最新会话」的节流间隔（供调用方做缓存判断）。
pub const SESSION_SCAN_INTERVAL_MS: f64 = SCAN_INTERVAL_MS;

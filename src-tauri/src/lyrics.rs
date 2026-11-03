//! 歌词检索：曲目变化后按「歌名 + 歌手 + 时长」查找带时间轴的 LRC。
//!
//! 独占一个线程跑阻塞式 HTTP（不占用媒体线程的轮询）；结果做内存缓存，
//! 无论是否找到歌词都会发出 `lyrics://changed`，由前端决定回退展示。
//!
//! 来源顺序：lrclib.net 开放接口 → 网易云音乐 web 接口 → 本地歌词文件兜底。
//!
//! 检索策略（「歌词经常找不到」的根因基本都在这里）：
//! - 播放器给出的标题常带修饰：`(Live)`、`- Remastered 2011`、`feat. xxx`、`【高音质】`，
//!   原样拿去搜几乎必然搜不到 → 先归一成「干净曲名 + 主歌手」，再按多组候选依次查；
//! - 同一首歌常有专辑版 / 现场版 / 翻唱多个条目，先用时长严格择优选；
//!   都不合适时再放宽时长（曲名、歌手仍然要对得上）捞一个近似版本，而不是直接判失败；
//! - 「网络失败」与「确实没有歌词」分开对待：前者马上放弃网络改走本地文件，
//!   后者落地短期负缓存，并在稍后自动重试几次；
//! - 排队只保留最新一条请求：切歌很快时，旧请求不会占住线程让新歌迟迟等不到歌词。

use std::collections::{HashMap, HashSet, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender, TryRecvError};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::log;

pub const EVENT_LYRICS_CHANGED: &str = "lyrics://changed";

/// 歌词缓存上限（LRU）。
const CACHE_LIMIT: usize = 96;
/// 命中结果的缓存时效：一小时内切回同一首歌不再重新联网。
const HIT_TTL_MS: f64 = 60.0 * 60_000.0;
/// 未命中结果的缓存时效：短一些，网络恢复后还有机会重查。
const MISS_TTL_MS: f64 = 3.0 * 60_000.0;
/// 未找到时的自动重试间隔（按次数递增）与次数上限。
const RETRY_DELAY_MS: f64 = 12_000.0;
const MAX_RETRIES: u32 = 2;
/// 单次 HTTP 请求超时。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(4);
/// 一次完整检索的时间预算，防止在断网 / 慢接口上耗掉太久。
const FETCH_BUDGET_MS: f64 = 12_000.0;
/// 单次检索最多发出的 HTTP 请求数。
const MAX_HTTP_CALLS: usize = 8;
/// 严格匹配：候选条目与当前曲目的时长最大容差（毫秒），超出就认为不是同一版本。
const STRICT_TOLERANCE_MS: f64 = 8_000.0;
/// 宽松匹配：曲名 / 歌手确认一致时，允许更大的时长差（用于时长标注不准的接口）。
const LOOSE_TOLERANCE_MS: f64 = 30_000.0;
/// 曲名相似度低于这个值就不认（避免拿完全不相干的条目凑数）。
const MIN_TITLE_SCORE: i32 = 55;
/// 宽松匹配要求的曲名相似度。
const MIN_LOOSE_TITLE_SCORE: i32 = 85;
/// 本地歌词文件的后缀（大小写不敏感）。
const LOCAL_EXTENSIONS: [&str; 2] = ["lrc", "txt"];

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) WinIslandPlus/1.1";

/// 曲名 / 歌手里的「修饰词」：带这些字样的括号段、破折号尾巴一律剥掉再搜。
const DECORATIONS: [&str; 56] = [
    "live",
    "remaster",
    "remastered",
    "version",
    "mix",
    "remix",
    "edit",
    "demo",
    "cover",
    "instrumental",
    "karaoke",
    "acoustic",
    "session",
    "deluxe",
    "mono",
    "stereo",
    "hq",
    "hires",
    "hi-res",
    "mv",
    "official",
    "audio",
    "lyric",
    "visualizer",
    "radio",
    "extended",
    "sped up",
    "slowed",
    "nightcore",
    "feat",
    "ft.",
    "featuring",
    "prod",
    "现场",
    "演唱会",
    "伴奏",
    "纯音乐",
    "翻唱",
    "女声",
    "男声",
    "高音质",
    "无损",
    "完整版",
    "片段",
    "铃声",
    "环绕",
    "重低音",
    "加速",
    "降速",
    "版本",
    "版",
    "单曲",
    "原唱",
    "超清",
    "高清",
    "抖音",
];

/// 歌词来源（只用于排查，不含任何曲目信息）。
#[derive(Serialize, Clone, Copy, PartialEq, Eq)]
#[serde(rename_all = "lowercase")]
pub enum LyricsOrigin {
    Lrclib,
    Netease,
    Local,
}

impl LyricsOrigin {
    fn label(self) -> &'static str {
        match self {
            Self::Lrclib => "lrclib",
            Self::Netease => "netease",
            Self::Local => "local",
        }
    }
}

/// 歌词检索结果推送（与前端 `LyricsSnapshot` 逐字段对应）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct LyricsPayload {
    pub title: String,
    pub artist: String,
    pub album_title: String,
    /// 原始 LRC 文本（带时间轴）；无歌词时为 None。
    pub lrc: Option<String>,
    /// 本地歌词文件的原始字节（小写十六进制）。
    ///
    /// 本地文件常见 UTF-8 / UTF-8 BOM / UTF-16 / GBK 混用，Rust 侧不做猜测，
    /// 原样交给前端用 `TextDecoder`（WebView2 自带 gb18030）解码。
    pub lrc_hex: Option<String>,
    pub origin: Option<LyricsOrigin>,
}

/// 歌词正文：网络结果已是 UTF-8 文本，本地文件保留原始字节。
#[derive(Clone)]
enum LyricsBody {
    Text(String),
    Bytes(Vec<u8>),
}

#[derive(Clone)]
struct LyricsHit {
    body: LyricsBody,
    origin: LyricsOrigin,
}

/// 一次检索请求（重试时沿用同一份参数，只递增 `attempt`）。
#[derive(Clone)]
struct LyricsRequest {
    key: String,
    title: String,
    artist: String,
    album_title: String,
    duration_ms: f64,
    attempt: u32,
}

enum LyricsCommand {
    Request(LyricsRequest),
}

struct Inner {
    commands: Mutex<Option<Sender<LyricsCommand>>>,
    /// 最新请求的曲目键：网络往返期间用户可能已经切歌，回来时据此丢弃过期结果。
    latest: Mutex<Option<String>>,
}

/// 歌词服务句柄（clone 廉价）。
#[derive(Clone)]
pub struct LyricsService {
    inner: Arc<Inner>,
}

impl LyricsService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                commands: Mutex::new(None),
                latest: Mutex::new(None),
            }),
        }
    }

    /// 请求某首歌的歌词：工作线程去重 + 缓存，结果经事件送达。
    ///
    /// `duration_ms` 用于在多个同名条目里挑时长最接近的版本（0 表示未知）。
    pub fn request(&self, title: &str, artist: &str, album_title: &str, duration_ms: f64) {
        let key = format!("{title}\u{1f}{artist}\u{1f}{album_title}");
        let command = LyricsCommand::Request(LyricsRequest {
            key: key.clone(),
            title: title.to_string(),
            artist: artist.to_string(),
            album_title: album_title.to_string(),
            duration_ms,
            attempt: 0,
        });

        if let Ok(mut guard) = self.inner.latest.lock() {
            *guard = Some(key);
        }
        if let Ok(guard) = self.inner.commands.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(command);
            }
        }
    }

    /// 启动歌词线程（应用装配时调用一次）。
    pub fn start(&self, app: AppHandle) {
        let (tx, rx) = channel::<LyricsCommand>();
        if let Ok(mut guard) = self.inner.commands.lock() {
            *guard = Some(tx);
        }

        let inner = self.inner.clone();
        std::thread::Builder::new()
            .name("winisland-lyrics".into())
            .spawn(move || run(app, rx, inner))
            .ok();
    }
}

fn run(app: AppHandle, rx: Receiver<LyricsCommand>, inner: Arc<Inner>) {
    let mut cache = LyricsCache::new(CACHE_LIMIT);
    // 待处理：队列里只保留最新一条（旧请求随切歌失去意义）
    let mut pending: Option<LyricsRequest> = None;
    // 未找到时的自动重试（只保留当前曲目的一条）
    let mut retry: Option<(f64, LyricsRequest)> = None;

    log::info(&format!(
        "lyrics: 本地歌词目录 {}",
        lyrics_dir().display()
    ));

    loop {
        // 1) 清空队列，只保留最新请求；换了曲目就丢掉旧曲目的重试计划
        loop {
            match rx.try_recv() {
                Ok(LyricsCommand::Request(request)) => {
                    if retry
                        .as_ref()
                        .map(|(_, waiting)| waiting.key != request.key)
                        .unwrap_or(false)
                    {
                        retry = None;
                    }
                    pending = Some(request);
                }
                Err(TryRecvError::Empty) => break,
                Err(TryRecvError::Disconnected) => break,
            }
        }

        // 2) 没有新请求时，看看有没有到点的重试
        if pending.is_none() {
            if let Some((at, request)) = retry.take() {
                if now_ms() >= at {
                    pending = Some(request);
                } else {
                    retry = Some((at, request));
                }
            }
        }

        // 3) 没活干就等：有重试计划时睡到那个时刻，否则阻塞等新请求
        let Some(request) = pending.take() else {
            let wait_ms = retry
                .as_ref()
                .map(|(at, _)| (*at - now_ms()).max(1.0))
                .unwrap_or(3_600_000.0);

            match rx.recv_timeout(Duration::from_millis(wait_ms as u64)) {
                Ok(LyricsCommand::Request(request)) => pending = Some(request),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    if pending.is_none() {
                        break;
                    }
                }
            }
            continue;
        };

        // 4) 拉取：缓存命中直接用；重试时绕过负缓存，避免刚失败就又被缓存挡回去
        let cached = if request.attempt == 0 {
            cache.get(&request.key)
        } else {
            None
        };

        let hit = match cached {
            Some(hit) => hit,
            None => {
                let fetched = fetch_lyrics(&request.title, &request.artist, request.duration_ms);
                cache.put(request.key.clone(), fetched.clone());
                fetched
            }
        };

        let missing = hit.is_none();
        match hit.as_ref() {
            Some(found) => log::info(&format!("lyrics: 命中 {} 歌词", found.origin.label())),
            None => log::info("lyrics: 未找到可用歌词"),
        }

        // 期间又切了歌：这次结果没人要，别推给前端（它会因为曲目键不匹配而丢弃）
        let superseded = inner
            .latest
            .lock()
            .map(|latest| latest.as_deref() != Some(request.key.as_str()))
            .unwrap_or(false);
        if superseded {
            continue;
        }

        let mut lrc: Option<String> = None;
        let mut lrc_hex: Option<String> = None;
        let mut origin: Option<LyricsOrigin> = None;

        if let Some(found) = hit {
            origin = Some(found.origin);
            match found.body {
                LyricsBody::Text(text) => lrc = Some(text),
                LyricsBody::Bytes(bytes) => lrc_hex = Some(to_hex(&bytes)),
            }
        }

        let _ = app.emit(
            EVENT_LYRICS_CHANGED,
            LyricsPayload {
                title: request.title.clone(),
                artist: request.artist.clone(),
                album_title: request.album_title.clone(),
                lrc,
                lrc_hex,
                origin,
            },
        );

        // 5) 没找到：安排一次延后重试（网络抖动、接口临时失败都很常见）
        if missing && request.attempt < MAX_RETRIES {
            let delay = RETRY_DELAY_MS * (request.attempt as f64 + 1.0);
            let mut next = request;
            next.attempt += 1;
            retry = Some((now_ms() + delay, next));
            log::info(&format!("lyrics: {} 秒后重试检索", (delay / 1_000.0) as i64));
        }
    }

    log::info("lyrics: 歌词线程退出");
}

// ---------------- 检索编排 ----------------

/// 一次检索的预算：时间 + 请求数，外加「网络不可用」的快速放弃开关。
struct Budget {
    deadline_ms: f64,
    offline: bool,
    calls: usize,
}

impl Budget {
    fn new() -> Self {
        Self {
            deadline_ms: now_ms() + FETCH_BUDGET_MS,
            offline: false,
            calls: 0,
        }
    }

    /// 还能再发一次请求吗（同时计数）。
    fn take(&mut self) -> bool {
        if self.offline || self.calls >= MAX_HTTP_CALLS || now_ms() > self.deadline_ms {
            return false;
        }
        self.calls += 1;
        true
    }

    fn available(&self) -> bool {
        !self.offline && self.calls < MAX_HTTP_CALLS && now_ms() <= self.deadline_ms
    }

    fn note(&mut self, error: &HttpError) {
        if matches!(error, HttpError::Offline) {
            // 连接失败 / 超时：再打别的接口也是白等，直接走本地文件
            self.offline = true;
        }
    }
}

fn fetch_lyrics(title: &str, artist: &str, duration_ms: f64) -> Option<LyricsHit> {
    let queries = query_candidates(title, artist);
    let mut budget = Budget::new();

    // 第一轮：严格匹配（时长差别太大的现场版 / 翻唱会被丢掉）
    for query in &queries {
        if !budget.take() {
            break;
        }
        match lrclib_search(query, duration_ms, &STRICT_RULES) {
            Ok(Some(hit)) => {
                log::info("lyrics: lrclib 严格匹配命中");
                return Some(hit);
            }
            Ok(None) => {}
            Err(error) => {
                log::warn(&format!("lyrics: lrclib 检索失败（{}）", error.label()));
                budget.note(&error);
            }
        }
    }

    // 第二轮：放宽时长但不放宽曲名——接口里的时长标注经常不准，直接判失败太苛刻
    if duration_ms > 0.0 && budget.available() {
        for query in queries.iter().take(2) {
            if !budget.take() {
                break;
            }
            match lrclib_search(query, duration_ms, &LOOSE_RULES) {
                Ok(Some(hit)) => {
                    log::info("lyrics: lrclib 近似版本命中（时长有出入）");
                    return Some(hit);
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn(&format!("lyrics: lrclib 检索失败（{}）", error.label()));
                    budget.note(&error);
                }
            }
        }
    }

    // 第三轮：网易云兜底
    if budget.available() {
        for query in queries.iter().take(2) {
            if !budget.take() {
                break;
            }
            match netease_search(query, duration_ms) {
                Ok(Some(hit)) => {
                    log::info("lyrics: netease 命中");
                    return Some(hit);
                }
                Ok(None) => {}
                Err(error) => {
                    log::warn(&format!("lyrics: netease 检索失败（{}）", error.label()));
                    budget.note(&error);
                }
            }
        }
    }

    // 第四轮：本地文件
    fetch_from_local(title, artist)
}

// ---------------- 查询候选 ----------------

/// 一组检索条件：曲名 + 歌手（歌手可以为空）。
struct Query {
    title: String,
    artist: String,
}

/// 交由各来源依次尝试的候选条件，最多四组（顺序即优先级）。
fn query_candidates(title: &str, artist: &str) -> Vec<Query> {
    let mut out: Vec<Query> = Vec::new();

    // 有些播放器把「歌名 - 歌手」（甚至「歌手 - 歌名」）整串塞进标题
    let (title_head, title_tail) = split_title_dash(title);
    let artist_hint = normalize(artist);
    let reversed = !artist_hint.is_empty() && normalize(&title_head).contains(&artist_hint);
    let (title_main, extra_artist) = if reversed {
        (
            title_tail.clone().unwrap_or_else(|| title_head.clone()),
            Some(title_head.clone()),
        )
    } else {
        (title_head.clone(), title_tail.clone())
    };

    let mut artists: Vec<String> = Vec::new();
    for raw in [artist, extra_artist.as_deref().unwrap_or("")] {
        for item in split_artists(raw) {
            let cleaned = clean_title(&item);
            if cleaned.is_empty() {
                continue;
            }
            if artists
                .iter()
                .any(|existing: &String| existing.eq_ignore_ascii_case(&cleaned))
            {
                continue;
            }
            artists.push(cleaned);
        }
    }

    let clean_title = clean_title(&title_main);
    let primary_artist = artists.first().cloned().unwrap_or_default();

    // 1) 净化曲名 + 首位歌手：命中率最高的一组
    push_query(&mut out, &clean_title, &primary_artist);
    // 2) 净化曲名 + 第二位歌手（合唱曲常见）
    if let Some(second) = artists.get(1) {
        push_query(&mut out, &clean_title, second);
    }
    // 3) 原始标题（有些曲名的括号就是正式名字的一部分）
    push_query(&mut out, &title_main, &primary_artist);
    // 4) 不限歌手：标题对得上就先用，时长负责择优
    push_query(&mut out, &clean_title, "");

    out
}

fn push_query(out: &mut Vec<Query>, title: &str, artist: &str) {
    let title = title.trim();
    if title.is_empty() {
        return;
    }
    let artist = artist.trim();
    if out
        .iter()
        .any(|query| query.title.eq_ignore_ascii_case(title) && query.artist.eq_ignore_ascii_case(artist))
    {
        return;
    }
    out.push(Query {
        title: title.to_string(),
        artist: artist.to_string(),
    });
}

/// 把「歌名 - 歌手」拆成两段；没有分隔符时原样返回。
fn split_title_dash(title: &str) -> (String, Option<String>) {
    for separator in [" - ", " – ", " — ", "－"] {
        if let Some(index) = title.find(separator) {
            let head = title[..index].trim();
            let tail = title[index + separator.len()..].trim();
            if !head.is_empty() && !tail.is_empty() {
                return (head.to_string(), Some(tail.to_string()));
            }
        }
    }
    (title.trim().to_string(), None)
}

/// 歌手串按常见分隔符拆开（`A / B`、`A & B`、`A feat. B` …）。
fn split_artists(raw: &str) -> Vec<String> {
    let mut out = Vec::new();
    for part in raw.split([',', '，', '/', '、', '&', ';', '；', '|', '×', '·']) {
        let mut part = part.trim();
        let lower = part.to_lowercase();
        for marker in [" feat. ", " feat ", " ft. ", " ft ", " featuring ", " with "] {
            if let Some(index) = lower.find(marker) {
                part = part[..index].trim();
                break;
            }
        }
        if !part.is_empty() {
            out.push(part.to_string());
        }
    }
    out
}

/// 剥掉曲名 / 歌手里的修饰段：
/// 括号里是 `(Live)` / `(Remastered 2011)` / `【高音质】` 这类信息就整段丢弃，
/// 括号外残留的 `feat. xxx` 尾巴也一并截掉。
fn clean_title(title: &str) -> String {
    let mut out = String::with_capacity(title.len());
    let mut chars = title.chars().peekable();

    while let Some(ch) = chars.next() {
        let close = match ch {
            '(' => Some(')'),
            '（' => Some('）'),
            '[' => Some(']'),
            '【' => Some('】'),
            '〔' => Some('〕'),
            _ => None,
        };

        let Some(close) = close else {
            out.push(ch);
            continue;
        };

        let mut inner = String::new();
        let mut closed = false;
        for next in chars.by_ref() {
            if next == close {
                closed = true;
                break;
            }
            inner.push(next);
        }

        if !closed {
            // 括号没闭合：原样保留，别把后半句吃掉
            out.push(ch);
            out.push_str(&inner);
            break;
        }

        if is_decoration(&inner) {
            continue;
        }

        out.push(ch);
        out.push_str(&inner);
        out.push(close);
    }

    let mut text = out.trim().to_string();
    if let Some(index) = find_marker(&text, &[" feat. ", " feat ", " ft. ", " ft ", " featuring "]) {
        text.truncate(index);
    }

    collapse_spaces(text.trim())
}

/// 括号内容是否是「修饰」：命中修饰词、纯数字（年份 / 编号）、或以 feat 开头。
fn is_decoration(inner: &str) -> bool {
    let text = inner.trim();
    if text.is_empty() {
        return true;
    }
    if text.chars().all(|ch| ch.is_ascii_digit()) {
        return true;
    }

    let lower = text.to_lowercase();
    if lower.starts_with("feat") || lower.starts_with("ft.") || lower.starts_with("with ") {
        return true;
    }

    DECORATIONS.iter().any(|keyword| {
        if keyword.is_ascii() {
            // 英文修饰词要求成词匹配，否则 "(Discovery)" 会被 "cover" 命中
            contains_word(&lower, keyword)
        } else {
            lower.contains(keyword)
        }
    })
}

/// 文本里是否出现独立成词的 `needle`（两侧不能紧挨字母数字）。
fn contains_word(haystack: &str, needle: &str) -> bool {
    if needle.is_empty() || needle.len() > haystack.len() {
        return false;
    }

    let mut start = 0;
    while start < haystack.len() {
        let Some(found) = haystack[start..].find(needle) else {
            return false;
        };
        let begin = start + found;
        let end = begin + needle.len();

        let left_ok = begin == 0 || !haystack.as_bytes()[begin - 1].is_ascii_alphanumeric();
        let right_ok = end == haystack.len() || !haystack.as_bytes()[end].is_ascii_alphanumeric();
        if left_ok && right_ok {
            return true;
        }

        start = end;
    }

    false
}

fn find_marker(text: &str, markers: &[&str]) -> Option<usize> {
    let lower = text.to_lowercase();
    markers.iter().filter_map(|marker| lower.find(marker)).min()
}

fn collapse_spaces(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut last_space = false;
    for ch in text.chars() {
        if ch.is_whitespace() {
            if !last_space && !out.is_empty() {
                out.push(' ');
            }
            last_space = true;
        } else {
            out.push(ch);
            last_space = false;
        }
    }
    out.trim_end().to_string()
}

// ---------------- HTTP ----------------

#[derive(Debug, Clone, Copy)]
enum HttpError {
    /// 连接失败 / 超时：等价于「此刻没有网络」。
    Offline,
    /// 接口返回了错误状态码。
    Status(u16),
    /// 返回了内容但不是预期的 JSON。
    Parse,
}

impl HttpError {
    fn label(self) -> &'static str {
        match self {
            Self::Offline => "网络不可用",
            Self::Status(401 | 403) => "接口拒绝访问",
            Self::Status(404) => "接口不存在",
            Self::Status(429) => "接口限流",
            Self::Status(_) => "接口返回错误",
            Self::Parse => "返回内容无法解析",
        }
    }
}

fn http_json(request: ureq::Request) -> Result<serde_json::Value, HttpError> {
    read_json(request.call())
}

fn read_json(result: Result<ureq::Response, ureq::Error>) -> Result<serde_json::Value, HttpError> {
    match result {
        Ok(response) => response.into_json().map_err(|_| HttpError::Parse),
        Err(ureq::Error::Status(code, _)) => Err(HttpError::Status(code)),
        Err(_) => Err(HttpError::Offline),
    }
}

// ---------------- 匹配打分 ----------------

/// 一条候选歌词与当前曲目的匹配评分。
struct Match {
    score: i32,
    duration_diff: f64,
}

/// 一轮检索的匹配规则：时长容差、曲名门槛、是否必须歌手也对得上。
struct MatchRules {
    tolerance_ms: f64,
    min_title_score: i32,
    require_artist: bool,
}

/// 严格：时长必须接近，曲名不必一字不差（现场版 / 翻唱会被自然排除）。
const STRICT_RULES: MatchRules = MatchRules {
    tolerance_ms: STRICT_TOLERANCE_MS,
    min_title_score: MIN_TITLE_SCORE,
    require_artist: false,
};

/// 宽松：曲名与歌手都要对得上，但容忍时长标注不准的条目。
const LOOSE_RULES: MatchRules = MatchRules {
    tolerance_ms: LOOSE_TOLERANCE_MS,
    min_title_score: MIN_LOOSE_TITLE_SCORE,
    require_artist: true,
};

/// 网易云：搜索结果的曲名常带后缀，曲名门槛放宽，靠歌手与时长排序。
const NETEASE_RULES: MatchRules = MatchRules {
    tolerance_ms: LOOSE_TOLERANCE_MS,
    min_title_score: MIN_TITLE_SCORE,
    require_artist: false,
};

/// 打分：曲名是硬门槛，歌手与时长负责在不同版本之间排序。
fn score_match(
    query: &Query,
    track: &str,
    artist: &str,
    candidate_ms: Option<f64>,
    duration_ms: f64,
    rules: &MatchRules,
) -> Option<Match> {
    let title_score = title_similarity(&query.title, track);
    if title_score < rules.min_title_score {
        return None;
    }

    let mut score = title_score;

    if !query.artist.is_empty() {
        let want = normalize(&query.artist);
        let got = normalize(artist);
        if !want.is_empty() && !got.is_empty() {
            if got.contains(&want) || want.contains(&got) {
                score += 20;
            } else if rules.require_artist {
                return None;
            } else {
                // 歌手对不上：可能是翻唱 / 同名片，扣分但仍允许（时长会再筛一次）
                score -= 20;
            }
        }
    }

    let mut duration_diff = f64::MAX;
    if let Some(candidate) = candidate_ms {
        if duration_ms > 0.0 {
            let diff = (candidate - duration_ms).abs();
            if diff > rules.tolerance_ms {
                return None;
            }
            duration_diff = diff;
            score += if diff <= 2_000.0 {
                25
            } else if diff <= 8_000.0 {
                12
            } else {
                0
            };
        }
    }

    Some(Match {
        score,
        duration_diff,
    })
}

/// 收集一轮检索里评分最高的可用条目。
struct Picker {
    best: Option<(i32, f64, String)>,
}

impl Picker {
    fn new() -> Self {
        Self { best: None }
    }

    fn offer(&mut self, lrc: &str, matched: Option<Match>) {
        let Some(matched) = matched else {
            return;
        };
        if !has_time_tags(lrc) {
            return;
        }

        let better = match &self.best {
            None => true,
            Some((score, diff, _)) => {
                matched.score > *score
                    || (matched.score == *score && matched.duration_diff < *diff)
            }
        };

        if better {
            self.best = Some((matched.score, matched.duration_diff, lrc.to_string()));
        }
    }

    fn into_hit(self, origin: LyricsOrigin) -> Option<LyricsHit> {
        let (_, _, lrc) = self.best?;
        Some(LyricsHit {
            body: LyricsBody::Text(lrc),
            origin,
        })
    }
}

/// 曲名相似度（0~100）：完全一致 100，包含关系 70~95，其余用双字组 Dice 系数。
fn title_similarity(query: &str, candidate: &str) -> i32 {
    let left = normalize(query);
    let right = normalize(candidate);
    if left.is_empty() || right.is_empty() {
        return 0;
    }
    if left == right {
        return 100;
    }
    if left.contains(&right) || right.contains(&left) {
        let short = left.chars().count().min(right.chars().count()) as f64;
        let long = left.chars().count().max(right.chars().count()) as f64;
        let ratio = if long > 0.0 { short / long } else { 0.0 };
        return (70.0 + 25.0 * ratio).round() as i32;
    }
    (dice(&left, &right) * 90.0).round() as i32
}

/// 双字组 Dice 系数（0~1）：对中文标题的近似匹配比编辑距离更稳。
fn dice(left: &str, right: &str) -> f64 {
    let left_grams = bigrams(left);
    let right_grams = bigrams(right);
    if left_grams.is_empty() || right_grams.is_empty() {
        return 0.0;
    }

    let mut hits = 0usize;
    for gram in &left_grams {
        if right_grams.contains(gram) {
            hits += 1;
        }
    }

    (2.0 * hits as f64) / ((left_grams.len() + right_grams.len()) as f64)
}

fn bigrams(text: &str) -> HashSet<String> {
    let chars: Vec<char> = text.chars().collect();
    if chars.len() < 2 {
        return chars.iter().map(|ch| ch.to_string()).collect();
    }

    let mut out = HashSet::with_capacity(chars.len());
    for index in 0..chars.len() - 1 {
        out.insert(format!("{}{}", chars[index], chars[index + 1]));
    }
    out
}

// ---------------- lrclib.net ----------------

/// lrclib.net 开放接口：返回歌词条目数组，按曲名 / 歌手 / 时长择优。
fn lrclib_search(
    query: &Query,
    duration_ms: f64,
    rules: &MatchRules,
) -> Result<Option<LyricsHit>, HttpError> {
    let mut request = ureq::get("https://lrclib.net/api/search")
        .query("track_name", &query.title)
        .timeout(REQUEST_TIMEOUT)
        .set("User-Agent", USER_AGENT);

    if !query.artist.is_empty() {
        request = request.query("artist_name", &query.artist);
    }

    let value = http_json(request)?;
    let items = match value.as_array() {
        Some(items) => items,
        None => return Err(HttpError::Parse),
    };

    let mut picker = Picker::new();
    for item in items {
        let synced = item
            .get("syncedLyrics")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        if synced.trim().is_empty() {
            continue;
        }

        let track = item
            .get("trackName")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let artist = item
            .get("artistName")
            .and_then(|value| value.as_str())
            .unwrap_or("");
        let candidate_ms = item
            .get("duration")
            .and_then(|value| value.as_f64())
            .map(|seconds| seconds * 1000.0);

        picker.offer(
            synced,
            score_match(query, track, artist, candidate_ms, duration_ms, rules),
        );
    }

    Ok(picker.into_hit(LyricsOrigin::Lrclib))
}

// ---------------- 网易云音乐 ----------------

/// 网易云音乐 web 接口兜底：先按「曲名 / 歌手 / 时长」选歌曲 id，再取歌词。
fn netease_search(query: &Query, duration_ms: f64) -> Result<Option<LyricsHit>, HttpError> {
    let keyword = if query.artist.trim().is_empty() {
        query.title.clone()
    } else {
        format!("{} {}", query.title, query.artist)
    };

    let value = read_json(
        ureq::post("https://music.163.com/api/search/get")
            .timeout(REQUEST_TIMEOUT)
            .set("User-Agent", USER_AGENT)
            .set("Referer", "https://music.163.com")
            .set("Cookie", "os=pc; appver=2.0.2")
            .send_form(&[
                ("s", keyword.as_str()),
                ("type", "1"),
                ("offset", "0"),
                ("limit", "10"),
            ]),
    )?;

    let songs = match value.pointer("/result/songs").and_then(|v| v.as_array()) {
        Some(songs) => songs,
        None => return Ok(None),
    };

    let mut best: Option<(i32, f64, i64)> = None;
    for song in songs {
        let Some(id) = song.get("id").and_then(|value| value.as_i64()) else {
            continue;
        };

        let track = song.get("name").and_then(|value| value.as_str()).unwrap_or("");
        let artist = song
            .get("artists")
            .and_then(|value| value.as_array())
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.get("name").and_then(|v| v.as_str()))
                    .collect::<Vec<_>>()
                    .join("/")
            })
            .unwrap_or_default();
        let candidate_ms = song.get("duration").and_then(|value| value.as_f64());

        let Some(matched) = score_match(
            query,
            track,
            &artist,
            candidate_ms,
            duration_ms,
            &NETEASE_RULES,
        ) else {
            continue;
        };

        let better = match &best {
            None => true,
            Some((score, diff, _)) => {
                matched.score > *score || (matched.score == *score && matched.duration_diff < *diff)
            }
        };
        if better {
            best = Some((matched.score, matched.duration_diff, id));
        }
    }

    let Some((_, _, song_id)) = best else {
        return Ok(None);
    };

    let value = http_json(
        ureq::get("https://music.163.com/api/song/lyric")
            .query("id", &song_id.to_string())
            .query("lv", "-1")
            .query("kv", "-1")
            .query("tv", "-1")
            .timeout(REQUEST_TIMEOUT)
            .set("User-Agent", USER_AGENT)
            .set("Referer", "https://music.163.com")
            .set("Cookie", "os=pc; appver=2.0.2"),
    )?;

    let lrc = value
        .pointer("/lrc/lyric")
        .and_then(|value| value.as_str())
        .unwrap_or("");

    // 纯文本歌词（没有时间轴）在这里就丢掉，好让本地文件有机会兜底。
    if lrc.trim().is_empty() || !has_time_tags(lrc) {
        return Ok(None);
    }

    Ok(Some(LyricsHit {
        body: LyricsBody::Text(lrc.to_string()),
        origin: LyricsOrigin::Netease,
    }))
}

// ---------------- 本地文件 ----------------

/// 本地歌词文件兜底：在 `%LOCALAPPDATA%\WinIslandPlus\lyrics` 里按文件名匹配。
///
/// 文件名里包含曲名即算命中；同时包含歌手、或文件名恰好等于曲名时优先。
fn fetch_from_local(title: &str, artist: &str) -> Option<LyricsHit> {
    let want_title = normalize(&clean_title(title));
    if want_title.is_empty() {
        return None;
    }
    let want_artist = normalize(artist);

    let mut best: Option<(i32, PathBuf)> = None;
    for entry in fs::read_dir(lyrics_dir()).ok()?.flatten() {
        let path = entry.path();
        if !path.is_file() {
            continue;
        }

        let extension = path
            .extension()
            .and_then(|value| value.to_str())
            .map(|value| value.to_ascii_lowercase())
            .unwrap_or_default();
        if !LOCAL_EXTENSIONS.contains(&extension.as_str()) {
            continue;
        }

        let haystack = normalize(
            path.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(""),
        );
        // 双向包含：文件名里带曲名，或曲名里带文件名（文件名取短标题的情况）。
        // 反向匹配要求文件名至少两个字，否则 `1.lrc` 这种会把所有带 1 的曲名都收进来。
        let reverse_ok = haystack.chars().count() >= 2 && want_title.contains(&haystack);
        if !haystack.contains(&want_title) && !reverse_ok {
            continue;
        }

        let mut score = 10;
        if extension == "lrc" {
            score += 2;
        }
        if !want_artist.is_empty() && haystack.contains(&want_artist) {
            score += 5;
        }
        if haystack == want_title {
            score += 6;
        }

        match &best {
            Some((best_score, _)) if *best_score >= score => {}
            _ => best = Some((score, path)),
        }
    }

    let (_, path) = best?;
    let bytes = fs::read(&path).ok()?;
    if bytes.is_empty() {
        return None;
    }

    Some(LyricsHit {
        body: LyricsBody::Bytes(bytes),
        origin: LyricsOrigin::Local,
    })
}

/// 本地歌词目录（与日志目录同级）。
fn lyrics_dir() -> PathBuf {
    log::root_dir().join("lyrics")
}

/// 归一：只留字母数字（含汉字）并转小写，忽略空格、括号、连字符等。
fn normalize(input: &str) -> String {
    input
        .chars()
        .filter(|ch| ch.is_alphanumeric())
        .flat_map(|ch| ch.to_lowercase())
        .collect()
}

/// 文本里是否至少有一个 `[分:秒]` 时间标签。
fn has_time_tags(text: &str) -> bool {
    let bytes = text.as_bytes();
    let mut index = 0;
    while index + 3 < bytes.len() {
        if bytes[index] == b'[' && bytes[index + 1].is_ascii_digit() {
            let mut cursor = index + 1;
            while cursor < bytes.len() && bytes[cursor].is_ascii_digit() {
                cursor += 1;
            }
            if cursor > index + 1 && cursor < bytes.len() && bytes[cursor] == b':' {
                return true;
            }
        }
        index += 1;
    }
    false
}

fn to_hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut out = String::with_capacity(bytes.len() * 2);
    for byte in bytes {
        out.push(DIGITS[(byte >> 4) as usize] as char);
        out.push(DIGITS[(byte & 0x0f) as usize] as char);
    }
    out
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|duration| duration.as_millis() as f64)
        .unwrap_or(0.0)
}

// ---------------- 缓存 ----------------

struct CacheEntry {
    hit: Option<LyricsHit>,
    stored_ms: f64,
}

struct LyricsCache {
    limit: usize,
    entries: HashMap<String, CacheEntry>,
    order: VecDeque<String>,
}

impl LyricsCache {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            entries: HashMap::new(),
            order: VecDeque::new(),
        }
    }

    /// 取缓存；过期条目视同没有（命中与未命中用不同 TTL）。
    fn get(&mut self, key: &str) -> Option<Option<LyricsHit>> {
        let entry = self.entries.get(key)?;
        let ttl = if entry.hit.is_some() {
            HIT_TTL_MS
        } else {
            MISS_TTL_MS
        };
        let expired = now_ms() - entry.stored_ms > ttl;

        if expired {
            self.entries.remove(key);
            self.order.retain(|item| item != key);
            return None;
        }

        Some(entry.hit.clone())
    }

    /// 无论有无歌词都缓存结果（未命中的条目按更短的 TTL 过期）。
    fn put(&mut self, key: String, hit: Option<LyricsHit>) {
        self.entries.insert(
            key.clone(),
            CacheEntry {
                hit,
                stored_ms: now_ms(),
            },
        );
        self.order.retain(|item| item != &key);
        self.order.push_back(key);

        while self.order.len() > self.limit {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn clean_title_strips_decorations() {
        assert_eq!(clean_title("夜曲"), "夜曲");
        assert_eq!(clean_title("晴天 (Live)"), "晴天");
        assert_eq!(clean_title("Shape of You (feat. XYZ)"), "Shape of You");
        assert_eq!(clean_title("七里香【高音质】"), "七里香");
        assert_eq!(clean_title("稻香 (2011)"), "稻香");
        // 成词匹配：不含修饰词的括号要保留
        assert_eq!(clean_title("Discovery (Album)"), "Discovery (Album)");
        assert_eq!(clean_title("夜曲 feat. 杨瑞代"), "夜曲");
    }

    #[test]
    fn query_candidates_handles_dash_forms() {
        // 「歌手 - 歌名」：首段就是歌手，要把两段对调
        let queries = query_candidates("周杰伦 - 夜曲", "周杰伦");
        assert_eq!(queries[0].title, "夜曲");
        assert_eq!(queries[0].artist, "周杰伦");

        // 「歌名 - 歌手」：首段是曲名
        let queries = query_candidates("夜曲 - 周杰伦", "周杰伦");
        assert_eq!(queries[0].title, "夜曲");
        assert_eq!(queries[0].artist, "周杰伦");

        // 合唱：歌手串要按分隔符拆开，第二位歌手也有一次机会
        let queries = query_candidates("来日方长", "周杰伦 & 杨瑞代");
        assert_eq!(queries[0].artist, "周杰伦");
        assert!(queries.iter().any(|query| query.artist == "杨瑞代"));
    }

    #[test]
    fn title_similarity_basics() {
        assert_eq!(title_similarity("夜曲", "夜曲"), 100);
        assert!(title_similarity("夜曲", "夜曲 (Live)") >= 70);
        assert!(title_similarity("夜曲", "完全不相干") < MIN_TITLE_SCORE);
    }

    #[test]
    fn score_match_rejects_far_durations() {
        let query = Query {
            title: "夜曲".to_string(),
            artist: "周杰伦".to_string(),
        };

        let near = score_match(&query, "夜曲", "周杰伦", Some(226_000.0), 225_000.0, &STRICT_RULES);
        assert!(near.is_some());

        let far = score_match(&query, "夜曲", "周杰伦", Some(300_000.0), 225_000.0, &STRICT_RULES);
        assert!(far.is_none());

        // 歌名对不上时，时长再接近也不能用
        let wrong = score_match(&query, "发如雪", "周杰伦", Some(225_000.0), 225_000.0, &LOOSE_RULES);
        assert!(wrong.is_none());

        // 宽松规则要求歌手也对得上
        let cover = score_match(&query, "夜曲", "某翻唱歌手", Some(225_000.0), 225_000.0, &LOOSE_RULES);
        assert!(cover.is_none());
    }
}

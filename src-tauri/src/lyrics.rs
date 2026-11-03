//! 歌词检索：曲目变化后按「歌名 + 歌手 + 时长」查找带时间轴的 LRC。
//!
//! 独占一个线程跑阻塞式 HTTP（不占用媒体线程的轮询）；结果做内存缓存，
//! 无论是否找到歌词都会发出 `lyrics://changed`，由前端决定回退展示。
//!
//! 来源顺序：lrclib.net 开放接口 → 网易云音乐 web 接口 → 本地歌词文件兜底。
//!
//! 时长用于择优：同一首歌往往有专辑版 / 现场版 / 翻唱多个条目，
//! 时间轴差别很大，只按时长最接近的那个算「同一版本」。

use std::collections::{HashMap, VecDeque};
use std::fs;
use std::path::PathBuf;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde::Serialize;
use tauri::{AppHandle, Emitter};

use crate::log;

pub const EVENT_LYRICS_CHANGED: &str = "lyrics://changed";

/// 歌词缓存上限（LRU）。
const CACHE_LIMIT: usize = 64;
/// 单次 HTTP 请求超时。
const REQUEST_TIMEOUT: Duration = Duration::from_secs(6);
/// 候选条目与当前曲目时长的最大容差（毫秒）：超出就认为不是同一版本。
const DURATION_TOLERANCE_MS: f64 = 8_000.0;
/// 本地歌词文件的后缀（大小写不敏感）。
const LOCAL_EXTENSIONS: [&str; 2] = ["lrc", "txt"];

const USER_AGENT: &str = "Mozilla/5.0 (Windows NT 10.0; Win64; x64) WinIslandPlus/1.1";

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

enum LyricsCommand {
    Request {
        key: String,
        title: String,
        artist: String,
        album_title: String,
        duration_ms: f64,
    },
}

struct Inner {
    commands: Mutex<Option<Sender<LyricsCommand>>>,
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
            }),
        }
    }

    /// 请求某首歌的歌词：工作线程去重 + 缓存，结果经事件送达。
    ///
    /// `duration_ms` 用于在多个同名条目里挑时长最接近的版本（0 表示未知）。
    pub fn request(&self, title: &str, artist: &str, album_title: &str, duration_ms: f64) {
        let command = LyricsCommand::Request {
            key: format!("{title}\u{1f}{artist}\u{1f}{album_title}"),
            title: title.to_string(),
            artist: artist.to_string(),
            album_title: album_title.to_string(),
            duration_ms,
        };
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

        std::thread::Builder::new()
            .name("winisland-lyrics".into())
            .spawn(move || run(app, rx))
            .ok();
    }
}

fn run(app: AppHandle, rx: Receiver<LyricsCommand>) {
    let mut cache = LyricsCache::new(CACHE_LIMIT);
    log::info(&format!(
        "lyrics: 本地歌词目录 {}",
        lyrics_dir().display()
    ));

    while let Ok(LyricsCommand::Request {
        key,
        title,
        artist,
        album_title,
        duration_ms,
    }) = rx.recv()
    {
        let hit = match cache.get(&key) {
            Some(cached) => cached,
            None => {
                let fetched = fetch_lyrics(&title, &artist, duration_ms);
                cache.put(key, fetched.clone());
                fetched
            }
        };

        if let Some(found) = hit.as_ref() {
            log::info(&format!("lyrics: 命中 {} 歌词", found.origin.label()));
        } else {
            log::info("lyrics: 未找到可用歌词");
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
                title,
                artist,
                album_title,
                lrc,
                lrc_hex,
                origin,
            },
        );
    }

    log::info("lyrics: 歌词线程退出");
}

fn fetch_lyrics(title: &str, artist: &str, duration_ms: f64) -> Option<LyricsHit> {
    fetch_from_lrclib(title, artist, duration_ms)
        .or_else(|| fetch_from_netease(title, artist, duration_ms))
        .or_else(|| fetch_from_local(title, artist))
}

/// lrclib.net 开放接口：返回歌词条目数组，取「时长最接近 + 带时间轴」的一条。
fn fetch_from_lrclib(title: &str, artist: &str, duration_ms: f64) -> Option<LyricsHit> {
    let response = ureq::get("https://lrclib.net/api/search")
        .query("track_name", title)
        .query("artist_name", artist)
        .timeout(REQUEST_TIMEOUT)
        .set("User-Agent", USER_AGENT)
        .call()
        .ok()?;

    let value: serde_json::Value = response.into_json().ok()?;
    let items = value.as_array()?;

    let mut best: Option<(f64, String)> = None;
    for item in items {
        let synced = item
            .get("syncedLyrics")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        if !synced.trim().is_empty() && has_time_tags(synced) {
            let candidate_ms = item
                .get("duration")
                .and_then(|v| v.as_f64())
                .map(|seconds| seconds * 1000.0);
            let score = duration_score(candidate_ms, duration_ms);
            match &best {
                Some((best_score, _)) if *best_score <= score => {}
                _ => best = Some((score, synced.to_string())),
            }
        }
    }

    let (score, lrc) = best?;
    if duration_ms > 0.0 && score > DURATION_TOLERANCE_MS {
        return None;
    }

    Some(LyricsHit {
        body: LyricsBody::Text(lrc),
        origin: LyricsOrigin::Lrclib,
    })
}

/// 网易云音乐 web 接口兜底：先按「时长最接近」选歌曲 id，再取歌词。
fn fetch_from_netease(title: &str, artist: &str, duration_ms: f64) -> Option<LyricsHit> {
    let keyword = if artist.trim().is_empty() {
        title.to_string()
    } else {
        format!("{title} {artist}")
    };

    let search = ureq::get("https://music.163.com/api/search/get/web")
        .query("s", &keyword)
        .query("type", "1")
        .query("limit", "10")
        .timeout(REQUEST_TIMEOUT)
        .set("User-Agent", USER_AGENT)
        .set("Referer", "https://music.163.com")
        .call()
        .ok()?;

    let search_value: serde_json::Value = search.into_json().ok()?;
    let songs = search_value.pointer("/result/songs")?.as_array()?;

    let mut song_id: Option<i64> = None;
    let mut best_score = f64::MAX;
    for song in songs {
        let id = match song.get("id").and_then(|v| v.as_i64()) {
            Some(id) => id,
            None => continue,
        };
        let candidate_ms = song.get("duration").and_then(|v| v.as_f64());
        let score = duration_score(candidate_ms, duration_ms);
        if score < best_score {
            best_score = score;
            song_id = Some(id);
        }
    }

    let song_id = song_id?;
    if duration_ms > 0.0 && best_score > DURATION_TOLERANCE_MS {
        return None;
    }

    let lyric = ureq::get("https://music.163.com/api/song/lyric")
        .query("id", &song_id.to_string())
        .query("lv", "-1")
        .query("kv", "-1")
        .query("tv", "-1")
        .timeout(REQUEST_TIMEOUT)
        .set("User-Agent", USER_AGENT)
        .set("Referer", "https://music.163.com")
        .call()
        .ok()?;

    let lyric_value: serde_json::Value = lyric.into_json().ok()?;
    let lrc = lyric_value
        .pointer("/lrc/lyric")
        .and_then(|v| v.as_str())
        .unwrap_or("");

    // 纯文本歌词（没有时间轴）在这里就丢掉，好让本地文件有机会兜底。
    if lrc.trim().is_empty() || !has_time_tags(lrc) {
        return None;
    }

    Some(LyricsHit {
        body: LyricsBody::Text(lrc.to_string()),
        origin: LyricsOrigin::Netease,
    })
}

/// 本地歌词文件兜底：在 `%LOCALAPPDATA%\WinIslandPlus\lyrics` 里按文件名匹配。
///
/// 文件名里包含曲名即算命中；同时包含歌手、或文件名恰好等于曲名时优先。
fn fetch_from_local(title: &str, artist: &str) -> Option<LyricsHit> {
    let want_title = normalize_name(title);
    if want_title.is_empty() {
        return None;
    }
    let want_artist = normalize_name(artist);

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

        let haystack = normalize_name(
            path.file_stem()
                .and_then(|value| value.to_str())
                .unwrap_or(""),
        );
        if !haystack.contains(&want_title) {
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

/// 文件名归一：只留字母数字（含汉字）并转小写，忽略空格、括号、连字符等。
fn normalize_name(input: &str) -> String {
    input
        .chars()
        .filter(|c| c.is_alphanumeric())
        .flat_map(|c| c.to_lowercase())
        .collect()
}

/// 候选时长与当前曲目的差值；任一方未知时返回一个很大的值。
fn duration_score(candidate_ms: Option<f64>, duration_ms: f64) -> f64 {
    match candidate_ms {
        Some(candidate) if duration_ms > 0.0 => (candidate - duration_ms).abs(),
        _ => f64::MAX / 4.0,
    }
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

struct LyricsCache {
    limit: usize,
    entries: HashMap<String, Option<LyricsHit>>,
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

    fn get(&self, key: &str) -> Option<Option<LyricsHit>> {
        self.entries.get(key).cloned()
    }

    /// 无论有无歌词都缓存结果（避免对无歌词的歌反复打网络）。
    fn put(&mut self, key: String, hit: Option<LyricsHit>) {
        self.entries.insert(key.clone(), hit);
        self.order.retain(|item| item != &key);
        self.order.push_back(key);

        while self.order.len() > self.limit {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

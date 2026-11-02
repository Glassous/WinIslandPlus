//! 基于系统 SMTC (`GlobalSystemMediaTransportControlsSessionManager`) 的媒体会话。
//!
//! 所有 WinRT 调用都跑在一条独占的 MTA 线程上：UI 线程只读已序列化的快照，
//! 媒体线程负责轮询、封面解码与取色，避免把 WinRT 对象跨线程带出去。

mod accent;

use std::collections::{HashMap, VecDeque};
use std::future::IntoFuture;
use std::sync::mpsc::{channel, Receiver, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use base64::Engine;
use serde::Serialize;
use tauri::{AppHandle, Emitter};
use windows::core::Interface;
use windows::Foundation::TimeSpan;
use windows_future::IAsyncOperation;
use windows::Media::Control::{
    GlobalSystemMediaTransportControlsSession,
    GlobalSystemMediaTransportControlsSessionManager,
    GlobalSystemMediaTransportControlsSessionMediaProperties,
    GlobalSystemMediaTransportControlsSessionPlaybackStatus,
};
use windows::Storage::Streams::{DataReader, IRandomAccessStream};

use crate::log;

pub const EVENT_MEDIA_CHANGED: &str = "media://changed";

/// 封面缓存上限。
const ARTWORK_CACHE_LIMIT: usize = 8;
/// 播放中的轮询间隔。
const POLL_PLAYING_MS: u64 = 350;
/// 暂停 / 无会话时的轮询间隔。
const POLL_IDLE_MS: u64 = 1000;

type Session = GlobalSystemMediaTransportControlsSession;
type Manager = GlobalSystemMediaTransportControlsSessionManager;

/// 某一时刻的媒体状态快照。不可变，避免跨线程共享可变对象。
#[derive(Serialize, Clone, Debug, Default, PartialEq)]
#[serde(rename_all = "PascalCase")]
pub struct MediaSnapshot {
    pub title: String,
    pub artist: String,
    pub album_title: String,
    pub source_app_name: String,
    pub is_playing: bool,
    /// 采样的播放位置（毫秒）。
    pub position_ms: f64,
    /// 总时长（毫秒），0 表示无法跳转。
    pub duration_ms: f64,
    /// 采样位置的本地时刻（Unix 毫秒）。
    pub position_stamp_ms: f64,
    /// 封面 data URL。
    pub artwork_url: Option<String>,
    pub artwork_version: i32,
    /// 封面主色 `#RRGGBB`。
    pub accent_color: Option<String>,
    /// 封面次色 `#RRGGBB`。
    pub accent_color_alt: Option<String>,
}

impl MediaSnapshot {
    fn track_key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.title, self.artist, self.album_title
        )
    }
}

enum MediaCommand {
    TogglePlayPause,
    Next,
    Previous,
    Seek(i64),
}

/// WinRT 异步操作 → 阻塞等待（媒体线程独占，不会卡 UI）。
fn wait<T>(operation: IAsyncOperation<T>) -> windows::core::Result<T>
where
    T: windows::core::RuntimeType,
    IAsyncOperation<T>: IntoFuture<Output = windows::core::Result<T>>,
{
    pollster::block_on(operation.into_future())
}

struct Inner {
    current: Arc<Mutex<Option<MediaSnapshot>>>,
    commands: Mutex<Option<Sender<MediaCommand>>>,
}

/// 媒体服务句柄（克隆成本极低）。
#[derive(Clone)]
pub struct MediaService {
    inner: Arc<Inner>,
}

impl MediaService {
    pub fn new() -> Self {
        Self {
            inner: Arc::new(Inner {
                current: Arc::new(Mutex::new(None)),
                commands: Mutex::new(None),
            }),
        }
    }

    pub fn current(&self) -> Option<MediaSnapshot> {
        self.inner
            .current
            .lock()
            .ok()
            .and_then(|guard| guard.clone())
    }

    /// 只取播放状态，避免为了一个布尔值克隆整个快照。
    pub fn is_playing(&self) -> Option<bool> {
        self.inner
            .current
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|snapshot| snapshot.is_playing))
    }

    fn send(&self, command: MediaCommand) {
        if let Ok(guard) = self.inner.commands.lock() {
            if let Some(tx) = guard.as_ref() {
                let _ = tx.send(command);
            }
        }
    }

    pub fn toggle_play_pause(&self) -> bool {
        self.send(MediaCommand::TogglePlayPause);
        true
    }

    pub fn next(&self) -> bool {
        self.send(MediaCommand::Next);
        true
    }

    pub fn previous(&self) -> bool {
        self.send(MediaCommand::Previous);
        true
    }

    pub fn seek(&self, position_ms: f64) -> bool {
        if !position_ms.is_finite() || position_ms < 0.0 {
            return false;
        }
        self.send(MediaCommand::Seek((position_ms * 10_000.0) as i64));
        true
    }

    /// 启动媒体线程（应用装配时调用一次）。
    pub fn start(&self, app: AppHandle) {
        let (tx, rx) = channel::<MediaCommand>();
        if let Ok(mut guard) = self.inner.commands.lock() {
            *guard = Some(tx);
        }

        let current = self.inner.current.clone();
        std::thread::Builder::new()
            .name("winisland-media".into())
            .spawn(move || run(app, current, rx))
            .ok();
    }
}

fn run(
    app: AppHandle,
    current: Arc<Mutex<Option<MediaSnapshot>>>,
    rx: Receiver<MediaCommand>,
) {
    // MTA 套间：SMTC 的异步调用在这里阻塞等待完成
    unsafe {
        let _ = windows::Win32::System::Com::CoInitializeEx(
            None,
            windows::Win32::System::Com::COINIT_MULTITHREADED,
        );
    }

    let mut manager: Option<Manager> = None;
    let mut cache = ArtworkCache::new(ARTWORK_CACHE_LIMIT);
    let mut last_emitted: Option<MediaSnapshot> = None;
    let mut manager_retry_at = 0.0f64;

    loop {
        let playing = current
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|s| s.is_playing))
            .unwrap_or(false);

        let interval = Duration::from_millis(if playing {
            POLL_PLAYING_MS
        } else {
            POLL_IDLE_MS
        });

        match rx.recv_timeout(interval) {
            Ok(command) => {
                if manager.is_none() {
                    manager = request_manager(&mut manager_retry_at);
                }
                if let Some(manager) = manager.as_ref() {
                    apply_command(manager, command);
                }
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => break,
        }

        if manager.is_none() {
            manager = request_manager(&mut manager_retry_at);
        }

        let Some(manager) = manager.as_ref() else {
            continue;
        };

        let snapshot = probe(manager, &mut cache);

        let should_emit = match (&snapshot, &last_emitted) {
            (None, None) => false,
            (None, Some(_)) => true,
            (Some(new), Some(old)) => {
                new.track_key() != old.track_key()
                    || new.is_playing != old.is_playing
                    || new.artwork_version != old.artwork_version
                    || (new.duration_ms - old.duration_ms).abs() > 300.0
                    || new.source_app_name != old.source_app_name
                    || (expected_position(old) - new.position_ms).abs() > 700.0
            }
            (Some(_), None) => true,
        };

        if let Ok(mut guard) = current.lock() {
            *guard = snapshot.clone();
        }

        if should_emit {
            let _ = app.emit(EVENT_MEDIA_CHANGED, snapshot.clone());
            last_emitted = snapshot;
        }
    }

    log::info("media: 媒体线程已退出");
}

fn expected_position(snapshot: &MediaSnapshot) -> f64 {
    if snapshot.is_playing {
        snapshot.position_ms + (now_ms() - snapshot.position_stamp_ms)
    } else {
        snapshot.position_ms
    }
}

fn request_manager(retry_at: &mut f64) -> Option<Manager> {
    if now_ms() < *retry_at {
        return None;
    }

    let result = Manager::RequestAsync().and_then(wait);

    match result {
        Ok(manager) => {
            log::info("media: SMTC 管理器就绪");
            Some(manager)
        }
        Err(err) => {
            log::warn(&format!("media: SMTC 初始化失败，稍后重试（{err}）"));
            *retry_at = now_ms() + 5_000.0;
            None
        }
    }
}

fn apply_command(manager: &Manager, command: MediaCommand) {
    let Some(session) = pick_session(manager) else {
        return;
    };

    let result = match command {
        MediaCommand::TogglePlayPause => session.TryTogglePlayPauseAsync().and_then(wait).map(|_| ()),
        MediaCommand::Next => session.TrySkipNextAsync().and_then(wait).map(|_| ()),
        MediaCommand::Previous => session.TrySkipPreviousAsync().and_then(wait).map(|_| ()),
        MediaCommand::Seek(ticks) => session
            .TryChangePlaybackPositionAsync(ticks)
            .and_then(wait)
            .map(|_| ()),
    };

    if let Err(err) = result {
        log::warn(&format!("media: 控制指令执行失败（{err}）"));
    }
}

/// 优先取「正在播放」的会话，其次当前会话，最后任意一个。
fn pick_session(manager: &Manager) -> Option<Session> {
    let mut fallback: Option<Session> = None;

    if let Ok(sessions) = manager.GetSessions() {
        let count = sessions.Size().unwrap_or(0);
        for index in 0..count {
            let Ok(session) = sessions.GetAt(index) else {
                continue;
            };

            let is_playing = session
                .GetPlaybackInfo()
                .and_then(|info| info.PlaybackStatus())
                .map(|status| status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
                .unwrap_or(false);

            if is_playing {
                return Some(session);
            }

            if fallback.is_none() {
                fallback = Some(session);
            }
        }
    }

    if let Ok(current) = manager.GetCurrentSession() {
        return Some(current);
    }

    fallback
}

fn probe(manager: &Manager, cache: &mut ArtworkCache) -> Option<MediaSnapshot> {
    let session = pick_session(manager)?;

    let playback = session.GetPlaybackInfo().ok()?;
    let is_playing = playback
        .PlaybackStatus()
        .map(|status| status == GlobalSystemMediaTransportControlsSessionPlaybackStatus::Playing)
        .unwrap_or(false);

    let props = session
        .TryGetMediaPropertiesAsync()
        .ok()
        .and_then(|op| wait(op).ok())?;

    let title = props.Title().map(|s| s.to_string()).unwrap_or_default();
    let artist = props.Artist().map(|s| s.to_string()).unwrap_or_default();
    let album = props.AlbumTitle().map(|s| s.to_string()).unwrap_or_default();

    if title.trim().is_empty() && artist.trim().is_empty() {
        return None;
    }

    let (duration_ms, position_ms) = match session.GetTimelineProperties() {
        Ok(timeline) => {
            let start = timeline.StartTime().map(to_ms).unwrap_or(0.0);
            let end = timeline.EndTime().map(to_ms).unwrap_or(0.0);
            let position = timeline.Position().map(to_ms).unwrap_or(0.0);
            ((end - start).max(0.0), (position - start).max(0.0))
        }
        Err(_) => (0.0, 0.0),
    };

    let display_title = if title.trim().is_empty() {
        album.clone()
    } else {
        title.clone()
    };
    let key = format!("{display_title}\u{1f}{artist}\u{1f}{album}");

    let artwork = match cache.get(&key) {
        Some(cached) => Some(cached),
        None => {
            let resolved = resolve_artwork(&props);
            cache.put(key.clone(), &resolved);
            resolved
        }
    };

    let source_app = session
        .SourceAppUserModelId()
        .map(|id| display_name_for(&id.to_string()))
        .unwrap_or_default();

    Some(MediaSnapshot {
        title: display_title,
        artist,
        album_title: album,
        source_app_name: source_app,
        is_playing,
        position_ms,
        duration_ms,
        position_stamp_ms: now_ms(),
        artwork_url: artwork.as_ref().map(|a| a.url.clone()),
        artwork_version: artwork.as_ref().map(|a| a.version).unwrap_or(0),
        accent_color: artwork.as_ref().map(|a| a.primary.clone()),
        accent_color_alt: artwork.as_ref().map(|a| a.secondary.clone()),
    })
}

fn to_ms(span: TimeSpan) -> f64 {
    span.Duration as f64 / 10_000.0
}

fn now_ms() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_millis() as f64)
        .unwrap_or(0.0)
}

// ---------------- 封面与取色 ----------------

#[derive(Clone)]
struct CachedArtwork {
    url: String,
    version: i32,
    primary: String,
    secondary: String,
}

struct ArtworkCache {
    limit: usize,
    entries: HashMap<String, CachedArtwork>,
    order: VecDeque<String>,
    next_version: i32,
}

impl ArtworkCache {
    fn new(limit: usize) -> Self {
        Self {
            limit,
            entries: HashMap::new(),
            order: VecDeque::new(),
            next_version: 0,
        }
    }

    fn get(&self, key: &str) -> Option<CachedArtwork> {
        self.entries.get(key).cloned()
    }

    /// 只缓存解码成功的封面；未解码时用 None 占位（避免反复尝试拖慢轮询）。
    fn put(&mut self, key: String, artwork: &Option<CachedArtwork>) {
        let Some(artwork) = artwork else {
            return;
        };

        let mut artwork = artwork.clone();
        if artwork.version == 0 {
            self.next_version += 1;
            artwork.version = self.next_version;
        }

        self.entries.insert(key.clone(), artwork);
        self.order.retain(|item| item != &key);
        self.order.push_back(key);

        while self.order.len() > self.limit {
            if let Some(oldest) = self.order.pop_front() {
                self.entries.remove(&oldest);
            }
        }
    }
}

fn resolve_artwork(
    props: &GlobalSystemMediaTransportControlsSessionMediaProperties,
) -> Option<CachedArtwork> {
    let thumbnail = props.Thumbnail().ok()?;
    let stream = thumbnail.OpenReadAsync().ok().and_then(|op| wait(op).ok())?;

    let size = stream.Size().ok()? as u32;
    if size == 0 {
        return None;
    }

    let base: IRandomAccessStream = stream.cast().ok()?;
    let reader = DataReader::CreateDataReader(&base).ok()?;
    wait(reader.LoadAsync(size).ok()?).ok()?;

    let mut buffer = vec![0u8; size as usize];
    reader.ReadBytes(&mut buffer).ok()?;

    let image = image::load_from_memory(&buffer).ok()?;

    // 取色：32×32 缩略图，按色相分桶提取主 / 次色
    let small = image.resize_exact(32, 32, image::imageops::FilterType::Triangle);
    let small_rgba = small.to_rgba8();
    let mut bgra = Vec::with_capacity(small_rgba.len());
    for pixel in small_rgba.pixels() {
        bgra.push(pixel[2]);
        bgra.push(pixel[1]);
        bgra.push(pixel[0]);
        bgra.push(pixel[3]);
    }
    let accent = accent::extract(&bgra, 32, 32, 32 * 4);

    // 封面：等比缩到 256 以内再编码，避免把大图塞进 IPC
    let cover = image.thumbnail(256, 256);
    let mut png = Vec::new();
    let mut cursor = std::io::Cursor::new(&mut png);
    cover.write_to(&mut cursor, image::ImageFormat::Png).ok()?;

    let encoded = base64::engine::general_purpose::STANDARD.encode(&png);

    Some(CachedArtwork {
        url: format!("data:image/png;base64,{encoded}"),
        version: 0,
        primary: format!(
            "#{:02X}{:02X}{:02X}",
            accent.primary.0, accent.primary.1, accent.primary.2
        ),
        secondary: format!(
            "#{:02X}{:02X}{:02X}",
            accent.secondary.0, accent.secondary.1, accent.secondary.2
        ),
    })
}

fn display_name_for(app_user_model_id: &str) -> String {
    if app_user_model_id.trim().is_empty() {
        return String::new();
    }

    let mut value = app_user_model_id;
    if let Some(index) = value.find('!') {
        value = &value[..index];
    }
    if let Some(index) = value.rfind(['\\', '/']) {
        value = &value[index + 1..];
    }
    if value.to_ascii_lowercase().ends_with(".exe") {
        value = &value[..value.len() - 4];
    }

    let friendly = match value.to_ascii_lowercase().as_str() {
        "msedgewebview2" => Some("网页媒体"),
        "msedge" => Some("Microsoft Edge"),
        "chrome" => Some("Chrome"),
        "firefox" => Some("Firefox"),
        "spotify" => Some("Spotify"),
        "music.ui" | "zunemusic" => Some("Media Player"),
        "qqmusic" => Some("QQ 音乐"),
        "cloudmusic" => Some("网易云音乐"),
        "kugou" => Some("酷狗音乐"),
        "kwmusic" => Some("酷我音乐"),
        "potplayermini64" | "potplayermini" => Some("PotPlayer"),
        "vlc" => Some("VLC"),
        "tiktok" | "douyin" => Some("抖音"),
        "bilibili" => Some("哔哩哔哩"),
        _ => None,
    };

    friendly.unwrap_or(value).to_string()
}

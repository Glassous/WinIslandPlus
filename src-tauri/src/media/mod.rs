//! 基于系统 SMTC (`GlobalSystemMediaTransportControlsSessionManager`) 的媒体会话。
//!
//! 所有 WinRT 调用都跑在一条独占的 MTA 线程上：UI 线程只读已序列化的快照，
//! 媒体线程负责轮询、封面解码与取色，避免把 WinRT 对象跨线程带出去。

mod accent;

use std::collections::{HashMap, HashSet, VecDeque};
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
use crate::lyrics::LyricsService;

pub const EVENT_MEDIA_CHANGED: &str = "media://changed";
/// 轻量位置推送：只有数字，没有封面与文本，用于歌词 / 进度的实时对位。
pub const EVENT_MEDIA_POSITION: &str = "media://position";

/// 封面缓存上限。
const ARTWORK_CACHE_LIMIT: usize = 8;
/// 播放中的轮询间隔。
const POLL_PLAYING_MS: u64 = 350;
/// 暂停 / 无会话时的轮询间隔。
const POLL_IDLE_MS: u64 = 1000;
/// 控制指令后的高密度探测间隔。
const POLL_FAST_MS: u64 = 80;
/// 控制指令后的高密度探测窗口（毫秒）。
const COMMAND_FAST_WINDOW_MS: f64 = 1200.0;
/// 位置快照的推送阈值：解码后的位置与上次推送相差超过它就再推一次。
const POSITION_EMIT_DELTA_MS: f64 = 120.0;
/// 跳转指令发出后，等多久再核对播放器是否真的跳到了目标位置。
const SEEK_VERIFY_MS: f64 = 900.0;
/// 核对的容差：跳转后位置与目标差这么多以内就算成功（播放中还会继续前进）。
const SEEK_VERIFY_TOLERANCE_MS: f64 = 2_500.0;
/// 歌词请求发出后多久还没结果就重发一次（防请求丢失 / 歌词线程正忙）。
const LYRICS_RECHECK_MS: f64 = 20_000.0;

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
    /// 播放器是否接受外部跳转。
    ///
    /// 一旦实测到「跳转指令发出去了、位置却没动」（部分应用不实现
    /// `TryChangePlaybackPositionAsync`，或只在自身 UI 内生效），
    /// 就对这个来源应用记一笔，前端据此禁用进度条拖动。
    pub seekable: bool,
}

impl MediaSnapshot {
    fn track_key(&self) -> String {
        format!(
            "{}\u{1f}{}\u{1f}{}",
            self.title, self.artist, self.album_title
        )
    }
}

/// 位置推送载荷（与前端 `PositionPayload` 对应）。
#[derive(Serialize, Clone)]
#[serde(rename_all = "PascalCase")]
pub struct PositionPayload {
    pub position_ms: f64,
    pub position_stamp_ms: f64,
    pub duration_ms: f64,
    pub is_playing: bool,
}

enum MediaCommand {
    TogglePlayPause,
    Next,
    Previous,
    Seek(i64),
}

/// 待核对的跳转指令。
struct PendingSeek {
    target_ms: f64,
    /// 指令发出前的播放位置与采样时刻：用来判断「位置到底有没有为跳转动过」。
    before_ms: f64,
    before_stamp_ms: f64,
    was_playing: bool,
    /// 指令发出时的曲目键：换了歌就不作数。
    track_key: String,
    /// 来源应用标识（与 `unseekable` 的键一致）。
    app_key: String,
    deadline_ms: f64,
}

/// 「不接受外部跳转」的应用档案 + 待核对的跳转。
#[derive(Default)]
struct SeekTracker {
    /// 实测不支持外部跳转的来源应用。
    unseekable: HashSet<String>,
    pending: Option<PendingSeek>,
}

impl SeekTracker {
    fn is_seekable(&self, app_key: &str) -> bool {
        // 拿不到应用标识时不作限制，宁可允许拖动也不要凭空空禁
        app_key.is_empty() || !self.unseekable.contains(app_key)
    }

    /// 发出跳转指令：记下目标位置与发出前的基准位置。
    fn note_command(&mut self, context: Option<&MediaSnapshot>, app_key: &str, target_ms: f64) {
        if app_key.is_empty() {
            self.pending = None;
            return;
        }

        self.pending = Some(PendingSeek {
            target_ms,
            before_ms: context.map(|snapshot| snapshot.position_ms).unwrap_or(0.0),
            before_stamp_ms: context
                .map(|snapshot| snapshot.position_stamp_ms)
                .unwrap_or_else(now_ms),
            was_playing: context.map(|snapshot| snapshot.is_playing).unwrap_or(true),
            track_key: context.map(|snapshot| snapshot.track_key()).unwrap_or_default(),
            app_key: app_key.to_string(),
            deadline_ms: now_ms() + SEEK_VERIFY_MS,
        });
    }

    /// 播放器明确拒绝跳转（返回 false / 报错）：直接判定该应用不支持。
    fn reject(&mut self, app_key: &str) {
        self.pending = None;
        if app_key.is_empty() {
            return;
        }
        if self.unseekable.insert(app_key.to_string()) {
            log::warn("media: 播放器拒绝外部进度跳转，已禁用该应用的拖动");
        }
    }

    /// 核对上一次跳转是否真的生效；返回 true 表示刚刚判定为「不支持跳转」。
    fn verify(&mut self, snapshot: &MediaSnapshot) -> bool {
        let Some(pending) = self.pending.as_ref() else {
            return false;
        };

        // 已经换歌：这次跳转的结果无从判断，不作数
        if snapshot.track_key() != pending.track_key {
            self.pending = None;
            return false;
        }

        let drift = (snapshot.position_ms - pending.target_ms).abs();
        if drift <= SEEK_VERIFY_TOLERANCE_MS {
            // 位置已经到目标附近：跳转生效
            self.pending = None;
            return false;
        }

        if now_ms() < pending.deadline_ms {
            return false;
        }

        let pending = self.pending.take().expect("pending 已判空");

        // 不跳转时位置本该走到哪里：实际位置与它基本一致，才说明跳转真的没生效。
        // 位置既没到目标、又明显动过的（比如播放器改了跳转语义）一律不下结论，
        // 免得把「能拖但落点有偏差」的播放器误判成不支持。
        let elapsed = if pending.was_playing {
            (now_ms() - pending.before_stamp_ms).max(0.0)
        } else {
            0.0
        };
        let untouched = pending.before_ms + elapsed;
        if (snapshot.position_ms - untouched).abs() > SEEK_VERIFY_TOLERANCE_MS {
            return false;
        }
        // 目标本来就离「自然走到的位置」很近（只是微调几毫秒）：本身就没什么可验证的
        if (pending.target_ms - untouched).abs() <= SEEK_VERIFY_TOLERANCE_MS {
            return false;
        }

        if self.unseekable.insert(pending.app_key) {
            log::warn(&format!(
                "media: 跳转后位置未变化（目标偏差 {:.0}ms），已禁用该应用的拖动",
                drift
            ));
            return true;
        }
        false
    }
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
    pub fn start(&self, app: AppHandle, lyrics: LyricsService) {
        let (tx, rx) = channel::<MediaCommand>();
        if let Ok(mut guard) = self.inner.commands.lock() {
            *guard = Some(tx);
        }

        let current = self.inner.current.clone();
        std::thread::Builder::new()
            .name("winisland-media".into())
            .spawn(move || run(app, current, rx, lyrics))
            .ok();
    }
}

fn run(
    app: AppHandle,
    current: Arc<Mutex<Option<MediaSnapshot>>>,
    rx: Receiver<MediaCommand>,
    lyrics: LyricsService,
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
    // 控制指令后的高密度探测窗口：SMTC 状态传播有延迟，指令后的第一次探测常读到旧状态，
    // 若不密集追一段时间，暂停态要等满 1s 轮询周期前端才能见到新状态（按钮"延迟"的根因）。
    let mut fast_until = 0.0f64;
    // 最近一次请求过歌词的曲目键与请求时刻（换歌时重新请求）。
    let mut last_lyrics_key = String::new();
    let mut last_lyrics_at = 0.0f64;
    // 跳转能力档案：哪些应用不接受外部跳转、哪次跳转还等着核对。
    let mut seek = SeekTracker::default();
    // 最近一次探测到的快照（跳转核对要拿它当「发出指令时的曲目」）。
    let mut last_probe: Option<MediaSnapshot> = None;
    // 上一次推送的实时位置（用于给前端持续对位，只在变化足够大时才推）。
    let mut last_position_emit: Option<(f64, bool)> = None;

    loop {
        let playing = current
            .lock()
            .ok()
            .and_then(|guard| guard.as_ref().map(|s| s.is_playing))
            .unwrap_or(false);

        let interval_ms = if now_ms() < fast_until {
            POLL_FAST_MS
        } else if playing {
            POLL_PLAYING_MS
        } else {
            POLL_IDLE_MS
        };

        match rx.recv_timeout(Duration::from_millis(interval_ms)) {
            Ok(command) => {
                if manager.is_none() {
                    manager = request_manager(&mut manager_retry_at);
                }
                if let Some(manager) = manager.as_ref() {
                    apply_command(manager, command, &mut seek, last_probe.as_ref());
                    fast_until = now_ms() + COMMAND_FAST_WINDOW_MS;
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

        let mut snapshot = probe(manager, &mut cache, &seek);

        // 跳转核对：指令发出后位置迟迟没跟上来 → 这个应用不接受外部跳转
        if let Some(s) = snapshot.as_mut() {
            if seek.verify(s) {
                s.seekable = false;
            }
        }

        // 曲目变化（含首次出现媒体）时请求歌词；媒体消失则清空键，回来后重新请求。
        // 歌词服务自带缓存与去重，结果经 lyrics://changed 推送。
        match &snapshot {
            Some(s) => {
                let key = s.track_key();
                if key != last_lyrics_key {
                    last_lyrics_key = key;
                    last_lyrics_at = now_ms();
                    // 带上时长：歌词接口按「时长最接近」择优，避免选中现场版/翻唱条目
                    lyrics.request(&s.title, &s.artist, &s.album_title, s.duration_ms);
                } else if now_ms() - last_lyrics_at > LYRICS_RECHECK_MS
                    && lyrics.delivered_key().as_deref() != Some(last_lyrics_key.as_str())
                {
                    // 这么久了还没有任何结果回来：可能请求丢了 / 线程在忙，再要一次。
                    // 歌词服务按曲目键去重并带缓存，重复请求不会再打一遍网络。
                    last_lyrics_at = now_ms();
                    log::info("media: 歌词迟迟未到，重新请求");
                    lyrics.request(&s.title, &s.artist, &s.album_title, s.duration_ms);
                }
            }
            None => {
                last_lyrics_key.clear();
                last_lyrics_at = 0.0;
            }
        }

        let should_emit = match (&snapshot, &last_emitted) {
            (None, None) => false,
            (None, Some(_)) => true,
            (Some(new), Some(old)) => {
                new.track_key() != old.track_key()
                    || new.is_playing != old.is_playing
                    || new.artwork_version != old.artwork_version
                    || (new.duration_ms - old.duration_ms).abs() > 300.0
                    || new.source_app_name != old.source_app_name
                    || new.seekable != old.seekable
                    || (expected_position(old) - new.position_ms).abs() > 700.0
            }
            (Some(_), None) => true,
        };

        if let Ok(mut guard) = current.lock() {
            *guard = snapshot.clone();
        }

        if should_emit {
            let _ = app.emit(EVENT_MEDIA_CHANGED, snapshot.clone());
            last_emitted = snapshot.clone();
        }

        // 轻量位置推送：不含封面与文本，可以高频发出。
        // 前端用它给歌词 / 进度做实时对位——只靠整份快照的话，
        // 播放位置在两次快照之间只能靠本地时钟外推，累积偏差会把整首歌的歌词顶偏。
        if let Some(s) = snapshot.as_ref() {
            let changed = match last_position_emit {
                Some((position, playing)) => {
                    s.is_playing != playing
                        || (s.position_ms - position).abs() > POSITION_EMIT_DELTA_MS
                }
                None => true,
            };

            if changed {
                last_position_emit = Some((s.position_ms, s.is_playing));
                let _ = app.emit(
                    EVENT_MEDIA_POSITION,
                    PositionPayload {
                        position_ms: s.position_ms,
                        position_stamp_ms: s.position_stamp_ms,
                        duration_ms: s.duration_ms,
                        is_playing: s.is_playing,
                    },
                );
            }
        } else {
            last_position_emit = None;
        }

        last_probe = snapshot;
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

fn apply_command(
    manager: &Manager,
    command: MediaCommand,
    seek: &mut SeekTracker,
    latest: Option<&MediaSnapshot>,
) {
    let Some(session) = pick_session(manager) else {
        return;
    };

    let app_key = session
        .SourceAppUserModelId()
        .map(|id| id.to_string())
        .unwrap_or_default();
    let is_seek = matches!(command, MediaCommand::Seek(_));

    let result = match command {
        MediaCommand::TogglePlayPause => session.TryTogglePlayPauseAsync().and_then(wait).map(|_| ()),
        MediaCommand::Next => session.TrySkipNextAsync().and_then(wait).map(|_| ()),
        MediaCommand::Previous => session.TrySkipPreviousAsync().and_then(wait).map(|_| ()),
        // 跳转是唯一会「返回 false 但不算报错」的指令：播放器可以拒绝（超出范围 / 不支持跳转）。
        // 这类应用之后一律不再允许拖动，免得用户反复拖一条推不动的进度条。
        MediaCommand::Seek(ticks) => {
            let target_ms = ticks as f64 / 10_000.0;

            session
                .TryChangePlaybackPositionAsync(ticks)
                .and_then(wait)
                .map(|changed| {
                    if changed {
                        // 返回 true 不代表真的跳了：记下目标位置，下一次探测再核对
                        seek.note_command(latest, &app_key, target_ms);
                    } else {
                        log::warn(&format!("media: 播放器未接受进度跳转 ticks={ticks}"));
                        seek.reject(&app_key);
                    }
                })
        }
    };

    if let Err(err) = result {
        log::warn(&format!("media: 控制指令执行失败（{err}）"));
        if is_seek {
            seek.reject(&app_key);
        }
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

fn probe(manager: &Manager, cache: &mut ArtworkCache, seek: &SeekTracker) -> Option<MediaSnapshot> {
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

    let app_id = session
        .SourceAppUserModelId()
        .map(|id| id.to_string())
        .unwrap_or_default();
    let source_app = display_name_for(&app_id);

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
        seekable: seek.is_seekable(&app_id),
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

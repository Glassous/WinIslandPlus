import type { MediaSnapshot } from "./types";

/**
 * 曲目身份键：与后端媒体快照 / 歌词缓存同构。
 *
 * 后端用同一个键判断「是否换歌」「歌词属于哪首歌」，因此前端做歌词对齐、
 * 时钟重锚时也必须用这里的实现，不能自己拼字符串。
 */
export function trackKeyOf(
  snapshot: Pick<MediaSnapshot, "Title" | "Artist" | "AlbumTitle">,
): string {
  return `${snapshot.Title}\u001f${snapshot.Artist}\u001f${snapshot.AlbumTitle}`;
}

/** 按当前时刻推算播放位置（播放中线性推进，暂停时固定）。 */
export function positionAt(snapshot: MediaSnapshot, now: number = Date.now()): number {
  let position = snapshot.PositionMs;
  if (snapshot.IsPlaying) {
    position += now - snapshot.PositionStampMs;
  }
  if (position < 0) position = 0;
  if (snapshot.DurationMs > 0 && position > snapshot.DurationMs) {
    position = snapshot.DurationMs;
  }
  return position;
}

/** 毫秒 → `m:ss` 或 `h:mm:ss`。 */
export function formatTime(ms: number): string {
  let value = Math.max(0, Math.floor(ms / 1000));
  const hours = Math.floor(value / 3600);
  value -= hours * 3600;
  const minutes = Math.floor(value / 60);
  const seconds = value - minutes * 60;
  const pad = (n: number) => (n < 10 ? `0${n}` : `${n}`);
  return hours >= 1
    ? `${hours}:${pad(minutes)}:${pad(seconds)}`
    : `${minutes}:${pad(seconds)}`;
}

/** `-m:ss` 形式的剩余时间。 */
export function formatRemaining(positionMs: number, durationMs: number): string {
  if (durationMs <= 0) return "-0:00";
  return `-${formatTime(Math.max(0, durationMs - positionMs))}`;
}

export { positionAt as currentPosition };

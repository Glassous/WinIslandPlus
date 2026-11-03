/**
 * LRC 解析：把带时间轴的歌词文本变成「每句有起止时间 + 句内可插值」的时间轴模型。
 *
 * 相比只记录每句开始时间的极简模型，这里额外做到：
 * - 每句算出结束时间（下一句开始；最后一句按曲目时长兜底）→ 句内进度才有意义；
 * - 支持增强型 LRC 的 `<mm:ss.xx>` 词级标签 → 句内进度可以逐词推进；
 * - 过滤元数据标签、折叠连续空行（保留一个作为间奏标记）、按时间稳定排序。
 */

/** 一句里的词时间轴（增强型 LRC 的 `<mm:ss.xx>` 标签）。 */
export interface LyricWord {
  startMs: number;
  endMs: number;
  /** 该词在句内的「视觉宽度权重」，用来把时间进度换算成文本进度。 */
  weight: number;
}

/** 带时间轴的一句歌词。 */
export interface LyricLine {
  startMs: number;
  /** 该句结束时间：下一句开始，或最后一句按曲目时长收尾。 */
  endMs: number;
  /** 句内文本；空串表示间奏（界面上显示为音符）。 */
  text: string;
  /** 词级时间轴（仅增强型 LRC 才有）。 */
  words?: LyricWord[];
}

const TIME_TAG = /\[(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?\]/g;
const WORD_TAG = /<(\d{1,3}):(\d{1,2})(?:[.:](\d{1,3}))?>/g;
const OFFSET_TAG = /\[offset:\s*([+-]?\d+)\s*\]/i;
/**
 * 制作信息行：QQ 音乐 / 网易云的歌词开头常带一串 `词：方文山`、`制作人 : Steve Mac`、
 * `编曲：林迈可`。它们有时间轴却不是歌词——不滤掉的话，前奏十几秒里卡片与胶囊
 * 会一直显示这些信息，看起来就像「歌词对不上」。整行当作空句（间奏）处理。
 */
const CREDIT_LINE =
  /^\s*(作词|作曲|编曲|词曲|词|曲|制作人|监制|统筹|企划|策划|宣发|发行|出品|原唱|演唱|混音|母带|录音|配唱|和声|吉他|贝斯|鼓|键盘|弦乐|人声|音乐总监|制作统筹|混音师|母带师|录音师|OP|SP|Lyrics|Composed|Arranged|Produced|Written|Mixed|Mastered)\s*(?:by\s*)?[:：]/i;
/** 汉字 / 全角 / 假名 / 韩文按整宽算，其余按半宽，用来近似文本的实际渲染宽度。 */
const WIDE_CHAR = /[\u2e80-\u9fff\uac00-\ud7af\uf900-\ufaff\ufe30-\ufe4f\uff00-\uffef]/;

/** 曲目时长未知时，最后一句默认停留时长。 */
const HOLD_MS = 6_000;

/**
 * 解析 LRC 文本为按时间排序的歌词行。
 *
 * - 支持一行多个时间标签（`[a][b]同一句`），每个标签展开成独立行；
 * - 支持 `[offset:±ms]`：正值让歌词整体提前；
 * - `[ti:]` / `[ar:]` 等元数据标签不含时间戳，自然被跳过；
 * - 制作信息行（`词：`、`编曲：`…）按空句丢掉；
 * - 传了 `trackTitle` 时，还会丢掉开头那行「歌名 - 歌手」式的头部信息；
 * - `durationMs` 已知时用于兜底最后一句的结束时间。
 */
export function parseLrc(lrc: string, durationMs = 0, trackTitle = ""): LyricLine[] {
  const offsetMatch = OFFSET_TAG.exec(lrc);
  const offset = offsetMatch ? Number(offsetMatch[1]) : 0;

  const lines: LyricLine[] = [];

  for (const raw of lrc.split(/\r?\n/)) {
    const stamps: number[] = [];
    let lastIndex = 0;

    TIME_TAG.lastIndex = 0;
    let match: RegExpExecArray | null;
    while ((match = TIME_TAG.exec(raw)) !== null) {
      stamps.push(toMs(match[1], match[2], match[3]));
      lastIndex = TIME_TAG.lastIndex;
    }

    if (stamps.length === 0) continue;

    const { text, words } = parseWords(raw.slice(lastIndex), stamps[0]);
    // 制作信息（词：/ 曲：/ 制作人：…）不是歌词：按空句处理，交给下面的空行折叠吃掉
    const content = CREDIT_LINE.test(text) ? "" : text;
    const shiftedWords =
      words && stamps.length === 1
        ? words.map((word) => ({
            startMs: Math.max(0, word.startMs - offset),
            endMs: Math.max(0, word.endMs - offset),
            weight: word.weight,
          }))
        : undefined;

    for (const stamp of stamps) {
      const startMs = Math.max(0, stamp - offset);
      const line: LyricLine = { startMs, endMs: startMs, text: content };
      if (shiftedWords && content.length > 0) line.words = shiftedWords;
      lines.push(line);
    }
  }

  lines.sort((a, b) => a.startMs - b.startMs);

  // 折叠连续空行（保留一个作为间奏标记），并去掉首尾的空行
  const compact: LyricLine[] = [];
  for (const line of lines) {
    const blank = line.text.length === 0;
    const previous = compact[compact.length - 1];
    if (blank && (!previous || previous.text.length === 0)) continue;
    compact.push(line);
  }
  while (compact.length > 0 && compact[compact.length - 1].text.length === 0) {
    compact.pop();
  }
  if (compact.length === 0) return [];

  // QQ / 网易云的歌词常在 0 秒处放一行「歌名 - 歌手 (English)」：它不是歌词，去掉
  if (compact[0].text.length > 0 && isHeaderLine(compact[0].text, trackTitle)) {
    compact.shift();
    if (compact.length === 0) return [];
  }

  // 每句的结束时间 = 下一句的开始；最后一句用曲目时长兜底
  for (let index = 0; index < compact.length; index++) {
    const line = compact[index];
    const next = compact[index + 1];
    line.endMs = next ? next.startMs : lastLineEnd(line, durationMs);

    // 词时间轴的最后一个词收在句尾，避免句内进度提前跑满
    const words = line.words;
    if (words && words.length > 0) {
      words[words.length - 1].endMs = line.endMs;
    }
  }

  return compact;
}

/**
 * 曲目时长可能晚于歌词到达：时长一确定就把最后一句的结束时间拉伸过去。
 */
export function retimeLyrics(lines: LyricLine[], durationMs: number): void {
  const last = lines[lines.length - 1];
  if (!last || durationMs <= 0) return;

  last.endMs = lastLineEnd(last, durationMs);
  const words = last.words;
  if (words && words.length > 0) {
    words[words.length - 1].endMs = last.endMs;
  }
}

/** 给定播放位置，返回应当显示的歌词行下标；首句之前返回 -1。 */
export function activeLineIndex(lines: readonly LyricLine[], positionMs: number): number {
  let low = 0;
  let high = lines.length - 1;
  let found = -1;

  while (low <= high) {
    const middle = (low + high) >> 1;
    if (lines[middle].startMs <= positionMs) {
      found = middle;
      low = middle + 1;
    } else {
      high = middle - 1;
    }
  }

  return found;
}

/**
 * 句内进度（0~1）：当前句还没开始时为 0，唱完后为 1。
 * 有词级时间轴时按词推进（更接近逐字高亮），否则在整句时长内线性插值。
 */
export function lineProgress(line: LyricLine | undefined, positionMs: number): number {
  if (!line) return 0;

  const span = line.endMs - line.startMs;
  if (span <= 0) return 1;

  const ratio = (positionMs - line.startMs) / span;
  if (ratio <= 0) return 0;
  if (ratio >= 1) return 1;

  const words = line.words;
  if (!words || words.length === 0) return ratio;

  let total = 0;
  for (const word of words) total += word.weight;
  if (total <= 0) return ratio;

  let done = 0;
  for (const word of words) {
    if (positionMs >= word.endMs) {
      done += word.weight;
      continue;
    }
    if (positionMs <= word.startMs) break;

    const wordSpan = word.endMs - word.startMs;
    if (wordSpan > 0) {
      done += word.weight * ((positionMs - word.startMs) / wordSpan);
    }
    break;
  }

  return Math.min(1, Math.max(0, done / total));
}

/**
 * 本地歌词文件兜底：后端把原始字节按十六进制送过来，这里按 BOM / 编码嗅探解码。
 * 中文 LRC 常见 UTF-8（含 BOM）、UTF-16 与 GBK，依次尝试。
 */
export function decodeLyricsHex(hex: string): string {
  if (hex.length < 2 || hex.length % 2 !== 0) return "";

  const bytes = new Uint8Array(hex.length >> 1);
  for (let index = 0; index < bytes.length; index++) {
    const byte = Number.parseInt(hex.slice(index * 2, index * 2 + 2), 16);
    if (Number.isNaN(byte)) return "";
    bytes[index] = byte;
  }

  if (bytes.length >= 3 && bytes[0] === 0xef && bytes[1] === 0xbb && bytes[2] === 0xbf) {
    return decodeWith(bytes.subarray(3), "utf-8");
  }
  if (bytes.length >= 2 && bytes[0] === 0xff && bytes[1] === 0xfe) {
    return decodeWith(bytes.subarray(2), "utf-16le");
  }
  if (bytes.length >= 2 && bytes[0] === 0xfe && bytes[1] === 0xff) {
    return decodeWith(bytes.subarray(2), "utf-16be");
  }

  // 没有 BOM：先严格按 UTF-8 解，失败再按 GB18030（GBK 超集）解
  for (const encoding of ["utf-8", "gb18030"]) {
    try {
      return new TextDecoder(encoding, { fatal: true }).decode(bytes);
    } catch {
      // 换下一种编码
    }
  }

  return decodeWith(bytes, "utf-8");
}

function decodeWith(bytes: Uint8Array, encoding: string): string {
  try {
    return new TextDecoder(encoding, { fatal: true }).decode(bytes);
  } catch {
    return new TextDecoder("utf-8").decode(bytes);
  }
}

function toMs(minutes: string, seconds: string, fraction: string | undefined): number {
  const fractionRaw = fraction ?? "0";
  // 小数位归一到毫秒：2 位 = 百分之一秒，3 位 = 毫秒
  const millis = Number(fractionRaw) * 10 ** (3 - fractionRaw.length);
  return Number(minutes) * 60_000 + Number(seconds) * 1000 + millis;
}

/**
 * 拆出「文本 + 词时间轴」。
 * 没有词级标签时只返回文本；有标签时每段文本对应一个词的开始时间。
 */
function parseWords(rest: string, baseMs: number): { text: string; words?: LyricWord[] } {
  if (!rest.includes("<")) {
    return { text: rest.trim() };
  }

  const chunks: { startMs: number; text: string }[] = [];
  let cursor = 0;
  let current = baseMs;

  WORD_TAG.lastIndex = 0;
  let match: RegExpExecArray | null;
  while ((match = WORD_TAG.exec(rest)) !== null) {
    const chunk = rest.slice(cursor, match.index);
    if (chunk.length > 0) chunks.push({ startMs: current, text: chunk });
    current = toMs(match[1], match[2], match[3]);
    cursor = match.index + match[0].length;
  }

  if (chunks.length === 0) return { text: rest.trim() };

  const tail = rest.slice(cursor);
  if (tail.length > 0) chunks.push({ startMs: current, text: tail });

  const text = chunks
    .map((chunk) => chunk.text)
    .join("")
    .replace(/\s+/g, " ")
    .trim();

  if (text.length === 0) return { text: "" };

  const words: LyricWord[] = [];
  for (let index = 0; index < chunks.length; index++) {
    const chunk = chunks[index];
    const weight = textWeight(chunk.text);
    if (weight <= 0) continue;

    const next = chunks[index + 1];
    words.push({
      startMs: chunk.startMs,
      // 词的结束时间默认是下一个词的开始，最后一个词由句尾兜底
      endMs: next ? next.startMs : chunk.startMs + 1_000,
      weight,
    });
  }

  return words.length > 0 ? { text, words } : { text };
}

function textWeight(text: string): number {
  let weight = 0;
  for (const char of text) {
    weight += WIDE_CHAR.test(char) ? 1 : 0.55;
  }
  return weight;
}

/**
 * 是否是歌词开头的「头部信息行」：形如 `歌名 - 歌手`、`歌名 (English Title)`。
 *
 * 判定刻意收紧：必须以曲名开头，并且带破折号或括号修饰——
 * 普通歌词里不会出现这种形态，误杀风险很低。
 */
function isHeaderLine(text: string, trackTitle: string): boolean {
  const want = normalizeForCompare(trackTitle);
  if (want.length === 0) return false;

  const got = normalizeForCompare(text);
  if (!got.startsWith(want)) return false;

  return /[-–—(（\[]/.test(text) && got.length > want.length;
}

/** 比对用的归一：只留字母数字（含汉字）并转小写。 */
function normalizeForCompare(text: string): string {
  return Array.from(text.toLowerCase())
    .filter((char) => /[\p{L}\p{N}]/u.test(char))
    .join("");
}

function lastLineEnd(line: LyricLine, durationMs: number): number {
  if (durationMs > line.startMs) return durationMs;

  const words = line.words;
  if (words && words.length > 0) {
    const end = words[words.length - 1].endMs;
    if (end > line.startMs) return end;
  }

  return line.startMs + HOLD_MS;
}

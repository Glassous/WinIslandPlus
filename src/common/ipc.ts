import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import type {
  Bootstrap,
  IslandSettings,
  ShapeRect,
  UpdateStatus,
} from "./types";
import { EVENTS } from "./types";

/** 类型化的 IPC 封装：所有失败统一打日志后抛出，调用方自行决定降级方式。 */
async function call<T>(command: string, args?: Record<string, unknown>): Promise<T> {
  try {
    return await invoke<T>(command, args);
  } catch (error) {
    console.error(`[ipc] ${command} 失败`, error);
    throw error;
  }
}

/** 与后端保持一致的命令集合。 */
export const api = {
  bootstrap: () => call<Bootstrap>("bootstrap"),
  islandReady: () => call<void>("island_ready"),

  /**
   * 每帧把当前可见形状推给后端（坐标：相对窗口客户区的物理像素）。
   * 两个岛面最多各一块：命中判定为「任一矩形包含光标」。
   */
  setShapes: (shapes: ShapeRect[]) => call<void>("set_island_shapes", { shapes }),
  setExpanded: (expanded: boolean) => call<void>("set_expanded", { expanded }),
  /** 收起后通知后端复位形状（避免残留圆角判定）。 */
  setDragging: (dragging: boolean) => call<void>("set_dragging", { dragging }),
  /** 用户主动操作卡片（按钮 / 进度条）：2 秒内不因"暂停"自动收起。 */
  notifyUserActed: () => call<void>("notify_user_acted"),

  togglePlayPause: () => call<boolean>("media_toggle_play_pause"),
  next: () => call<boolean>("media_next"),
  previous: () => call<boolean>("media_previous"),
  seek: (positionMs: number) => call<boolean>("media_seek", { positionMs }),

  appInfo: () => call<{ Version: string; ExecutablePath: string }>("app_info"),

  getSettings: () => call<IslandSettings>("get_settings"),
  replaceSettings: (settings: IslandSettings) =>
    call<IslandSettings>("replace_settings", { settings }),
  getStartupEnabled: () => call<boolean>("get_startup_enabled"),
  setStartupEnabled: (enabled: boolean) =>
    call<boolean>("set_startup_enabled", { enabled }),

  openSettings: () => call<void>("open_settings_window"),
  saveSettings: (settings: IslandSettings) =>
    call<void>("save_settings", { settings }),
  cancelSettings: (settings: IslandSettings) =>
    call<void>("cancel_settings", { settings }),
  closeSettingsWindow: () => call<void>("close_settings_window"),
  /** 光标在窗口客户区内的位置（CSS px）与左键状态，用于进度条拖动轮询。 */
  cursorProbe: () =>
    call<{ X: number; Y: number; Down: boolean } | null>("cursor_probe"),

  toggleIslandVisible: () => call<boolean>("toggle_island_visible"),

  /** 当前更新检测结论；结果过期时后端会顺手发起一次后台检测。 */
  updateStatus: () => call<UpdateStatus>("update_status"),
  /** 用系统默认浏览器打开发布页。 */
  openReleasePage: () => call<boolean>("open_release_page"),

  quit: () => call<void>("quit_app"),
  log: (level: "info" | "warn" | "error", message: string) =>
    call<void>("frontend_log", { level, message }),
} as const;

/** 订阅后端事件，返回取消函数。 */
export function on<T>(event: string, handler: (payload: T) => void): UnlistenFn {
  let unlisten: UnlistenFn | undefined;
  let disposed = false;

  void listen<T>(event, (e) => handler(e.payload)).then((fn) => {
    if (disposed) fn();
    else unlisten = fn;
  });

  return () => {
    disposed = true;
    unlisten?.();
  };
}

export const events = EVENTS;

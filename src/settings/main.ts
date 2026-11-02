import { api, events, on } from "../common/ipc";
import type { IslandSettings, IslandSizePreset, PlacementPreset } from "../common/types";
import { ComboBox, Slider, ToggleSwitch } from "./controls";

const SIZE_OPTIONS = [
  { value: "Small", label: "小" },
  { value: "Medium", label: "中（默认）" },
  { value: "Large", label: "大" },
  { value: "Custom", label: "自定义" },
];

const PLACEMENT_OPTIONS = [
  { value: "TopCenter", label: "顶部居中（默认）" },
  { value: "TopLeft", label: "顶部左侧" },
  { value: "TopRight", label: "顶部右侧" },
  { value: "BottomCenter", label: "底部居中" },
  { value: "Custom", label: "自定义" },
];

const el = {
  customSizePanel: document.getElementById("customSizePanel")!,
  widthValue: document.getElementById("widthValue")!,
  idleCollapseDelayPanel: document.getElementById("idleCollapseDelayPanel")!,
  idleCollapseDelayValue: document.getElementById("idleCollapseDelayValue")!,
  hint: document.getElementById("hintText")!,
  version: document.getElementById("versionText")!,
  resetPosition: document.getElementById("resetPositionButton") as HTMLButtonElement,
  save: document.getElementById("saveButton") as HTMLButtonElement,
  cancel: document.getElementById("cancelButton") as HTMLButtonElement,
} as const;

let original: IslandSettings;
let working: IslandSettings;

/** 本窗口内用户是否显式改过落位预设（决定「取消」是否回滚位置）。 */
let placementTouched = false;

/** 即时预览的请求还没返回时，忽略自己触发的 settings://changed。 */
let syncing = false;

let sizeCombo: ComboBox;
let placementCombo: ComboBox;
let widthSlider: Slider;
let idleSlider: Slider;
let persistentSwitch: ToggleSwitch;
let autoCollapseSwitch: ToggleSwitch;
let idleCollapseSwitch: ToggleSwitch;
let autoStartSwitch: ToggleSwitch;

// ---------------- 预览 / 保存 / 取消 ----------------

async function preview(): Promise<void> {
  syncing = true;
  try {
    await api.replaceSettings(working);
    el.hint.textContent = "已即时预览";
  } catch (error) {
    el.hint.textContent = "预览失败，请查看日志";
    void api.log("error", `settings 预览失败: ${String(error)}`);
  } finally {
    syncing = false;
  }
}

/** 位置字段平时以运行时状态为准，避免保存时用打开窗口时的旧快照覆盖刚拖好的位置。 */
function copyPlacement(target: IslandSettings, source: IslandSettings): void {
  target.Placement = source.Placement;
  target.AnchorXRatio = source.AnchorXRatio;
  target.AnchorYRatio = source.AnchorYRatio;
  target.BottomAnchored = source.BottomAnchored;
  target.MonitorLeft = source.MonitorLeft;
  target.MonitorTop = source.MonitorTop;
  target.MonitorWidth = source.MonitorWidth;
  target.MonitorHeight = source.MonitorHeight;
}

async function mergeRuntimePlacement(): Promise<void> {
  if (placementTouched) return;
  try {
    const current = await api.getSettings();
    copyPlacement(working, current);
  } catch {
    // 读不到就沿用工作副本
  }
}

async function save(): Promise<void> {
  await mergeRuntimePlacement();
  try {
    await api.saveSettings(working);
  } catch (error) {
    el.hint.textContent = "保存失败，请查看日志";
    void api.log("error", `settings 保存失败: ${String(error)}`);
  }
}

async function cancel(): Promise<void> {
  const rollback = structuredClone(original);

  // 拖动岛体是直接操作，不属于"设置项"，取消时保留最新位置；
  // 但若本窗口里显式改过落位预设，则一并回滚。
  if (!placementTouched) {
    try {
      copyPlacement(rollback, await api.getSettings());
    } catch {
      // 忽略
    }
  }

  try {
    await api.cancelSettings(rollback);
  } catch (error) {
    void api.log("error", `settings 取消失败: ${String(error)}`);
  }
}

// ---------------- 控件装配 ----------------

function formatSize(width: number): string {
  const w = Math.round(width);
  // 整数除法（75 → 75 × 37）
  return `${w} × ${Math.floor(w / 2)}`;
}

function formatSeconds(seconds: number): string {
  return `${Number(seconds.toFixed(1))} 秒`;
}

function setCustomSizeVisible(visible: boolean): void {
  el.customSizePanel.hidden = !visible;
}

function setIdlePanelEnabled(enabled: boolean): void {
  el.idleCollapseDelayPanel.classList.toggle("disabled", !enabled);
}

function buildControls(settings: IslandSettings): void {
  sizeCombo = new ComboBox(document.getElementById("sizeCombo")!, SIZE_OPTIONS, settings.Size);
  sizeCombo.bindOnChange((value) => {
    working.Size = value as IslandSizePreset;
    setCustomSizeVisible(value === "Custom");
    void preview();
  });

  placementCombo = new ComboBox(
    document.getElementById("placementCombo")!,
    PLACEMENT_OPTIONS,
    settings.Placement,
  );
  placementCombo.bindOnChange((value) => {
    placementTouched = true;
    working.Placement = value as PlacementPreset;
    working.BottomAnchored = value === "BottomCenter";
    if (value === "TopCenter") {
      working.AnchorXRatio = 0.5;
      working.AnchorYRatio = 0;
    }
    void preview();
  });

  widthSlider = new Slider(document.getElementById("widthSlider")!, 60, 96, 2, settings.CustomPillWidth);
  widthSlider.bindOnChange((value) => {
    working.Size = "Custom";
    working.CustomPillWidth = value;
    working.CustomPillHeight = value / 2;
    el.widthValue.textContent = formatSize(value);
    if (sizeCombo.value !== "Custom") sizeCombo.value = "Custom";
    setCustomSizeVisible(true);
    void preview();
  });

  idleSlider = new Slider(
    document.getElementById("idleCollapseDelaySlider")!,
    1,
    15,
    0.5,
    settings.IdleCollapseSeconds,
  );
  idleSlider.bindOnChange((value) => {
    working.IdleCollapseSeconds = value;
    el.idleCollapseDelayValue.textContent = formatSeconds(value);
    void preview();
  });

  persistentSwitch = new ToggleSwitch(
    document.getElementById("persistentSwitch")!,
    settings.Persistent,
  );
  persistentSwitch.bindOnChange((value) => {
    working.Persistent = value;
    void preview();
  });

  autoCollapseSwitch = new ToggleSwitch(
    document.getElementById("autoCollapseSwitch")!,
    settings.AutoCollapseOnPause,
  );
  autoCollapseSwitch.bindOnChange((value) => {
    working.AutoCollapseOnPause = value;
    void preview();
  });

  idleCollapseSwitch = new ToggleSwitch(
    document.getElementById("idleCollapseSwitch")!,
    settings.IdleCollapse,
  );
  idleCollapseSwitch.bindOnChange((value) => {
    working.IdleCollapse = value;
    setIdlePanelEnabled(value);
    void preview();
  });

  autoStartSwitch = new ToggleSwitch(
    document.getElementById("autoStartSwitch")!,
    settings.AutoStart,
  );
  autoStartSwitch.bindOnChange((value) => {
    void (async () => {
      const ok = await api.setStartupEnabled(value);
      working.AutoStart = value;
      el.hint.textContent = ok ? "开机自启已更新" : "开机自启设置失败，请查看日志";
    })();
  });

  el.resetPosition.addEventListener("click", () => {
    placementTouched = true;
    working.Placement = "TopCenter";
    working.BottomAnchored = false;
    working.AnchorXRatio = 0.5;
    working.AnchorYRatio = 0;
    placementCombo.value = "TopCenter";
    void preview();
  });

  el.save.addEventListener("click", () => void save());
  el.cancel.addEventListener("click", () => void cancel());

  // 控件初值
  el.widthValue.textContent = formatSize(settings.CustomPillWidth);
  el.idleCollapseDelayValue.textContent = formatSeconds(settings.IdleCollapseSeconds);
  setCustomSizeVisible(settings.Size === "Custom");
  setIdlePanelEnabled(settings.IdleCollapse);
}

// ---------------- 启动 ----------------

async function init(): Promise<void> {
  const [settings, startupEnabled, info] = await Promise.all([
    api.getSettings(),
    api.getStartupEnabled(),
    api.appInfo(),
  ]);

  original = structuredClone(settings);
  working = structuredClone(settings);
  // 自启状态以注册表为准
  working.AutoStart = startupEnabled;
  settings.AutoStart = startupEnabled;

  buildControls(settings);

  el.version.textContent = `版本 ${info.Version}  ·  ${info.ExecutablePath}`;

  // 岛体拖动会直接写回配置里的位置字段：持续把最新位置合并进工作副本
  on<IslandSettings>(events.settingsChanged, (current) => {
    if (syncing) return;

    copyPlacement(working, current);
    placementTouched = false;

    if (current.Placement === "Custom") {
      placementCombo.value = "Custom";
    }
  });
}

void init().catch((error) => {
  el.hint.textContent = "初始化失败，请查看日志";
  void api.log("error", `settings 初始化失败: ${String(error)}`);
});

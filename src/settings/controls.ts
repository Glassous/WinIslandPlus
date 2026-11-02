/** 自绘 Fluent 控件：ComboBox / ToggleSwitch / Slider（行为对齐 Fluent 默认控件）。 */

export interface ComboOption {
  value: string;
  label: string;
}

const CHEVRON = "\uE70D";
const CHECK = "\uE73E";

export class ComboBox {
  private readonly button: HTMLButtonElement;
  private readonly valueEl: HTMLElement;
  private readonly popup: HTMLElement;
  private readonly options: ComboOption[];
  private selected: string;
  private onChange: ((value: string) => void) | null = null;
  private open = false;
  private readonly onDocumentPointerDown: (event: PointerEvent) => void;

  constructor(root: HTMLElement, options: ComboOption[], selected: string) {
    this.options = options;
    this.selected = selected;

    root.innerHTML = "";
    root.classList.add("combo");

    this.button = document.createElement("button");
    this.button.type = "button";
    this.button.className = "combo-button";

    this.valueEl = document.createElement("span");
    this.valueEl.className = "combo-value";

    const chevron = document.createElement("span");
    chevron.className = "combo-chevron";
    chevron.textContent = CHEVRON;

    this.button.append(this.valueEl, chevron);

    this.popup = document.createElement("div");
    this.popup.className = "combo-popup";
    this.popup.hidden = true;

    for (const option of this.options) {
      const item = document.createElement("div");
      item.className = "combo-item";
      item.dataset.value = option.value;

      const label = document.createElement("span");
      label.textContent = option.label;

      const check = document.createElement("span");
      check.className = "check";
      check.textContent = CHECK;

      item.append(label, check);
      item.addEventListener("click", () => {
        this.close();
        this.select(option.value, true);
      });

      this.popup.appendChild(item);
    }

    root.append(this.button, this.popup);

    this.button.addEventListener("click", (event) => {
      event.stopPropagation();
      this.toggle();
    });

    this.onDocumentPointerDown = (event) => {
      if (!this.open) return;
      if (!root.contains(event.target as Node)) this.close();
    };
    document.addEventListener("pointerdown", this.onDocumentPointerDown);

    this.render();
  }

  get value(): string {
    return this.selected;
  }

  set value(value: string) {
    this.select(value, false);
  }

  setEnabled(enabled: boolean): void {
    this.button.disabled = !enabled;
  }

  bindOnChange(handler: (value: string) => void): void {
    this.onChange = handler;
  }

  dispose(): void {
    document.removeEventListener("pointerdown", this.onDocumentPointerDown);
  }

  private toggle(): void {
    if (this.open) this.close();
    else this.openPopup();
  }

  private openPopup(): void {
    this.open = true;
    this.popup.hidden = false;
  }

  private close(): void {
    this.open = false;
    this.popup.hidden = true;
  }

  private select(value: string, notify: boolean): void {
    if (!this.options.some((option) => option.value === value)) return;
    this.selected = value;
    this.render();
    if (notify) this.onChange?.(value);
  }

  private render(): void {
    const option = this.options.find((item) => item.value === this.selected);
    this.valueEl.textContent = option?.label ?? "";

    for (const item of Array.from(this.popup.children) as HTMLElement[]) {
      item.classList.toggle("selected", item.dataset.value === this.selected);
    }
  }
}

export class ToggleSwitch {
  private readonly root: HTMLElement;
  private checkedValue: boolean;
  private onChange: ((value: boolean) => void) | null = null;

  constructor(root: HTMLElement, checked: boolean) {
    this.root = root;
    this.checkedValue = checked;
    this.render();

    root.addEventListener("click", () => this.toggle());
    root.addEventListener("keydown", (event) => {
      if (event.key === " " || event.key === "Enter") {
        event.preventDefault();
        this.toggle();
      }
    });
  }

  get checked(): boolean {
    return this.checkedValue;
  }

  set checked(value: boolean) {
    this.checkedValue = value;
    this.render();
  }

  bindOnChange(handler: (value: boolean) => void): void {
    this.onChange = handler;
  }

  private toggle(): void {
    this.checkedValue = !this.checkedValue;
    this.render();
    this.onChange?.(this.checkedValue);
  }

  private render(): void {
    this.root.classList.toggle("on", this.checkedValue);
    this.root.setAttribute("aria-checked", this.checkedValue ? "true" : "false");
    this.root.tabIndex = 0;
  }
}

export class Slider {
  private readonly root: HTMLElement;
  private readonly rail: HTMLElement;
  private readonly fill: HTMLElement;
  private readonly thumb: HTMLElement;
  private readonly min: number;
  private readonly max: number;
  private readonly step: number;
  private current: number;
  private onChange: ((value: number) => void) | null = null;

  constructor(root: HTMLElement, min: number, max: number, step: number, value: number) {
    this.root = root;
    this.min = min;
    this.max = max;
    this.step = step;
    this.current = this.quantize(value);

    root.innerHTML = "";
    root.classList.add("slider");

    this.rail = document.createElement("div");
    this.rail.className = "slider-rail";

    this.fill = document.createElement("div");
    this.fill.className = "slider-fill";

    this.thumb = document.createElement("div");
    this.thumb.className = "slider-thumb";

    this.rail.append(this.fill, this.thumb);
    root.appendChild(this.rail);

    root.addEventListener("pointerdown", this.onPointerDown);
    root.addEventListener("pointermove", this.onPointerMove);
    root.addEventListener("pointerup", this.onPointerUp);
    root.addEventListener("pointercancel", this.onPointerUp);

    this.render();
  }

  get value(): number {
    return this.current;
  }

  set value(value: number) {
    this.current = this.quantize(value);
    this.render();
  }

  bindOnChange(handler: (value: number) => void): void {
    this.onChange = handler;
  }

  private onPointerDown = (event: PointerEvent): void => {
    if (event.button !== 0) return;
    this.root.setPointerCapture(event.pointerId);
    this.root.classList.add("dragging");
    this.applyFromClientX(event.clientX, true);
  };

  private onPointerMove = (event: PointerEvent): void => {
    if (!this.root.classList.contains("dragging")) return;
    this.applyFromClientX(event.clientX, true);
  };

  private onPointerUp = (event: PointerEvent): void => {
    if (this.root.hasPointerCapture(event.pointerId)) {
      this.root.releasePointerCapture(event.pointerId);
    }
    this.root.classList.remove("dragging");
  };

  private applyFromClientX(clientX: number, notify: boolean): void {
    const rect = this.rail.getBoundingClientRect();
    if (rect.width <= 0) return;

    const ratio = Math.min(1, Math.max(0, (clientX - rect.left) / rect.width));
    const next = this.quantize(this.min + ratio * (this.max - this.min));
    if (next === this.current) return;

    this.current = next;
    this.render();
    if (notify) this.onChange?.(this.current);
  }

  private quantize(value: number): number {
    if (!Number.isFinite(value)) return this.min;
    const steps = Math.round((value - this.min) / this.step);
    const quantized = this.min + steps * this.step;
    return Math.min(this.max, Math.max(this.min, Number(quantized.toFixed(4))));
  }

  private render(): void {
    const ratio = (this.current - this.min) / (this.max - this.min || 1);
    const percent = Math.min(100, Math.max(0, ratio * 100));
    this.fill.style.width = `${percent}%`;
    this.thumb.style.left = `${percent}%`;
    this.root.setAttribute("aria-valuenow", String(this.current));
  }
}

import { SlidersHorizontal, Sparkles } from "lucide-react";
import type { KeyboardEvent, ReactNode } from "react";
import { useTranslation } from "react-i18next";
import type { UiMode } from "../lib/uiMode";
import { writeUiMode } from "../lib/uiMode";

const MODE_ICON_PX = 14;

export function UiModeSwitch({
  mode,
  onChange,
}: {
  mode: UiMode;
  onChange: (mode: UiMode) => void;
}) {
  const { t, i18n } = useTranslation();
  const rtl = i18n.dir() === "rtl";

  function select(next: UiMode) {
    if (next === mode) return;
    writeUiMode(next);
    onChange(next);
  }

  function onKeyDown(event: KeyboardEvent<HTMLDivElement>) {
    const forward = rtl
      ? event.key === "ArrowLeft"
      : event.key === "ArrowRight";
    const backward = rtl
      ? event.key === "ArrowRight"
      : event.key === "ArrowLeft";
    if (forward) {
      event.preventDefault();
      select("advanced");
    } else if (backward) {
      event.preventDefault();
      select("basic");
    }
  }

  return (
    <div
      role="radiogroup"
      aria-label={t("uiModeLabel")}
      className="relative inline-grid w-full max-w-xs grid-cols-2 rounded-[5px] border border-[rgb(var(--border-default))] bg-canvas p-0.5"
      onKeyDown={onKeyDown}
    >
      <div
        aria-hidden
        className="pointer-events-none absolute inset-y-0.5 w-[calc(50%-0.125rem)] rounded-[5px] bg-surface transition-[inset-inline-start] duration-150 motion-reduce:transition-none"
        style={{
          insetInlineStart:
            mode === "basic" ? "0.125rem" : "calc(50% + 0.0625rem)",
        }}
      />
      <ModeOption
        icon={<Sparkles size={MODE_ICON_PX} aria-hidden />}
        label={t("uiModeBasic")}
        checked={mode === "basic"}
        onSelect={() => select("basic")}
      />
      <ModeOption
        icon={<SlidersHorizontal size={MODE_ICON_PX} aria-hidden />}
        label={t("uiModeAdvanced")}
        checked={mode === "advanced"}
        onSelect={() => select("advanced")}
      />
    </div>
  );
}

function ModeOption({
  icon,
  label,
  checked,
  onSelect,
}: {
  icon: ReactNode;
  label: string;
  checked: boolean;
  onSelect: () => void;
}) {
  return (
    <button
      type="button"
      role="radio"
      aria-checked={checked}
      tabIndex={checked ? 0 : -1}
      onClick={onSelect}
      className={`relative z-10 inline-flex h-7 items-center justify-center gap-1 rounded-[5px] px-2 text-[10px] font-semibold transition-colors ${
        checked ? "text-brand" : "text-muted hover:text-ink"
      }`}
    >
      {icon}
      {label}
    </button>
  );
}

import { ClipboardPaste, Copy, Scissors, TextSelect } from "lucide-react";
import { useEffect, useState, type ReactNode } from "react";
import { useTranslation } from "react-i18next";
import { readClipboardText, writeClipboardText } from "../lib/clipboard";
import { isEditableTarget, selectionLength } from "../lib/editableTarget";
import { BUTTON_ICON_PX } from "./AppButton";

interface MenuState {
  x: number;
  y: number;
  field: HTMLInputElement | HTMLTextAreaElement;
}

export function InputContextMenu() {
  const { t } = useTranslation();
  const [menu, setMenu] = useState<MenuState | null>(null);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    const onContextMenu = (event: MouseEvent) => {
      if (!isEditableTarget(event.target)) {
        setMenu(null);
        setError(null);
        return;
      }
      event.preventDefault();
      event.target.focus();
      setError(null);
      // Clamp so the menu never opens outside the viewport when the user
      // right-clicks near the bottom or the trailing edge.
      const menuWidth = 176;
      const menuHeight = 168;
      setMenu({
        x: Math.max(
          8,
          Math.min(event.clientX, window.innerWidth - menuWidth - 8),
        ),
        y: Math.max(
          8,
          Math.min(event.clientY, window.innerHeight - menuHeight - 8),
        ),
        field: event.target,
      });
    };
    const dismiss = () => {
      setMenu(null);
      setError(null);
    };
    document.addEventListener("contextmenu", onContextMenu);
    document.addEventListener("click", dismiss);
    window.addEventListener("blur", dismiss);
    window.addEventListener("resize", dismiss);
    return () => {
      document.removeEventListener("contextmenu", onContextMenu);
      document.removeEventListener("click", dismiss);
      window.removeEventListener("blur", dismiss);
      window.removeEventListener("resize", dismiss);
    };
  }, []);

  if (!menu) {
    return null;
  }

  const readonly = menu.field.readOnly || menu.field.disabled;
  const hasSelection = selectionLength(menu.field) > 0;

  return (
    <ul
      role="menu"
      data-testid="input-context-menu"
      className="fixed z-[80] min-w-40 rounded-lg border border-ink/10 bg-surface py-1 text-sm shadow-xl"
      style={{ left: menu.x, top: menu.y }}
      onClick={(event) => event.stopPropagation()}
    >
      <MenuItem
        icon={<TextSelect size={BUTTON_ICON_PX} aria-hidden />}
        label={t("contextSelectAll")}
        disabled={menu.field.disabled}
        onSelect={() => {
          menu.field.focus();
          menu.field.select();
        }}
      />
      <MenuItem
        icon={<Copy size={BUTTON_ICON_PX} aria-hidden />}
        label={t("contextCopy")}
        disabled={!hasSelection}
        onSelect={() => {
          void copySelection(menu.field).catch(() =>
            setError(t("contextClipboardFailed")),
          );
        }}
      />
      <MenuItem
        icon={<Scissors size={BUTTON_ICON_PX} aria-hidden />}
        label={t("contextCut")}
        disabled={readonly || !hasSelection}
        onSelect={() => {
          void cutSelection(menu.field).catch(() =>
            setError(t("contextClipboardFailed")),
          );
        }}
      />
      <MenuItem
        icon={<ClipboardPaste size={BUTTON_ICON_PX} aria-hidden />}
        label={t("contextPaste")}
        disabled={readonly}
        onSelect={() => {
          void pasteInto(menu.field).catch(() =>
            setError(t("contextClipboardFailed")),
          );
        }}
      />
      {error ? (
        <li className="px-3 py-1.5 text-xs text-danger" role="alert">
          {error}
        </li>
      ) : null}
    </ul>
  );
}

function MenuItem({
  icon,
  label,
  disabled,
  onSelect,
}: {
  icon: ReactNode;
  label: string;
  disabled: boolean;
  onSelect: () => void;
}) {
  return (
    <li role="none">
      <button
        type="button"
        role="menuitem"
        disabled={disabled}
        className="inline-flex w-full items-center gap-2 px-3 py-1.5 text-start disabled:text-muted"
        onClick={onSelect}
      >
        {icon}
        {label}
      </button>
    </li>
  );
}

async function copySelection(
  field: HTMLInputElement | HTMLTextAreaElement,
): Promise<void> {
  const text = field.value.slice(
    field.selectionStart ?? 0,
    field.selectionEnd ?? 0,
  );
  if (text) {
    await writeClipboardText(text);
  }
}

async function cutSelection(
  field: HTMLInputElement | HTMLTextAreaElement,
): Promise<void> {
  await copySelection(field);
  const start = field.selectionStart ?? 0;
  const end = field.selectionEnd ?? 0;
  replaceRange(field, start, end, "");
}

async function pasteInto(
  field: HTMLInputElement | HTMLTextAreaElement,
): Promise<void> {
  const target = field;
  if (!fieldIsEditable(target)) {
    return;
  }
  const start = target.selectionStart ?? target.value.length;
  const end = target.selectionEnd ?? target.value.length;
  const text = await readClipboardText();
  if (!fieldIsEditable(target)) {
    return;
  }
  replaceRange(target, start, end, text);
}

function fieldIsEditable(
  field: HTMLInputElement | HTMLTextAreaElement,
): boolean {
  return field.isConnected && !field.disabled && !field.readOnly;
}

function nativeValueSetter(
  field: HTMLInputElement | HTMLTextAreaElement,
): ((value: string) => void) | undefined {
  const prototype =
    field instanceof HTMLTextAreaElement
      ? HTMLTextAreaElement.prototype
      : HTMLInputElement.prototype;
  const descriptor = Object.getOwnPropertyDescriptor(prototype, "value");
  return descriptor?.set
    ? (value: string) => descriptor.set?.call(field, value)
    : undefined;
}

function replaceRange(
  field: HTMLInputElement | HTMLTextAreaElement,
  start: number,
  end: number,
  insert: string,
): void {
  const next = field.value.slice(0, start) + insert + field.value.slice(end);
  const setValue = nativeValueSetter(field);
  if (setValue) {
    setValue(next);
  } else {
    field.value = next;
  }
  field.dispatchEvent(
    new InputEvent("input", {
      bubbles: true,
      inputType: insert ? "insertFromPaste" : "deleteContentBackward",
    }),
  );
  const caret = start + insert.length;
  field.setSelectionRange(caret, caret);
}

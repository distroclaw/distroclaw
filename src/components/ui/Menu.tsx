import { KeyboardEvent, ReactNode, RefObject, useEffect, useRef } from "react";

export interface MenuItem {
  id: string;
  label: ReactNode;
  disabled?: boolean;
  onSelect: () => void;
  /** Leaves the menu open after selecting (for switching to a sub-list). */
  keepOpen?: boolean;
  /** Makes the item one of a set of choices (a radio item); the checked one gets focus when the menu opens. */
  checked?: boolean;
}

const ITEM = "[role^=menuitem]:not(:disabled)";

interface Props {
  /** Heading shown above the items. */
  label?: string;
  items: MenuItem[];
  /** Shown instead of items when there are none. */
  empty?: ReactNode;
  /** Muted note under the items. */
  hint?: ReactNode;
  ariaLabel: string;
  onClose: () => void;
  /** Clicks inside this element (the opener) do not count as outside clicks, so a toggle button works. */
  anchorRef?: RefObject<HTMLElement | null>;
}

/** A small popover menu: outside click and Escape close it, arrow keys move between items. */
export function Menu({ label, items, empty, hint, ariaLabel, onClose, anchorRef }: Props) {
  const root = useRef<HTMLDivElement>(null);
  const closeRef = useRef(onClose);
  const anchor = useRef(anchorRef);
  useEffect(() => {
    closeRef.current = onClose;
    anchor.current = anchorRef;
  }, [onClose, anchorRef]);

  useEffect(() => {
    const opener = document.activeElement as HTMLElement | null;
    const el = root.current;
    (el?.querySelector<HTMLButtonElement>(`${ITEM}[aria-checked="true"]`) ?? el?.querySelector<HTMLButtonElement>(ITEM))?.focus();
    const onDown = (e: MouseEvent) => {
      const target = e.target as Node;
      if (anchor.current?.current?.contains(target)) return;
      if (root.current && !root.current.contains(target)) closeRef.current();
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (e.key === "Escape") {
        e.stopPropagation();
        closeRef.current();
      }
    };
    document.addEventListener("mousedown", onDown);
    document.addEventListener("keydown", onKey, true);
    return () => {
      document.removeEventListener("mousedown", onDown);
      document.removeEventListener("keydown", onKey, true);
      if (opener && opener.isConnected) opener.focus();
    };
  }, []);

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    const buttons = Array.from(root.current?.querySelectorAll<HTMLButtonElement>(ITEM) ?? []);
    if (buttons.length === 0) return;
    const i = buttons.indexOf(document.activeElement as HTMLButtonElement);
    let next: number | null = null;
    if (e.key === "ArrowDown") next = i < 0 ? 0 : (i + 1) % buttons.length;
    else if (e.key === "ArrowUp") next = i < 0 ? buttons.length - 1 : (i - 1 + buttons.length) % buttons.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = buttons.length - 1;
    if (next === null) return;
    e.preventDefault();
    buttons[next].focus();
  };

  return (
    <div
      ref={root}
      className="menu"
      role="menu"
      aria-label={ariaLabel}
      onKeyDown={onKeyDown}
      onClick={(e) => e.stopPropagation()}
    >
      {label && <div className="menu-label">{label}</div>}
      {items.map((item) => (
        <button
          key={item.id}
          type="button"
          role={item.checked === undefined ? "menuitem" : "menuitemradio"}
          aria-checked={item.checked}
          disabled={item.disabled}
          onClick={() => {
            item.onSelect();
            if (!item.keepOpen) closeRef.current();
          }}
        >
          {item.label}
        </button>
      ))}
      {items.length === 0 && empty && <div className="menu-empty">{empty}</div>}
      {hint && <div className="menu-empty">{hint}</div>}
    </div>
  );
}

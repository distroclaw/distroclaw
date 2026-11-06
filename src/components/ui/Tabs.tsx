import { KeyboardEvent, ReactNode, useRef } from "react";

export interface TabItem {
  id: string;
  label: ReactNode;
}

interface TabsProps {
  tabs: TabItem[];
  value: string;
  onChange: (id: string) => void;
  /** Accessible name for the tab list. */
  label: string;
}

const tabId = (id: string) => `tab-${id}`;
const panelId = (id: string) => `tabpanel-${id}`;

/** A tab list drawn like the segmented control; arrow keys move between tabs. */
export function Tabs({ tabs, value, onChange, label }: TabsProps) {
  const list = useRef<HTMLDivElement>(null);

  const focusTab = (id: string) => {
    onChange(id);
    list.current?.querySelector<HTMLButtonElement>(`#${tabId(id)}`)?.focus();
  };

  const onKey = (e: KeyboardEvent<HTMLDivElement>) => {
    const i = tabs.findIndex((t) => t.id === value);
    if (i < 0 || tabs.length === 0) return;
    let next: number | null = null;
    if (e.key === "ArrowRight") next = (i + 1) % tabs.length;
    else if (e.key === "ArrowLeft") next = (i - 1 + tabs.length) % tabs.length;
    else if (e.key === "Home") next = 0;
    else if (e.key === "End") next = tabs.length - 1;
    if (next === null) return;
    e.preventDefault();
    focusTab(tabs[next].id);
  };

  return (
    <div ref={list} className="segmented tabs" role="tablist" aria-label={label} onKeyDown={onKey}>
      {tabs.map((t) => {
        const selected = t.id === value;
        return (
          <button
            key={t.id}
            id={tabId(t.id)}
            type="button"
            role="tab"
            aria-selected={selected}
            aria-controls={panelId(t.id)}
            tabIndex={selected ? 0 : -1}
            className={selected ? "on" : ""}
            onClick={() => onChange(t.id)}
          >
            {t.label}
          </button>
        );
      })}
    </div>
  );
}

interface TabPanelProps {
  id: string;
  active: boolean;
  children: ReactNode;
}

/** Renders its children only while active; keep shared state in the parent. */
export function TabPanel({ id, active, children }: TabPanelProps) {
  if (!active) return null;
  return (
    <div role="tabpanel" id={panelId(id)} aria-labelledby={tabId(id)}>
      {children}
    </div>
  );
}

import { createContext, ReactNode, useCallback, useContext, useEffect, useMemo, useRef, useState } from "react";
import { XIcon } from "../Icons";

type ToastKind = "info" | "error";

interface ToastItem {
  id: number;
  message: string;
  kind: ToastKind;
}

export type ToastFn = (message: string, options?: { kind?: ToastKind }) => void;

const ToastContext = createContext<ToastFn | null>(null);

const DISMISS_MS = 5000;

export function ToastProvider({ children }: { children: ReactNode }) {
  const [items, setItems] = useState<ToastItem[]>([]);
  const nextId = useRef(1);
  const timers = useRef(new Map<number, number>());

  const dismiss = useCallback((id: number) => {
    const timer = timers.current.get(id);
    if (timer !== undefined) window.clearTimeout(timer);
    timers.current.delete(id);
    setItems((xs) => xs.filter((x) => x.id !== id));
  }, []);

  const toast = useCallback<ToastFn>(
    (message, { kind = "info" } = {}) => {
      const id = nextId.current++;
      setItems((xs) => [...xs, { id, message, kind }]);
      timers.current.set(id, window.setTimeout(() => dismiss(id), DISMISS_MS));
    },
    [dismiss],
  );

  useEffect(() => {
    const pending = timers.current;
    return () => {
      for (const timer of pending.values()) window.clearTimeout(timer);
      pending.clear();
    };
  }, []);

  const value = useMemo(() => toast, [toast]);

  return (
    <ToastContext.Provider value={value}>
      {children}
      <div className="toasts" role="status" aria-live="polite">
        {items.map((t) => (
          <div key={t.id} className={`toast ${t.kind === "error" ? "error" : ""}`} role={t.kind === "error" ? "alert" : undefined}>
            <span>{t.message}</span>
            <button className="icon-btn" aria-label="Dismiss" onClick={() => dismiss(t.id)}>
              <XIcon />
            </button>
          </div>
        ))}
      </div>
    </ToastContext.Provider>
  );
}

const fallback: ToastFn = (message) => console.warn(message);

export function useToast(): ToastFn {
  return useContext(ToastContext) ?? fallback;
}

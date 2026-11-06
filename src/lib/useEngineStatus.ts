import { useCallback, useEffect, useRef, useState } from "react";
import { api, EngineStatus } from "./api";

/** Poll quickly while the engine starts, then slowly forever so a crash is noticed. */
const FAST_MS = 400;
const SLOW_MS = 5000;

export function useEngineStatus(enabled: boolean) {
  const [status, setStatus] = useState<EngineStatus | null>(null);
  const [everReady, setEverReady] = useState(false);
  const everReadyRef = useRef(false);
  const pollRef = useRef<() => void>(() => {});
  /** Bumped by `apply`, so a poll that started before it cannot bring back the older status. */
  const generation = useRef(0);

  const accept = useCallback((s: EngineStatus) => {
    setStatus(s);
    if (s.state === "ready" && !everReadyRef.current) {
      everReadyRef.current = true;
      setEverReady(true);
    }
  }, []);

  useEffect(() => {
    if (!enabled) return;
    let alive = true;
    let inFlight = false;
    let timer: number | undefined;

    const schedule = () => {
      window.clearTimeout(timer);
      timer = window.setTimeout(poll, everReadyRef.current ? SLOW_MS : FAST_MS);
    };

    const poll = async () => {
      if (!alive || inFlight) return;
      inFlight = true;
      const started = generation.current;
      try {
        const s = await api.engineStatus();
        if (!alive || started !== generation.current) return;
        accept(s);
      } catch {
        /* keep polling */
      } finally {
        inFlight = false;
        if (alive) schedule();
      }
    };

    pollRef.current = () => {
      poll();
    };
    poll();

    const wake = () => {
      if (document.visibilityState !== "hidden") poll();
    };
    window.addEventListener("focus", wake);
    document.addEventListener("visibilitychange", wake);
    return () => {
      alive = false;
      window.clearTimeout(timer);
      window.removeEventListener("focus", wake);
      document.removeEventListener("visibilitychange", wake);
      pollRef.current = () => {};
    };
  }, [enabled, accept]);

  /** Shows a status a command just returned (e.g. after switching models) without waiting for the next poll. */
  const apply = useCallback(
    (s: EngineStatus) => {
      generation.current += 1;
      accept(s);
    },
    [accept],
  );

  const retry = useCallback(async () => {
    try {
      await api.engineRetry();
    } catch {
      /* the next poll reports the state */
    }
    pollRef.current();
  }, []);

  return { status, everReady, retry, apply };
}

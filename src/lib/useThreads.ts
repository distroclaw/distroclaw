import { useCallback, useEffect, useRef, useState } from "react";
import { api, Attachment, ChatEvent, errorText, ExtEvent, Message, normalizeMessage, ResearchStatus, SendOptions, Source } from "./api";

/** Events shown inside a reply while it streams (deltas are folded into content/thinking). */
export type LiveEvent = Extract<ChatEvent, { type: "skill" | "toolCall" | "toolResult" }>;

export interface LiveReply {
  content: string;
  thinking: string;
  events: LiveEvent[];
  /** Online search is running; cleared by its sources or the first streamed delta. */
  searching: boolean;
  /** Pages found by online search, cited in the reply as `[n]`. */
  sources: Source[];
  /** Events the core does not handle, for the extension's message addon. */
  extEvents: ExtEvent[];
  /** Olymp's latest progress while it researches; cleared once the answer starts. */
  research: ResearchStatus | null;
}

const emptyReply = (): LiveReply => ({
  content: "",
  thinking: "",
  events: [],
  searching: false,
  sources: [],
  extEvents: [],
  research: null,
});

interface DeltaBuffer {
  content: string;
  thinking: string;
  raf: number | null;
}

/**
 * Loaded message threads and in-flight replies, keyed by conversation id.
 * Each send closes over its own id, so switching chats while a reply streams
 * never mixes state between conversations.
 */
export function useThreads(onSettled?: (id: string) => unknown) {
  const [threads, setThreads] = useState<Record<string, Message[]>>({});
  const [live, setLive] = useState<Record<string, LiveReply>>({});
  const [errors, setErrors] = useState<Record<string, string>>({});
  /** Mirror of `live` for callbacks that run outside render. */
  const liveRef = useRef<Record<string, LiveReply>>({});
  /** Latest load per conversation; older responses are dropped. */
  const loadSeq = useRef<Record<string, number>>({});
  const buffers = useRef<Record<string, DeltaBuffer>>({});
  const settledRef = useRef(onSettled);

  useEffect(() => {
    settledRef.current = onSettled;
  }, [onSettled]);

  useEffect(
    () => () => {
      for (const buf of Object.values(buffers.current)) {
        if (buf.raf !== null) cancelAnimationFrame(buf.raf);
      }
    },
    [],
  );

  const commitLive = useCallback((next: Record<string, LiveReply>) => {
    liveRef.current = next;
    setLive(next);
  }, []);

  const setError = useCallback((id: string, message: string) => {
    setErrors((x) => ({ ...x, [id]: message }));
  }, []);

  const clearError = useCallback((id: string) => {
    setErrors((x) => {
      if (!(id in x)) return x;
      const next = { ...x };
      delete next[id];
      return next;
    });
  }, []);

  /** Applies buffered deltas to the live reply at most once per animation frame. */
  const flush = useCallback(
    (id: string) => {
      const buf = buffers.current[id];
      if (!buf) return;
      if (buf.raf !== null) {
        cancelAnimationFrame(buf.raf);
        buf.raf = null;
      }
      const cur = liveRef.current[id];
      if (!cur) {
        delete buffers.current[id];
        return;
      }
      if (!buf.content && !buf.thinking) return;
      const next = {
        ...cur,
        content: cur.content + buf.content,
        thinking: cur.thinking + buf.thinking,
        searching: false,
        // The answer has started: the research progress line gives way to it.
        research: buf.content ? null : cur.research,
      };
      buf.content = "";
      buf.thinking = "";
      commitLive({ ...liveRef.current, [id]: next });
    },
    [commitLive],
  );

  /** Fetches a thread; resolves to undefined when a newer load for the same id has started. */
  const fetchThread = useCallback(async (id: string): Promise<Message[] | undefined> => {
    const seq = (loadSeq.current[id] ?? 0) + 1;
    loadSeq.current[id] = seq;
    const msgs = await api.getMessages(id);
    return loadSeq.current[id] === seq ? msgs : undefined;
  }, []);

  const reload = useCallback(
    async (id: string) => {
      try {
        const msgs = await fetchThread(id);
        if (msgs) setThreads((t) => ({ ...t, [id]: msgs }));
      } catch (e) {
        setError(id, errorText(e));
      }
    },
    [fetchThread, setError],
  );

  /** Loads a thread unless a reply is streaming into it (its optimistic state must stay). */
  const load = useCallback(
    (id: string) => {
      if (liveRef.current[id]) return Promise.resolve();
      return reload(id);
    },
    [reload],
  );

  /**
   * Runs one streamed reply into `id`: opens the live bubble, folds events into it,
   * then reloads the saved thread. `run` receives the event handler to pass to the backend.
   */
  const stream = useCallback(
    async (id: string, run: (onEvent: (e: ChatEvent) => void) => Promise<unknown>) => {
      clearError(id);
      commitLive({ ...liveRef.current, [id]: emptyReply() });
      buffers.current[id] = { content: "", thinking: "", raf: null };

      const onEvent = (e: ChatEvent) => {
        const buf = buffers.current[id];
        if (!buf || !liveRef.current[id]) return;
        if (e.type === "token" || e.type === "thinking") {
          if (e.type === "token") buf.content += e.delta;
          else buf.thinking += e.delta;
          if (buf.raf === null) {
            buf.raf = requestAnimationFrame(() => {
              buf.raf = null;
              flush(id);
            });
          }
          return;
        }
        flush(id);
        if (e.type === "error") {
          setError(id, e.message);
          return;
        }
        if (e.type === "messageSaved") {
          // The bubble so far is now a saved message; the continuation gets a fresh live bubble.
          // The settle reload replaces this thread anyway, so the push only has to be id-deduped.
          const saved = normalizeMessage(e.message);
          setThreads((t) => {
            const list = t[id] ?? [];
            const known = list.some((m) => m.id === saved.id);
            return { ...t, [id]: known ? list.map((m) => (m.id === saved.id ? saved : m)) : [...list, saved] };
          });
          commitLive({ ...liveRef.current, [id]: emptyReply() });
          return;
        }
        const cur = liveRef.current[id];
        if (!cur) return;
        if (e.type === "searching") {
          commitLive({ ...liveRef.current, [id]: { ...cur, searching: true } });
          return;
        }
        if (e.type === "sources") {
          commitLive({ ...liveRef.current, [id]: { ...cur, searching: false, sources: e.sources ?? [] } });
          return;
        }
        if (e.type === "research") {
          commitLive({ ...liveRef.current, [id]: { ...cur, research: e } });
          return;
        }
        if (e.type === "skill" || e.type === "toolCall" || e.type === "toolResult") {
          commitLive({ ...liveRef.current, [id]: { ...cur, events: [...cur.events, e] } });
          return;
        }
        // Anything else comes from an extension of the backend; its message addon shows it.
        const other = e as unknown as ExtEvent;
        commitLive({ ...liveRef.current, [id]: { ...cur, extEvents: [...cur.extEvents, other] } });
      };

      try {
        await run(onEvent);
      } catch (e) {
        setError(id, errorText(e));
      } finally {
        const buf = buffers.current[id];
        if (buf && buf.raf !== null) cancelAnimationFrame(buf.raf);
        delete buffers.current[id];
        // Load the saved reply before dropping the live one so the bubble never flickers away.
        let msgs: Message[] | undefined;
        try {
          msgs = await fetchThread(id);
        } catch (e) {
          setError(id, errorText(e));
        }
        if (msgs) setThreads((t) => ({ ...t, [id]: msgs }));
        const next = { ...liveRef.current };
        delete next[id];
        commitLive(next);
        try {
          await settledRef.current?.(id);
        } catch {
          /* the list refresh is best effort */
        }
      }
    },
    [clearError, commitLive, fetchThread, flush, setError],
  );

  const send = useCallback(
    (id: string, text: string, attachments: Attachment[], options: SendOptions = {}) => {
      const optimistic: Message = {
        id: `pending-${Date.now()}`,
        conversationId: id,
        role: "user",
        content: text.trim(),
        thinking: "",
        attachments,
        createdAt: Date.now(),
        toolEvents: [],
      };
      setThreads((t) => ({ ...t, [id]: [...(t[id] ?? []), optimistic] }));
      return stream(id, (onEvent) => api.sendMessage(id, text, attachments, options, onEvent));
    },
    [stream],
  );

  /** Drops local state for a conversation that no longer exists. */
  const forget = useCallback(
    (id: string) => {
      loadSeq.current[id] = (loadSeq.current[id] ?? 0) + 1;
      setThreads((t) => {
        if (!(id in t)) return t;
        const next = { ...t };
        delete next[id];
        return next;
      });
      clearError(id);
    },
    [clearError],
  );

  return { threads, live, errors, load, reload, send, stream, forget, clearError };
}

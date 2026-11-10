import { DragEvent, useCallback, useEffect, useRef, useState } from "react";
import { AllowedFolder, api, Attachment, Conversation, EngineStatus, errorText, ExtEvent, formatGb, InstalledModel, Message, Project, ResearchStatus, SendOptions, SkillSummary, Source } from "../lib/api";
import { FilesUpdater, Drafts, draftKey } from "../lib/drafts";
import { usePendingAttachments } from "../lib/attachments";
import { LiveReply } from "../lib/useThreads";
import { ext } from "../ext";
import type { WorkspaceHandle } from "../ext/types";
import { AttachmentChip } from "./AttachmentChip";
import { Composer } from "./Composer";
import { CheckIcon, FolderIcon, GlobeIcon, Logo } from "./Icons";
import { Menu } from "./ui/Menu";
import { Markdown } from "./Markdown";
import { ReplyEvent, SkillChip, ToolCalls } from "./chat/ToolCalls";
import { MessageActions } from "./chat/MessageActions";
import { Sources } from "./chat/Sources";

const { MessageAddon } = ext;

/** Explicit choices for one message: a skill and/or folders pinned in the composer. */
export type SendPins = Pick<SendOptions, "skillId" | "folderIds">;

interface Props {
  conversation: Conversation | null;
  project: Project | null;
  onOpenProject: () => void;
  messages: Message[];
  /** True while the thread for this conversation has not been fetched yet. */
  loading?: boolean;
  /** The reply streaming into this conversation, if any. */
  live: LiveReply | null;
  error: string | null;
  engine: EngineStatus;
  onEngineRetry: () => void;
  /** The model being switched to, while a switch runs. */
  switchingModel: string | null;
  /** Switches the chat model for the whole app. */
  onSwitchModel: (tag: string) => void;
  drafts: Drafts;
  /** "Show what DistroClaw used" setting. */
  showSkillChips: boolean;
  /** All skills; the composer offers the enabled, available ones. */
  skills: SkillSummary[];
  folders: AllowedFolder[];
  /** The Think pill in the composer (the "thinking" setting, shared by every chat). */
  thinking: boolean;
  onToggleThinking: () => void;
  /** The Search pill in the composer (the "web_search" setting, shared by every chat). */
  webSearch: boolean;
  onToggleWebSearch: () => void;
  onSend: (text: string, attachments: Attachment[], pins: SendPins) => void;
  onStop: () => void;
  /** Passed on to the extension's message addon. */
  workspace: WorkspaceHandle;
}

const SUGGESTIONS = [
  "Explain how public-key cryptography works, simply",
  "Write a Python function that removes duplicates from a list",
  "Summarize the pros and cons of working offline-first",
];

function Thinking({ text, live }: { text: string; live: boolean }) {
  if (!text.trim()) return null;
  return (
    <details className="thinking" open={live}>
      <summary>{live ? "Thinking…" : "Show reasoning"}</summary>
      <div className="thinking-body">{text.trim()}</div>
    </details>
  );
}

function Attachments({ items }: { items: Attachment[] }) {
  if (items.length === 0) return null;
  const images = items.filter((a) => a.kind === "image" && a.data);
  const docs = items.filter((a) => a.kind !== "image");
  return (
    <div className="msg-attachments">
      {images.map((a, i) => (
        <img key={i} className="msg-image" src={`data:${a.mime};base64,${a.data}`} alt={a.name} title={a.name} />
      ))}
      {docs.map((a, i) => (
        <AttachmentChip key={i} name={a.name} attachment={a} />
      ))}
    </div>
  );
}

interface BubbleProps {
  role: string;
  content: string;
  thinking: string;
  attachments?: Attachment[];
  live?: boolean;
  events?: ReplyEvent[];
  /** Pages found by online search, cited in the text as `[n]`. */
  sources?: Source[];
  /** Online search is running for this live reply. */
  searching?: boolean;
  /** Olymp's progress while it researches for this live reply. */
  research?: ResearchStatus | null;
  showSkillChips?: boolean;
  /** The saved message; absent while the reply streams. */
  saved?: Message;
  /** Streamed events the core does not handle, for the message addon. */
  extEvents?: ExtEvent[];
  workspace: WorkspaceHandle;
}

/** One line of Olymp's progress: the step it is on and how far it got. */
function ResearchLine({ status }: { status: ResearchStatus }) {
  const parts = [
    status.searches > 0 && `${status.searches} search${status.searches === 1 ? "" : "es"}`,
    status.reads > 0 && `${status.reads} read`,
    status.sources > 0 && `${status.sources} source${status.sources === 1 ? "" : "s"}`,
  ].filter(Boolean);
  return (
    <div className="searching research" role="status" aria-live="polite">
      <GlobeIcon />
      <span>
        <b>Olymp</b> · {status.detail}
        {parts.length > 0 && <span className="research-count"> · {parts.join(", ")}</span>}
      </span>
    </div>
  );
}

function Bubble({ role, content, thinking, attachments = [], live, events = [], sources, searching = false, research = null, showSkillChips = false, saved, extEvents = [], workspace }: BubbleProps) {
  if (role === "user") {
    return (
      <div className="msg user">
        <Attachments items={attachments} />
        {content && <div className="bubble">{content}</div>}
      </div>
    );
  }
  const seen = new Set<string>();
  const skills = events.flatMap((e) => (e.type === "skill" && !seen.has(e.id) && seen.add(e.id) ? [e] : []));
  const waiting = !content && (searching || !!research || toolRowsPending(events));
  return (
    <div className="msg assistant">
      {live && searching && (
        <div className="searching" role="status">
          <GlobeIcon /> Searching the web…
        </div>
      )}
      {live && research && !content && <ResearchLine status={research} />}
      <ToolCalls events={events} live={live} />
      <Thinking text={thinking} live={!!live && !content} />
      {content ? (
        <Markdown text={content} live={live} sources={sources} />
      ) : (
        live && !waiting && <span className="typing"><i /><i /><i /></span>
      )}
      {sources && sources.length > 0 && <Sources sources={sources} />}
      {MessageAddon && <MessageAddon message={saved ?? null} events={extEvents} workspace={workspace} />}
      {showSkillChips && skills.map((s) => <SkillChip key={s.id} title={s.title} />)}
      {!live && content && <MessageActions content={content} />}
    </div>
  );
}

/** The sources saved with a reply; the stored array itself, so the memoised Markdown keeps its props. */
const savedSources = (events: ReplyEvent[]): Source[] | undefined => {
  for (const e of events) if (e.type === "sources") return e.sources;
  return undefined;
};

/** True while a tool call has no result yet, so the typing dots do not double up with the spinner. */
const toolRowsPending = (events: ReplyEvent[]) => {
  const done = new Set(events.flatMap((e) => (e.type === "toolResult" ? [e.id] : [])));
  return events.some((e) => e.type === "toolCall" && !done.has(e.id));
};

/** The header badge: a button that opens the installed chat models and switches between them. */
function ModelBadge({ engine, switchingModel, onSwitch }: { engine: EngineStatus; switchingModel: string | null; onSwitch: (tag: string) => void }) {
  const anchor = useRef<HTMLDivElement>(null);
  const button = useRef<HTMLButtonElement>(null);
  const [open, setOpen] = useState(false);
  const [models, setModels] = useState<InstalledModel[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  /** Ignores a list that arrives after the menu was closed or reopened. */
  const request = useRef(0);
  const ready = engine.state === "ready";
  const locked = switchingModel !== null || !ready;

  const close = () => {
    request.current += 1;
    setOpen(false);
  };

  const toggle = () => {
    if (open) {
      close();
      return;
    }
    if (locked) return;
    // WebKit does not focus a clicked button; the menu returns focus to whatever had it when it opened.
    button.current?.focus();
    const id = ++request.current;
    setModels(null);
    setListError(null);
    setOpen(true);
    api
      .listChatModels()
      .then((list) => id === request.current && setModels(list))
      .catch((e) => id === request.current && setListError(errorText(e)));
  };

  return (
    <div className="menu-anchor model-anchor" ref={anchor}>
      <button
        ref={button}
        type="button"
        className="badge"
        aria-haspopup="menu"
        aria-expanded={open}
        aria-disabled={locked || undefined}
        aria-busy={switchingModel !== null || undefined}
        title={ready ? "Switch the chat model" : engine.message || "Local AI is not ready"}
        onClick={toggle}
      >
        <span className="dot" style={ready ? undefined : { ["--ok" as string]: "var(--warn)" }} />{" "}
        {switchingModel !== null ? "Switching…" : `${engine.model} · local`}
      </button>
      {open && (
        <Menu
          // Remounts once the list arrives, so the menu focuses the current model.
          key={models ? "list" : "loading"}
          ariaLabel="Chat model"
          anchorRef={anchor}
          onClose={close}
          empty={listError ?? (models ? "No chat models installed" : "Loading…")}
          items={(models ?? []).map((m) => ({
            id: m.name,
            checked: m.current,
            label: (
              <>
                <span className="model-check">{m.current && <CheckIcon />}</span>
                <span className="model-name">{m.name}</span>
                <span className="muted">{formatGb(m.sizeBytes)}</span>
              </>
            ),
            onSelect: () => {
              if (!m.current) onSwitch(m.name);
            },
          }))}
        />
      )}
    </div>
  );
}

export function ChatView({ conversation, project, onOpenProject, messages, loading, live, error, engine, onEngineRetry, switchingModel, onSwitchModel, drafts, showSkillChips, skills, folders, thinking, onToggleThinking, webSearch, onToggleWebSearch, onSend, onStop, workspace }: Props) {
  const scroller = useRef<HTMLDivElement>(null);
  const key = draftKey(conversation?.id);
  const draft = drafts.get(key);
  const { setFiles, setText, setSkill, setFolders, clear: clearDraft } = drafts;
  const usableSkills = skills.filter((s) => s.enabled && s.available);
  const pinnedSkill = usableSkills.find((s) => s.id === draft.skillId) ?? null;
  const pinnedFolders = folders.filter((f) => draft.folderIds.includes(f.id));
  const toggleFolder = (id: string) =>
    setFolders(key, (ids) => (ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id]));
  const setItems = useCallback((fn: FilesUpdater) => setFiles(key, fn), [setFiles, key]);
  const files = usePendingAttachments({ items: draft.files, setItems });
  const [dragging, setDragging] = useState(false);
  const dragDepth = useRef(0);

  useEffect(() => {
    const el = scroller.current;
    if (el) el.scrollTop = el.scrollHeight;
  }, [messages, live?.content, live?.thinking, live?.events, live?.searching, live?.sources, live?.extEvents, live?.research]);

  // Screen readers hear when a reply starts and ends.
  const [announce, setAnnounce] = useState("");
  const wasLive = useRef(false);
  useEffect(() => {
    if (live) {
      wasLive.current = true;
      setAnnounce("DistroClaw is replying");
    } else if (wasLive.current) {
      wasLive.current = false;
      setAnnounce("Reply finished");
    }
  }, [live]);

  // Stop the active reply when the window closes mid-stream. Switching views keeps the
  // reply running: it belongs to the conversation, not to this view.
  const liveState = useRef<{ id: string | undefined; live: boolean }>({ id: undefined, live: false });
  useEffect(() => {
    liveState.current = { id: conversation?.id, live: live !== null };
  }, [conversation?.id, live]);
  useEffect(() => {
    const stop = () => {
      const s = liveState.current;
      if (s.id && s.live) api.stopGeneration(s.id).catch(() => {});
    };
    window.addEventListener("beforeunload", stop);
    return () => window.removeEventListener("beforeunload", stop);
  }, []);

  const send = (text: string, attachments: Attachment[]) => {
    onSend(text, attachments, {
      skillId: pinnedSkill?.id,
      folderIds: pinnedFolders.length > 0 ? pinnedFolders.map((f) => f.id) : undefined,
    });
    clearDraft(key);
  };

  const hasFiles = (e: DragEvent) => Array.from(e.dataTransfer.types).includes("Files");
  const onDragEnter = (e: DragEvent) => {
    if (!hasFiles(e)) return;
    e.preventDefault();
    dragDepth.current += 1;
    setDragging(true);
  };
  const onDragLeave = () => {
    dragDepth.current = Math.max(0, dragDepth.current - 1);
    if (dragDepth.current === 0) setDragging(false);
  };
  const onDrop = (e: DragEvent) => {
    e.preventDefault();
    dragDepth.current = 0;
    setDragging(false);
    if (e.dataTransfer.files.length) files.add(e.dataTransfer.files);
  };

  const empty = messages.length === 0 && !live && !loading;
  const engineReady = engine.state === "ready";

  return (
    <main
      className="chat"
      onDragEnter={onDragEnter}
      onDragOver={(e) => hasFiles(e) && e.preventDefault()}
      onDragLeave={onDragLeave}
      onDrop={onDrop}
    >
      <header className="chat-header" data-tauri-drag-region>
        <span className="chat-title" data-tauri-drag-region>
          {project && (
            <button className="crumb" onClick={onOpenProject} title="Project settings">
              <FolderIcon /> {project.name}
            </button>
          )}
          {project && <span className="crumb-sep">/</span>}
          <span className="chat-title-text">{conversation?.title ?? "New chat"}</span>
        </span>
        <ModelBadge engine={engine} switchingModel={switchingModel} onSwitch={onSwitchModel} />
      </header>

      {!engineReady && (
        <div className="engine-banner" role="status">
          <span>{engine.state === "error" || engine.state === "needs_model" ? engine.message : "Restarting local AI…"}</span>
          {engine.state === "error" && (
            <button className="btn tiny" onClick={onEngineRetry}>
              Retry
            </button>
          )}
        </div>
      )}

      <div className="sr-only" role="status" aria-live="polite">
        {announce}
      </div>

      <div className="messages" ref={scroller}>
        {empty ? (
          <div className="empty">
            <Logo size={60} />
            <h2>
              How can I <em>help</em> today?
            </h2>
            <div className="suggestions">
              {SUGGESTIONS.map((s) => (
                <button key={s} className="suggestion" onClick={() => send(s, [])}>
                  {s}
                </button>
              ))}
            </div>
          </div>
        ) : (
          <div className="thread">
            {messages.map((m) => (
              <Bubble
                key={m.id}
                role={m.role}
                content={m.content}
                thinking={m.thinking}
                attachments={m.attachments}
                events={m.toolEvents}
                sources={savedSources(m.toolEvents)}
                showSkillChips={showSkillChips}
                saved={m}
                workspace={workspace}
              />
            ))}
            {live && (
              <Bubble
                role="assistant"
                content={live.content}
                thinking={live.thinking}
                events={live.events}
                sources={live.sources}
                searching={live.searching}
                research={live.research}
                showSkillChips={showSkillChips}
                extEvents={live.extEvents}
                workspace={workspace}
                live
              />
            )}
            {error && <div className="error-banner">{error}</div>}
          </div>
        )}
      </div>

      <Composer
        busy={live !== null}
        disabled={!engineReady}
        focusKey={conversation?.id}
        text={draft.text}
        onTextChange={(t) => setText(key, t)}
        pending={files.items}
        ready={files.ready}
        reading={files.reading}
        onAddFiles={files.add}
        onRemoveFile={files.remove}
        skills={usableSkills}
        folders={folders.filter((f) => f.available)}
        pinnedSkill={pinnedSkill}
        pinnedFolders={pinnedFolders}
        onPinSkill={(id) => setSkill(key, id)}
        onToggleFolder={toggleFolder}
        thinking={thinking}
        onToggleThinking={onToggleThinking}
        webSearch={webSearch}
        onToggleWebSearch={onToggleWebSearch}
        onSend={send}
        onStop={onStop}
      />

      {dragging && (
        <div className="drop-overlay">
          <div>Drop images or documents to attach</div>
        </div>
      )}
    </main>
  );
}

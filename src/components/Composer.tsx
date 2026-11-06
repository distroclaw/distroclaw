import { ClipboardEvent, KeyboardEvent, useEffect, useId, useMemo, useRef, useState } from "react";
import { AllowedFolder, Attachment, SkillSummary } from "../lib/api";
import { PendingAttachment } from "../lib/attachments";
import { truncateMiddle } from "../lib/format";
import { AttachmentChip } from "./AttachmentChip";
import { FileIcon, FolderIcon, GlobeIcon, PlusIcon, SendIcon, SparkleIcon, StopIcon, ThinkIcon, XIcon } from "./Icons";
import { Menu, MenuItem } from "./ui/Menu";

interface Props {
  busy: boolean;
  /** Sending is blocked (for example while the local engine restarts). */
  disabled?: boolean;
  focusKey: string | undefined;
  /** Draft text, held by the parent so each conversation keeps its own. */
  text: string;
  onTextChange: (text: string) => void;
  pending: PendingAttachment[];
  ready: Attachment[];
  reading: boolean;
  onAddFiles: (files: Iterable<File>) => void;
  onRemoveFile: (key: number) => void;
  /** Skills the user may pick explicitly (enabled and available). */
  skills: SkillSummary[];
  /** Allowed folders that can be pinned to one message. */
  folders: AllowedFolder[];
  pinnedSkill: SkillSummary | null;
  pinnedFolders: AllowedFolder[];
  onPinSkill: (id: string | null) => void;
  onToggleFolder: (id: string) => void;
  /** Whether the model thinks before it answers; remembered across chats. */
  thinking: boolean;
  onToggleThinking: () => void;
  /** Whether the app may search the web before the model answers; remembered across chats. */
  webSearch: boolean;
  onToggleWebSearch: () => void;
  onSend: (text: string, attachments: Attachment[]) => void;
  onStop: () => void;
}

type PlusMenu = "main" | "folders" | "skills";

/** The "/query" at the start of the draft, or null when the draft is not a slash command. */
const slashQueryOf = (text: string): string | null => {
  const m = /^\/(\S*)$/.exec(text);
  return m ? m[1].toLowerCase() : null;
};

export function Composer(props: Props) {
  const {
    busy, disabled = false, focusKey, text, onTextChange, pending, ready, reading, onAddFiles, onRemoveFile,
    skills, folders, pinnedSkill, pinnedFolders, onPinSkill, onToggleFolder, thinking, onToggleThinking,
    webSearch, onToggleWebSearch, onSend, onStop,
  } = props;
  const input = useRef<HTMLTextAreaElement>(null);
  const picker = useRef<HTMLInputElement>(null);
  const plusAnchor = useRef<HTMLDivElement>(null);
  const [plusMenu, setPlusMenu] = useState<PlusMenu | null>(null);
  const [slashIndex, setSlashIndex] = useState(0);
  /** Escape hides the slash list until the draft changes again. */
  const [slashDismissed, setSlashDismissed] = useState(false);
  const listId = useId();

  useEffect(() => input.current?.focus(), [focusKey]);

  useEffect(() => {
    const el = input.current;
    if (!el) return;
    el.style.height = "auto";
    el.style.height = `${Math.min(el.scrollHeight, 200)}px`;
  }, [text]);

  const slashQuery = slashQueryOf(text);
  const slashItems = useMemo(() => {
    if (slashQuery === null || slashDismissed || pinnedSkill) return [];
    return skills.filter((s) => s.slash.slice(1).toLowerCase().startsWith(slashQuery) || s.title.toLowerCase().includes(slashQuery));
  }, [skills, slashQuery, slashDismissed, pinnedSkill]);
  const slashOpen = slashItems.length > 0;
  const activeIndex = Math.min(slashIndex, Math.max(0, slashItems.length - 1));

  useEffect(() => {
    setSlashIndex(0);
    setSlashDismissed(false);
  }, [text]);

  const pickSkill = (s: SkillSummary) => {
    onPinSkill(s.id);
    if (slashQuery !== null) onTextChange("");
    input.current?.focus();
  };

  const canSend = !busy && !disabled && !reading && (text.trim().length > 0 || ready.length > 0);

  const submit = () => {
    if (!canSend) return;
    onSend(text, ready);
  };

  const onKey = (e: KeyboardEvent<HTMLTextAreaElement>) => {
    if (slashOpen) {
      if (e.key === "ArrowDown" || e.key === "ArrowUp") {
        e.preventDefault();
        const n = slashItems.length;
        setSlashIndex((i) => (e.key === "ArrowDown" ? (i + 1) % n : (i - 1 + n) % n));
        return;
      }
      if (e.key === "Enter" || e.key === "Tab") {
        e.preventDefault();
        pickSkill(slashItems[activeIndex]);
        return;
      }
      if (e.key === "Escape") {
        e.preventDefault();
        setSlashDismissed(true);
        return;
      }
    }
    if (e.key === "Enter" && !e.shiftKey && !e.nativeEvent.isComposing) {
      e.preventDefault();
      submit();
    }
  };

  // Cmd+V with an image or files on the clipboard attaches them; plain text pastes normally.
  const onPaste = (e: ClipboardEvent<HTMLTextAreaElement>) => {
    const files = Array.from(e.clipboardData.files);
    if (files.length === 0) return;
    e.preventDefault();
    onAddFiles(files);
  };

  const pinnedIds = new Set(pinnedFolders.map((f) => f.id));
  const back: MenuItem = { id: "back", label: "‹ Back", keepOpen: true, onSelect: () => setPlusMenu("main") };
  const plusItems: Record<PlusMenu, { label: string; items: MenuItem[]; hint?: string }> = {
    main: {
      label: "Add to message",
      items: [
        { id: "files", label: <><FileIcon /> Images or documents…</>, onSelect: () => picker.current?.click() },
        { id: "folder", label: <><FolderIcon /> A folder for this message…</>, keepOpen: true, disabled: folders.length === 0, onSelect: () => setPlusMenu("folders") },
        { id: "skill", label: <><SparkleIcon /> Use a skill…</>, keepOpen: true, disabled: skills.length === 0, onSelect: () => setPlusMenu("skills") },
      ],
      hint: folders.length === 0 ? "Add a folder in Settings → Access to use it here." : undefined,
    },
    folders: {
      label: "Folders",
      items: [
        back,
        ...folders.map((f) => ({
          id: f.id,
          label: (
            <>
              <FolderIcon /> {f.label}
              {pinnedIds.has(f.id) && <span className="muted">· pinned</span>}
            </>
          ),
          keepOpen: true,
          onSelect: () => onToggleFolder(f.id),
        })),
      ],
    },
    skills: {
      label: "Skills",
      items: [
        back,
        ...skills.map((s) => ({
          id: s.id,
          label: (
            <>
              <span className="mono muted">{s.slash}</span> {s.title}
            </>
          ),
          onSelect: () => pickSkill(s),
        })),
      ],
    },
  };

  const hasChips = pending.length > 0 || pinnedSkill !== null || pinnedFolders.length > 0;

  return (
    <div className="composer-wrap">
      <div className="composer">
        {slashOpen && (
          <div className="menu slash-menu" role="listbox" id={listId} aria-label="Skills">
            {slashItems.map((s, i) => (
              <button
                key={s.id}
                type="button"
                role="option"
                id={`${listId}-${i}`}
                aria-selected={i === activeIndex}
                className={i === activeIndex ? "on" : ""}
                onMouseDown={(e) => e.preventDefault()}
                onMouseEnter={() => setSlashIndex(i)}
                onClick={() => pickSkill(s)}
              >
                <span className="mono">{s.slash}</span>
                <b>{s.title}</b>
                <span className="muted">{s.description}</span>
              </button>
            ))}
          </div>
        )}
        {hasChips && (
          <div className="composer-attachments">
            {pinnedSkill && (
              <div className="chip pin" title={pinnedSkill.description}>
                <span className="chip-icon"><SparkleIcon /></span>
                <span className="chip-text">
                  <span className="chip-name">{pinnedSkill.title}</span>
                  <span className="chip-detail mono">{pinnedSkill.slash}</span>
                </span>
                <button type="button" className="chip-remove" aria-label={`Remove skill ${pinnedSkill.title}`} title="Remove" onClick={() => onPinSkill(null)}>
                  <XIcon />
                </button>
              </div>
            )}
            {pinnedFolders.map((f) => (
              <div key={f.id} className="chip pin" title={f.path}>
                <span className="chip-icon"><FolderIcon /></span>
                <span className="chip-text">
                  <span className="chip-name">{f.label}</span>
                  <span className="chip-detail mono">{truncateMiddle(f.path, 28)}</span>
                </span>
                <button type="button" className="chip-remove" aria-label={`Remove folder ${f.label}`} title="Remove" onClick={() => onToggleFolder(f.id)}>
                  <XIcon />
                </button>
              </div>
            ))}
            {pending.map((p) => (
              <AttachmentChip
                key={p.key}
                name={p.name}
                attachment={p.attachment}
                status={p.status}
                error={p.error}
                onRemove={() => onRemoveFile(p.key)}
              />
            ))}
          </div>
        )}
        <div className="composer-row">
          <div className="menu-anchor up" ref={plusAnchor}>
            <button
              type="button"
              className="attach"
              aria-label="Add to message"
              aria-haspopup="menu"
              aria-expanded={plusMenu !== null}
              title="Add images, documents, a folder or a skill"
              onClick={() => setPlusMenu((m) => (m ? null : "main"))}
            >
              <PlusIcon />
            </button>
            {plusMenu && (
              <Menu
                key={plusMenu}
                ariaLabel="Add to message"
                label={plusItems[plusMenu].label}
                items={plusItems[plusMenu].items}
                hint={plusItems[plusMenu].hint}
                anchorRef={plusAnchor}
                onClose={() => setPlusMenu(null)}
              />
            )}
          </div>
          <input
            ref={picker}
            type="file"
            multiple
            hidden
            onChange={(e) => {
              if (e.target.files) onAddFiles(e.target.files);
              e.target.value = "";
              input.current?.focus();
            }}
          />
          <textarea
            ref={input}
            rows={1}
            value={text}
            placeholder={skills.length > 0 ? "Message DistroClaw… (type / for a skill)" : "Message DistroClaw…"}
            aria-autocomplete={skills.length > 0 ? "list" : undefined}
            aria-controls={slashOpen ? listId : undefined}
            aria-expanded={slashOpen}
            aria-activedescendant={slashOpen ? `${listId}-${activeIndex}` : undefined}
            onChange={(e) => onTextChange(e.target.value)}
            onKeyDown={onKey}
            onPaste={onPaste}
          />
          <button
            type="button"
            className="think-toggle"
            aria-pressed={thinking}
            title={thinking ? "Thinking is on: the model reasons before it answers" : "Thinking is off: faster, shorter replies"}
            onClick={onToggleThinking}
          >
            <ThinkIcon /> Think
          </button>
          <button
            type="button"
            className="search-toggle"
            aria-pressed={webSearch}
            title={webSearch ? "Online search is on: looks things up on the internet when a question needs it" : "Online search is off: answers come from the model alone"}
            onClick={onToggleWebSearch}
          >
            <GlobeIcon /> Search
          </button>
          {busy ? (
            <button className="send stop" onClick={onStop} title="Stop" aria-label="Stop">
              <StopIcon />
            </button>
          ) : (
            <button className="send" onClick={submit} disabled={!canSend} title="Send" aria-label="Send">
              <SendIcon />
            </button>
          )}
        </div>
      </div>
      <p className="composer-hint">
        Paste or drop images and documents. {webSearch ? "Your files stay on this computer." : "Everything stays on this computer."} DistroClaw can make mistakes. Check important information.
      </p>
    </div>
  );
}

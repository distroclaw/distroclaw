import { KeyboardEvent, useState } from "react";
import { AllowedFolder, Conversation, Project } from "../lib/api";
import { formatCount } from "../lib/format";
import { ext } from "../ext";
import { ChevronIcon, FileIcon, FolderIcon, GearIcon, Logo, PencilIcon, PlusIcon, TrashIcon } from "./Icons";
import { Menu } from "./ui/Menu";
import type { SettingsTab } from "./SettingsDialog";

const { SidebarAreas, WorkspaceStatus } = ext;

/** The core area: a new chat about the allowed folders. */
const DOCUMENTS = "documents";

interface Props {
  conversations: Conversation[];
  projects: Project[];
  activeId: string | null;
  /** Conversations a reply is currently streaming into. */
  liveIds: Set<string>;
  onSelect: (id: string) => void;
  onNew: (projectId: string | null) => void;
  onRename: (id: string, title: string) => void;
  onDelete: (id: string) => void;
  onMove: (id: string, projectId: string | null) => void;
  onNewProject: () => void;
  onEditProject: (project: Project) => void;
  onOpenSettings: (tab?: SettingsTab) => void;
  onOpenArea: (area: string) => void;
  activeArea: string | null;
  folders: AllowedFolder[];
}

/** Enter or Space activates a row the way a click does. */
const activateOnKey = (action: () => void) => (e: KeyboardEvent<HTMLElement>) => {
  if (e.target !== e.currentTarget) return;
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    action();
  }
};

interface RowProps {
  conversation: Conversation;
  projects: Project[];
  active: boolean;
  live: boolean;
  onSelect: () => void;
  onRename: (title: string) => void;
  onDelete: () => void;
  onMove: (projectId: string | null) => void;
}

function ConversationRow({ conversation: c, projects, active, live, onSelect, onRename, onDelete, onMove }: RowProps) {
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(c.title);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [moving, setMoving] = useState(false);

  const commit = () => {
    if (draft.trim() && draft.trim() !== c.title) onRename(draft.trim());
    setEditing(false);
  };
  const onKey = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "Enter") commit();
    if (e.key === "Escape") setEditing(false);
  };
  const startRename = () => {
    setDraft(c.title);
    setEditing(true);
  };

  return (
    <div
      className={`conversation ${active ? "active" : ""} ${moving ? "menu-open" : ""}`}
      role="button"
      tabIndex={editing ? -1 : 0}
      aria-current={active ? "true" : undefined}
      onClick={() => !editing && onSelect()}
      onKeyDown={activateOnKey(() => !editing && onSelect())}
    >
      {editing ? (
        <input
          className="input rename"
          value={draft}
          autoFocus
          aria-label="Chat name"
          onChange={(e) => setDraft(e.target.value)}
          onKeyDown={onKey}
          onBlur={commit}
          onClick={(e) => e.stopPropagation()}
        />
      ) : (
        <span className="conversation-title" onDoubleClick={startRename}>
          {c.title}
        </span>
      )}
      {live && !editing && <span className="dot" title="Replying" aria-label="Replying" role="img" />}
      {!editing && (
        <div className="conversation-actions" onClick={(e) => e.stopPropagation()}>
          {confirmDelete ? (
            <button className="btn danger tiny" autoFocus onClick={onDelete} onBlur={() => setConfirmDelete(false)}>
              Delete?
            </button>
          ) : (
            <>
              <button
                className="icon-btn"
                title="Move to project"
                aria-label={`Move "${c.title}" to a project`}
                aria-haspopup="menu"
                aria-expanded={moving}
                onClick={(e) => {
                  e.stopPropagation();
                  setMoving((m) => !m);
                }}
              >
                <FolderIcon />
              </button>
              <button className="icon-btn" title="Rename" aria-label={`Rename "${c.title}"`} onClick={startRename}>
                <PencilIcon />
              </button>
              <button className="icon-btn" title="Delete" aria-label={`Delete "${c.title}"`} onClick={() => setConfirmDelete(true)}>
                <TrashIcon />
              </button>
            </>
          )}
        </div>
      )}
      {moving && (
        <Menu
          label="Move to"
          ariaLabel={`Move "${c.title}" to a project`}
          hint={projects.length === 0 ? "Create a project first." : undefined}
          onClose={() => setMoving(false)}
          items={[
            ...projects.map((p) => ({
              id: p.id,
              label: (
                <>
                  <FolderIcon /> {p.name}
                </>
              ),
              disabled: p.id === c.projectId,
              onSelect: () => onMove(p.id),
            })),
            { id: "none", label: "No project", disabled: c.projectId === null, onSelect: () => onMove(null) },
          ]}
        />
      )}
    </div>
  );
}

export function Sidebar(props: Props) {
  const { conversations, projects, activeId, liveIds, onSelect, onNew, onRename, onDelete, onMove, folders } = props;
  const [collapsed, setCollapsed] = useState<Set<string>>(new Set());

  const toggle = (id: string) =>
    setCollapsed((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });

  const row = (c: Conversation) => (
    <ConversationRow
      key={c.id}
      conversation={c}
      projects={projects}
      active={c.id === activeId}
      live={liveIds.has(c.id)}
      onSelect={() => onSelect(c.id)}
      onRename={(t) => onRename(c.id, t)}
      onDelete={() => onDelete(c.id)}
      onMove={(p) => onMove(c.id, p)}
    />
  );

  const loose = conversations.filter((c) => !c.projectId || !projects.some((p) => p.id === c.projectId));
  const fileCount = folders.reduce((n, f) => n + f.fileCount, 0);
  const documentsOn = props.activeArea === DOCUMENTS;
  const folderSummary =
    folders.length > 0 ? (
      <>
        {folders.length} {folders.length === 1 ? "folder" : "folders"} · {formatCount(fileCount)}{" "}
        {fileCount === 1 ? "file" : "files"}
      </>
    ) : (
      <button className="text-link" onClick={() => props.onOpenSettings("access")}>
        Add a folder to get started
      </button>
    );

  return (
    <aside className="sidebar">
      <div className="sidebar-top" data-tauri-drag-region>
        <div className="brand" data-tauri-drag-region>
          <Logo size={24} />
          <span>
            Distro<span className="brand-claw">Claw</span>
          </span>
        </div>
        <button className="btn new-chat" onClick={() => onNew(null)}>
          <PlusIcon /> New chat
        </button>
      </div>

      <nav className="conversation-list" aria-label="Chats and projects">
        <div className="section-header">
          <span>Projects</span>
          <button className="icon-btn" title="New project" aria-label="New project" onClick={props.onNewProject}>
            <PlusIcon />
          </button>
        </div>
        {projects.length === 0 && (
          <button className="empty-project" onClick={props.onNewProject}>
            Group chats and give them shared instructions
          </button>
        )}
        {projects.map((p) => {
          const chats = conversations.filter((c) => c.projectId === p.id);
          const open = !collapsed.has(p.id);
          return (
            <div key={p.id} className="project">
              <div
                className="project-row"
                role="button"
                tabIndex={0}
                aria-expanded={open}
                onClick={() => toggle(p.id)}
                onKeyDown={activateOnKey(() => toggle(p.id))}
              >
                <ChevronIcon open={open} />
                <FolderIcon />
                <span className="project-name">{p.name}</span>
                <div className="project-actions" onClick={(e) => e.stopPropagation()}>
                  <button className="icon-btn" title="New chat in project" aria-label={`New chat in ${p.name}`} onClick={() => onNew(p.id)}>
                    <PlusIcon />
                  </button>
                  <button className="icon-btn" title="Project settings" aria-label={`Settings for ${p.name}`} onClick={() => props.onEditProject(p)}>
                    <GearIcon />
                  </button>
                </div>
              </div>
              {open && (
                <div className="project-chats">
                  {chats.length === 0 ? <p className="muted small pad">No chats yet.</p> : chats.map(row)}
                </div>
              )}
            </div>
          );
        })}

        <div className="section-header">
          <span>Chats</span>
        </div>
        {loose.length === 0 && <p className="muted small pad">No conversations yet.</p>}
        {loose.map(row)}
      </nav>

      <div className="sidebar-bottom">
        <nav className="workspace-areas" aria-label="Workspace">
          <div className="section-header">
            <span>Workspace</span>
          </div>
          <button
            className={`area ${documentsOn ? "on" : ""}`}
            aria-current={documentsOn ? "page" : undefined}
            onClick={() => props.onOpenArea(DOCUMENTS)}
          >
            <FileIcon />
            <span>Documents</span>
          </button>
          {SidebarAreas && <SidebarAreas active={props.activeArea} onOpen={props.onOpenArea} />}
          <p className="workspace-status">
            {WorkspaceStatus ? <WorkspaceStatus folders={folders}>{folderSummary}</WorkspaceStatus> : folderSummary}
          </p>
        </nav>
        <button className="btn settings-btn" onClick={() => props.onOpenSettings()}>
          <GearIcon /> Settings
        </button>
      </div>
    </aside>
  );
}

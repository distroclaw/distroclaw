import { useCallback, useEffect, useRef, useState } from "react";
import { api, Conversation, errorOf, SkillSummary } from "../../lib/api";
import { PencilIcon, PlusIcon, TrashIcon } from "../Icons";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Menu } from "../ui/Menu";
import { Toggle } from "../ui/Toggle";
import { SkillEditor } from "./SkillEditor";

interface Props {
  /** Opens the test chat the backend created and sends the prompt there. */
  onTest: (conversation: Conversation, prompt: string, skillId: string) => void;
  /** Called after any change so the composer's skill list can refresh. */
  onChanged?: () => void;
}

/** The skill that can never be switched off. */
const ALWAYS_ON = "general-chat";

function SkillRow({
  skill: s,
  busy,
  onToggle,
  onEdit,
  onDelete,
}: {
  skill: SkillSummary;
  busy: boolean;
  onToggle: (enabled: boolean) => void;
  onEdit?: () => void;
  onDelete?: () => void;
}) {
  return (
    <li className={`skill-row ${s.available ? "" : "locked"}`}>
      <div className="skill-text">
        <div className="label-row">
          <b>{s.title}</b>
          <span className="mono small muted">{s.slash}</span>
        </div>
        <p className="field-help">{s.description}</p>
        {s.error && <p className="error-text">{s.error}</p>}
      </div>
      <Toggle
        inline
        label={`${s.title} enabled`}
        checked={s.enabled}
        disabled={busy || !s.available || s.id === ALWAYS_ON}
        onChange={onToggle}
      />
      {onEdit && (
        <button type="button" className="icon-btn" title="Edit" aria-label={`Edit ${s.title}`} onClick={onEdit}>
          <PencilIcon />
        </button>
      )}
      {onDelete && (
        <button type="button" className="icon-btn" title="Delete" aria-label={`Delete ${s.title}`} onClick={onDelete}>
          <TrashIcon />
        </button>
      )}
    </li>
  );
}

export function SkillsPanel({ onTest, onChanged }: Props) {
  const [skills, setSkills] = useState<SkillSummary[] | null>(null);
  const [error, setError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [addOpen, setAddOpen] = useState(false);
  // undefined = the list; null = writing a new skill; otherwise editing that skill
  const [editing, setEditing] = useState<SkillSummary | null | undefined>(undefined);
  const [deleting, setDeleting] = useState<SkillSummary | null>(null);
  const addAnchor = useRef<HTMLDivElement>(null);

  const load = useCallback(() => {
    setError(null);
    api
      .listSkills()
      .then(setSkills)
      .catch((e) => setError(errorOf(e).message));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const changed = () => {
    load();
    onChanged?.();
  };

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorOf(e).message);
    } finally {
      setBusy(false);
    }
  };

  const toggle = (s: SkillSummary, enabled: boolean) =>
    run(async () => {
      setSkills((xs) => (xs ?? []).map((x) => (x.id === s.id ? { ...x, enabled } : x)));
      try {
        await api.setSkillEnabled(s.id, enabled);
      } catch (e) {
        setSkills((xs) => (xs ?? []).map((x) => (x.id === s.id ? { ...x, enabled: s.enabled } : x)));
        throw e;
      }
      onChanged?.();
    });

  const importSkill = () =>
    run(async () => {
      const s = await api.importSkill();
      if (s) changed();
    });

  const remove = async (s: SkillSummary) => {
    await api.deleteSkill(s.id);
    setDeleting(null);
    changed();
  };

  if (editing !== undefined) {
    return (
      <SkillEditor
        skill={editing}
        onBack={() => setEditing(undefined)}
        onSaved={changed}
        onTest={onTest}
      />
    );
  }

  const builtin = (skills ?? []).filter((s) => s.source === "builtin");
  const user = (skills ?? []).filter((s) => s.source === "user");

  const list = (items: SkillSummary[], own: boolean) => (
    <ul className="item-list skills">
      {items.map((s) => (
        <SkillRow
          key={s.id}
          skill={s}
          busy={busy}
          onToggle={(on) => toggle(s, on)}
          onEdit={own ? () => setEditing(s) : undefined}
          onDelete={own ? () => setDeleting(s) : undefined}
        />
      ))}
    </ul>
  );

  return (
    <>
      <section className="field">
        <label>Built in</label>
        <p className="field-help">DistroClaw picks the matching skill for each message. Type "/" in a chat to choose one yourself.</p>
        {skills === null && !error && <p className="muted small">Loading…</p>}
        {builtin.length > 0 && list(builtin, false)}
      </section>

      <section className="field">
        <div className="label-row">
          <label>Your skills</label>
          <div className="menu-anchor" ref={addAnchor} style={{ marginLeft: "auto" }}>
            <button
              type="button"
              className="btn"
              disabled={busy || skills === null}
              aria-haspopup="menu"
              aria-expanded={addOpen}
              onClick={() => setAddOpen((o) => !o)}
            >
              <PlusIcon /> Add skill
            </button>
            {addOpen && (
              <Menu
                ariaLabel="Add skill"
                anchorRef={addAnchor}
                onClose={() => setAddOpen(false)}
                items={[
                  { id: "import", label: "Import from folder…", onSelect: importSkill },
                  { id: "write", label: "Write a skill", onSelect: () => setEditing(null) },
                ]}
              />
            )}
          </div>
        </div>
        {skills && user.length === 0 && (
          <p className="muted small">No skills of your own yet. A skill is a short SKILL.md with instructions.</p>
        )}
        {user.length > 0 && list(user, true)}
      </section>

      {error && (
        <div className="error-banner" role="alert">
          {error}
          {skills === null && (
            <>
              {" "}
              <button className="text-link" onClick={load}>
                Try again
              </button>
            </>
          )}
        </div>
      )}

      {deleting && (
        <ConfirmDialog
          title={`Delete ${deleting.title}?`}
          message="The skill file is removed from DistroClaw. Chats that used it are kept."
          confirmLabel="Delete"
          danger
          onConfirm={() => remove(deleting)}
          onCancel={() => setDeleting(null)}
        />
      )}
    </>
  );
}

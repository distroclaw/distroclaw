import { useEffect, useState } from "react";
import { api, errorText, Project, SkillSummary } from "../lib/api";
import { Modal } from "./Modal";

/** A project can keep at most this many skills always on. */
const MAX_PROJECT_SKILLS = 2;

interface Props {
  project: Project | null;
  chatCount: number;
  onSave: (name: string, instructions: string, skillIds: string[]) => Promise<void>;
  onDelete?: () => Promise<void>;
  onClose: () => void;
}

export function ProjectDialog({ project, chatCount, onSave, onDelete, onClose }: Props) {
  const [name, setName] = useState(project?.name ?? "");
  const [instructions, setInstructions] = useState(project?.instructions ?? "");
  const [skillIds, setSkillIds] = useState<string[]>(project?.skillIds ?? []);
  const [skills, setSkills] = useState<SkillSummary[]>([]);
  const [confirmDelete, setConfirmDelete] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    let alive = true;
    api
      .listSkills()
      .then((xs) => alive && setSkills(xs.filter((s) => s.enabled && s.available)))
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, []);

  const toggleSkill = (id: string) =>
    setSkillIds((ids) => (ids.includes(id) ? ids.filter((x) => x !== id) : [...ids, id]));
  const atMax = skillIds.length >= MAX_PROJECT_SKILLS;

  const run = async (action: () => Promise<void>) => {
    try {
      await action();
      onClose();
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <Modal
      title={project ? "Project settings" : "New project"}
      onClose={onClose}
      footer={
        <>
          {onDelete &&
            (confirmDelete ? (
              <button className="btn danger" onClick={() => run(onDelete)}>
                Delete project{chatCount ? ` (keeps ${chatCount} chat${chatCount > 1 ? "s" : ""})` : ""}
              </button>
            ) : (
              <button className="btn ghost-danger" onClick={() => setConfirmDelete(true)}>Delete…</button>
            ))}
          <span className="spacer" />
          <button className="btn" onClick={onClose}>Cancel</button>
          <button className="btn primary" disabled={!name.trim()} onClick={() => run(() => onSave(name, instructions, skillIds))}>
            {project ? "Save" : "Create project"}
          </button>
        </>
      }
    >
      <section className="field">
        <label htmlFor="project-name">Name</label>
        <input
          id="project-name"
          className="input"
          value={name}
          autoFocus
          placeholder="e.g. Thesis research"
          onChange={(e) => setName(e.target.value)}
        />
      </section>
      <section className="field">
        <label htmlFor="project-instructions">Project instructions</label>
        <p className="field-help">Applied to every chat in this project, in addition to your global instructions.</p>
        <textarea
          id="project-instructions"
          className="input"
          rows={8}
          value={instructions}
          placeholder="e.g. You are helping me write a thesis on renewable energy. Cite sections of attached documents."
          onChange={(e) => setInstructions(e.target.value)}
        />
      </section>
      {skills.length > 0 && (
        <section className="field">
          <label id="project-skills-label">Skills always on in this project</label>
          <p className="field-help">Up to {MAX_PROJECT_SKILLS}. They are used in every chat here, on top of what DistroClaw picks.</p>
          <div className="check-list" role="group" aria-labelledby="project-skills-label">
            {skills.map((s) => {
              const on = skillIds.includes(s.id);
              return (
                <label key={s.id} className={`consent ${!on && atMax ? "disabled" : ""}`}>
                  <input type="checkbox" checked={on} disabled={!on && atMax} onChange={() => toggleSkill(s.id)} />
                  <span>
                    {s.title} <span className="mono muted small">{s.slash}</span>
                  </span>
                </label>
              );
            })}
          </div>
        </section>
      )}
      {confirmDelete && (
        <p className="field-help">Chats in this project are not deleted — they move back to Chats.</p>
      )}
      {error && <p className="error-text">{error}</p>}
    </Modal>
  );
}

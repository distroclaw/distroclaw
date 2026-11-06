import { useEffect, useState } from "react";
import { api, Conversation, errorText, SkillSummary } from "../../lib/api";
import { ext } from "../../ext";
import type { SkillPermission } from "../../ext/types";

interface Props {
  /** null writes a new skill; otherwise the raw SKILL.md of this one is edited. */
  skill: SkillSummary | null;
  onBack: () => void;
  onSaved: (skill: SkillSummary) => void;
  /** Opens the test chat the backend created and sends the prompt there. */
  onTest: (conversation: Conversation, prompt: string, skillId: string) => void;
}

/** "What may it do?" groups, mapped to tool names the backend knows. */
const PERMISSIONS: SkillPermission[] = [
  { id: "read", label: "Read files", tools: ["read_file", "file_info"] },
  { id: "search", label: "Search files", tools: ["get_allowed_folders", "list_folder", "search_files"] },
  ...ext.skillPermissions,
];
const PERMISSION_HELP = PERMISSIONS.flatMap((p) => (p.help ? [p.help] : [])).join(" ");

const DESCRIPTION_MIN = 10;
const DESCRIPTION_MAX = 200;
const BODY_MIN = 20;
const BODY_MAX = 2400;

/** Lowercase, non-alphanumerics to "-", trimmed, at most 48 characters. */
export function slugOf(name: string): string {
  return name
    .toLowerCase()
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-+|-+$/g, "")
    .slice(0, 48)
    .replace(/-+$/, "");
}

export function buildSkillMarkdown(input: { name: string; description: string; instructions: string; tools: string[] }): string {
  const slug = slugOf(input.name);
  return [
    "---",
    `name: ${slug}`,
    `title: ${input.name.trim()}`,
    `description: ${input.description.trim()}`,
    "edition: free",
    `tools: [${input.tools.join(", ")}]`,
    "triggers:",
    `  slash: /${slug}`,
    "---",
    input.instructions.trim(),
    "",
  ].join("\n");
}

export function SkillEditor({ skill, onBack, onSaved, onTest }: Props) {
  // New skill: a form. Existing skill: the raw file.
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [instructions, setInstructions] = useState("");
  const [perms, setPerms] = useState<Set<string>>(new Set());
  const [markdown, setMarkdown] = useState<string | null>(skill ? null : "");
  const [loaded, setLoaded] = useState<string | null>(null);
  const [savedId, setSavedId] = useState<string | null>(skill?.id ?? null);
  const [dirty, setDirty] = useState(!skill);
  const [prompt, setPrompt] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!skill) return;
    let alive = true;
    api
      .getSkill(skill.id)
      .then((d) => {
        if (!alive) return;
        setMarkdown(d.markdown);
        setLoaded(d.markdown);
      })
      .catch((e) => alive && setError(errorText(e)));
    return () => {
      alive = false;
    };
  }, [skill]);

  const isNew = !skill;

  const validate = (): string | null => {
    if (!isNew) return markdown?.trim() ? null : "The skill file is empty.";
    if (!name.trim() || slugOf(name).length < 2) return "Give the skill a name.";
    const d = description.trim().length;
    if (d < DESCRIPTION_MIN || d > DESCRIPTION_MAX) return `Describe when to use it in ${DESCRIPTION_MIN} to ${DESCRIPTION_MAX} characters.`;
    const b = instructions.trim().length;
    if (b < BODY_MIN || b > BODY_MAX) return `Instructions need ${BODY_MIN} to ${BODY_MAX} characters.`;
    return null;
  };

  const text = (): string =>
    isNew
      ? buildSkillMarkdown({
          name,
          description,
          instructions,
          tools: PERMISSIONS.filter((p) => perms.has(p.id)).flatMap((p) => p.tools),
        })
      : (markdown ?? "");

  /** Saves when needed and returns the skill id, or null when validation failed. */
  const persist = async (): Promise<string | null> => {
    const problem = validate();
    if (problem) {
      setError(problem);
      return null;
    }
    if (!dirty && savedId) return savedId;
    const saved = await api.saveSkill(savedId, text());
    setSavedId(saved.id);
    setDirty(false);
    if (!isNew) setLoaded(markdown);
    onSaved(saved);
    return saved.id;
  };

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
    } catch (e) {
      setError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  const save = () =>
    run(async () => {
      if (await persist()) onBack();
    });

  const test = () =>
    run(async () => {
      const id = await persist();
      if (!id) return;
      const conversation = await api.testSkill(id, prompt.trim());
      onTest(conversation, prompt.trim(), id);
    });

  const togglePerm = (id: string) => {
    setPerms((s) => {
      const next = new Set(s);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      return next;
    });
    setDirty(true);
  };

  return (
    <>
      <section className="field">
        <div className="label-row">
          <button type="button" className="text-link" onClick={onBack}>
            ‹ Skills
          </button>
          <b>{isNew ? "Write a skill" : `Edit ${skill.title}`}</b>
        </div>
      </section>

      {isNew ? (
        <>
          <section className="field">
            <label htmlFor="skill-name">Name</label>
            <input
              id="skill-name"
              className="input"
              value={name}
              autoFocus
              placeholder="e.g. Meeting minutes"
              onChange={(e) => {
                setName(e.target.value);
                setDirty(true);
              }}
            />
            {slugOf(name) && <p className="field-help mono">/{slugOf(name)}</p>}
          </section>
          <section className="field">
            <label htmlFor="skill-description">When should DistroClaw use it?</label>
            <input
              id="skill-description"
              className="input"
              value={description}
              placeholder="e.g. When I paste meeting notes and ask for minutes or action items."
              maxLength={DESCRIPTION_MAX}
              onChange={(e) => {
                setDescription(e.target.value);
                setDirty(true);
              }}
            />
          </section>
          <section className="field">
            <label htmlFor="skill-instructions">Instructions</label>
            <p className="field-help">Short rules the model follows. Say what to produce and what to ask before guessing.</p>
            <textarea
              id="skill-instructions"
              className="input"
              rows={8}
              value={instructions}
              maxLength={BODY_MAX}
              onChange={(e) => {
                setInstructions(e.target.value);
                setDirty(true);
              }}
            />
          </section>
          <section className="field">
            <label id="skill-perms-label">What may it do?</label>
            <div className="check-list" role="group" aria-labelledby="skill-perms-label">
              {PERMISSIONS.map((p) => (
                <label key={p.id} className="consent">
                  <input type="checkbox" checked={perms.has(p.id)} onChange={() => togglePerm(p.id)} />
                  <span>{p.label}</span>
                </label>
              ))}
            </div>
            {PERMISSION_HELP && <p className="field-help">{PERMISSION_HELP}</p>}
          </section>
        </>
      ) : (
        <section className="field">
          <label htmlFor="skill-markdown">Edit the skill file</label>
          {markdown === null ? (
            <p className="muted small">Loading…</p>
          ) : (
            <textarea
              id="skill-markdown"
              className="input mono"
              rows={16}
              value={markdown}
              spellCheck={false}
              onChange={(e) => {
                setMarkdown(e.target.value);
                setDirty(e.target.value !== loaded);
              }}
            />
          )}
        </section>
      )}

      <section className="field">
        <label htmlFor="skill-test-prompt">Try it</label>
        <div className="key-row">
          <input
            id="skill-test-prompt"
            className="input"
            value={prompt}
            placeholder="A message to test the skill with"
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={(e) => e.key === "Enter" && prompt.trim() && !busy && test()}
          />
          <button type="button" className="btn" disabled={busy || !prompt.trim() || (!isNew && markdown === null)} onClick={test}>
            Run test
          </button>
        </div>
        <p className="field-help">Opens a chat pinned to this skill and sends the message.</p>
      </section>

      {error && <p className="error-text">{error}</p>}

      <div className="label-row">
        <button type="button" className="btn" onClick={onBack} disabled={busy}>
          Cancel
        </button>
        <button type="button" className="btn primary" onClick={save} disabled={busy || (!isNew && markdown === null)}>
          {busy ? "Saving…" : "Save skill"}
        </button>
      </div>
    </>
  );
}

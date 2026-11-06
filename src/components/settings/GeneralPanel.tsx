import { ThemePref } from "../../lib/theme";
import { Toggle } from "../ui/Toggle";

interface Props {
  instructions: string;
  onInstructions: (v: string) => void;
  theme: ThemePref;
  onTheme: (t: ThemePref) => void;
  showSkillChips: boolean;
  onShowSkillChips: (v: boolean) => void;
  thinking: boolean;
  onThinking: (v: boolean) => void;
  webSearch: boolean;
  onWebSearch: (v: boolean) => void;
}

const THEMES: ThemePref[] = ["system", "light", "dark"];

export function GeneralPanel(p: Props) {
  return (
    <>
      <section className="field">
        <label htmlFor="global-instructions">Instructions for all chats</label>
        <p className="field-help">
          Tell DistroClaw how to respond everywhere — tone, language, format. Projects can add their own on top.
        </p>
        <textarea
          id="global-instructions"
          className="input"
          rows={5}
          value={p.instructions}
          placeholder="e.g. Answer concisely. Use British English and metric units."
          onChange={(e) => p.onInstructions(e.target.value)}
        />
      </section>

      <section className="field">
        <label id="theme-label">Theme</label>
        <div className="segmented" role="group" aria-labelledby="theme-label">
          {THEMES.map((t) => (
            <button key={t} type="button" className={p.theme === t ? "on" : ""} aria-pressed={p.theme === t} onClick={() => p.onTheme(t)}>
              {t[0].toUpperCase() + t.slice(1)}
            </button>
          ))}
        </div>
      </section>

      <Toggle
        label="Show what DistroClaw used"
        description="Shows a small note under a reply when a built-in skill was used."
        checked={p.showSkillChips}
        onChange={p.onShowSkillChips}
      />
      <Toggle
        label="Thinking"
        description="Lets the model think before it answers. Faster, shorter replies when off."
        checked={p.thinking}
        onChange={p.onThinking}
      />
      <Toggle
        label="Online search"
        description="Looks things up on the internet when a question needs it. Only search words are sent, never your files."
        checked={p.webSearch}
        onChange={p.onWebSearch}
      />
    </>
  );
}

import { useCallback, useEffect, useRef, useState } from "react";
import { api, Conversation, EngineStatus, errorText } from "../lib/api";
import { previewTheme, ThemePref } from "../lib/theme";
import { Modal } from "./Modal";
import { TabPanel, Tabs } from "./ui/Tabs";
import { GeneralPanel } from "./settings/GeneralPanel";
import { AccessPanel } from "./settings/AccessPanel";
import { SkillsPanel } from "./settings/SkillsPanel";
import { CONTEXT_STOPS, DEFAULT_CONTEXT, ModelPanel } from "./settings/ModelPanel";
import { AboutPanel } from "./settings/AboutPanel";

type Tab = "general" | "access" | "skills" | "model" | "about";

const TABS: { id: Tab; label: string }[] = [
  { id: "general", label: "General" },
  { id: "access", label: "Access" },
  { id: "skills", label: "Skills" },
  { id: "model", label: "Model" },
  { id: "about", label: "About" },
];

interface Props {
  engine: EngineStatus;
  /** The model being switched to, while a switch runs. */
  switchingModel: string | null;
  /** Switches the chat model at once, not on Save. */
  onSwitchModel: (tag: string) => void;
  theme: ThemePref;
  onTheme: (t: ThemePref) => void;
  /** A skill test chat was created: open it and send the prompt there. */
  onTestSkill: (conversation: Conversation, prompt: string, skillId: string) => void;
  /** Called after Save or after an Access change, so the workspace can re-read settings. */
  onChanged?: () => void;
  onClose: () => void;
  initialTab?: Tab;
}

const onOff = (v: boolean) => (v ? "on" : "off");

export function SettingsDialog({ engine, switchingModel, onSwitchModel, theme, onTheme, onTestSkill, onChanged, onClose, initialTab = "general" }: Props) {
  const [tab, setTab] = useState<Tab>(initialTab);
  const [instructions, setInstructions] = useState("");
  const [contextIndex, setContextIndex] = useState(CONTEXT_STOPS.indexOf(DEFAULT_CONTEXT));
  const [showSkillChips, setShowSkillChips] = useState(false);
  const [thinking, setThinking] = useState(true);
  /** Value when the dialog opened; `thinking` is only written when it changed here, so the composer's Think pill is not overwritten on Save. */
  const loadedThinking = useRef(true);
  const [webSearch, setWebSearch] = useState(false);
  /** Same as `loadedThinking`, for the composer's Search pill. Online search is off unless set to "on". */
  const loadedWebSearch = useRef(false);
  const [loaded, setLoaded] = useState(false);
  const [loadError, setLoadError] = useState<string | null>(null);
  const [error, setError] = useState<string | null>(null);
  // The theme is previewed while the dialog is open and only saved on Save.
  const [themeDraft, setThemeDraft] = useState(theme);
  const committedTheme = useRef(theme);

  useEffect(() => {
    committedTheme.current = theme;
  }, [theme]);
  useEffect(() => () => previewTheme(committedTheme.current), []);

  const load = useCallback(() => {
    setLoadError(null);
    Promise.all([
      api.getSetting("instructions"),
      api.getSetting("context_length"),
      api.getSetting("show_skill_chips"),
      api.getSetting("thinking"),
      api.getSetting("web_search"),
    ])
      .then(([ins, ctx, chips, think, search]) => {
        setInstructions(ins ?? "");
        const i = CONTEXT_STOPS.indexOf(Number(ctx));
        if (i >= 0) setContextIndex(i);
        setShowSkillChips(chips === "on");
        loadedThinking.current = think !== "off";
        setThinking(loadedThinking.current);
        loadedWebSearch.current = search === "on";
        setWebSearch(loadedWebSearch.current);
        setLoaded(true);
      })
      .catch((e) => setLoadError(errorText(e)));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const pickTheme = (t: ThemePref) => {
    setThemeDraft(t);
    previewTheme(t);
  };

  const cancel = () => {
    previewTheme(committedTheme.current);
    onClose();
  };

  const save = async () => {
    try {
      await api.setSetting("instructions", instructions.trim());
      await api.setSetting("context_length", String(CONTEXT_STOPS[contextIndex]));
      await api.setSetting("show_skill_chips", onOff(showSkillChips));
      if (thinking !== loadedThinking.current) {
        await api.setSetting("thinking", onOff(thinking));
        loadedThinking.current = thinking;
      }
      if (webSearch !== loadedWebSearch.current) {
        await api.setSetting("web_search", onOff(webSearch));
        loadedWebSearch.current = webSearch;
      }
      if (themeDraft !== committedTheme.current) {
        committedTheme.current = themeDraft;
        onTheme(themeDraft);
      }
      onChanged?.();
      onClose();
    } catch (e) {
      setError(errorText(e));
    }
  };

  return (
    <Modal
      title="Settings"
      wide
      onClose={cancel}
      footer={
        <>
          <button className="btn" onClick={cancel}>
            Cancel
          </button>
          <button className="btn primary" onClick={save} disabled={!loaded}>
            Save
          </button>
        </>
      }
    >
      <Tabs tabs={TABS} value={tab} onChange={(id) => setTab(id as Tab)} label="Settings sections" />

      {loadError && (
        <div className="error-banner">
          {loadError}{" "}
          <button className="text-link" onClick={load}>
            Try again
          </button>
        </div>
      )}

      <TabPanel id="general" active={tab === "general"}>
        <GeneralPanel
          instructions={instructions}
          onInstructions={setInstructions}
          theme={themeDraft}
          onTheme={pickTheme}
          showSkillChips={showSkillChips}
          onShowSkillChips={setShowSkillChips}
          thinking={thinking}
          onThinking={setThinking}
          webSearch={webSearch}
          onWebSearch={setWebSearch}
        />
      </TabPanel>
      <TabPanel id="access" active={tab === "access"}>
        <AccessPanel onChanged={onChanged} />
      </TabPanel>
      <TabPanel id="skills" active={tab === "skills"}>
        <SkillsPanel onTest={onTestSkill} onChanged={onChanged} />
      </TabPanel>
      <TabPanel id="model" active={tab === "model"}>
        <ModelPanel
          engine={engine}
          switchingModel={switchingModel}
          onSwitchModel={onSwitchModel}
          contextIndex={contextIndex}
          onContextIndex={setContextIndex}
        />
      </TabPanel>
      <TabPanel id="about" active={tab === "about"}>
        <AboutPanel />
      </TabPanel>

      {error && <p className="error-text">{error}</p>}
    </Modal>
  );
}

export type SettingsTab = Tab;

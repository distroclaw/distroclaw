import { useCallback, useEffect, useMemo, useRef, useState } from "react";
import { AllowedFolder, api, Attachment, Conversation, EngineStatus, errorText, Project, SendOptions, SkillSummary } from "../lib/api";
import { ThemePref } from "../lib/theme";
import { useThreads } from "../lib/useThreads";
import { useDrafts } from "../lib/drafts";
import { ext } from "../ext";
import type { WorkspaceHandle } from "../ext/types";
import { Sidebar } from "./Sidebar";
import { ChatView, SendPins } from "./ChatView";
import { SettingsDialog, SettingsTab } from "./SettingsDialog";
import { ProjectDialog } from "./ProjectDialog";
import { useToast } from "./ui/Toast";

interface WorkspaceProps {
  engine: EngineStatus;
  onEngineRetry: () => void;
  /** Shows a status a command returned (after switching models) without waiting for the next poll. */
  onEngine: (s: EngineStatus) => void;
  theme: ThemePref;
  setTheme: (t: ThemePref) => void;
}

export function Workspace({ engine, onEngineRetry, onEngine, theme, setTheme }: WorkspaceProps) {
  const [conversations, setConversations] = useState<Conversation[]>([]);
  const [projects, setProjects] = useState<Project[]>([]);
  // undefined = closed, otherwise the tab the dialog opens on
  const [settingsTab, setSettingsTab] = useState<SettingsTab | undefined>(undefined);
  /** The sidebar area whose view replaces the chat (see `ext.areaViews`); null shows the chat. */
  const [areaView, setAreaView] = useState<string | null>(null);
  /** Area chosen in the sidebar; sent with the next message in that conversation. */
  const [pendingArea, setPendingArea] = useState<{ id: string; area: string } | null>(null);
  const [folders, setFolders] = useState<AllowedFolder[]>([]);
  const [skills, setSkills] = useState<SkillSummary[]>([]);
  const [showSkillChips, setShowSkillChips] = useState(false);
  /** The "thinking" user setting, shown as the Think pill in the composer; null in the store means on. */
  const [thinking, setThinking] = useState(true);
  /** The "web_search" user setting, shown as the Search pill in the composer; off unless set to "on". */
  const [webSearch, setWebSearch] = useState(false);
  // undefined = closed, null = creating a new project
  const [editingProject, setEditingProject] = useState<Project | null | undefined>(undefined);
  const [activeId, setActiveId] = useState<string | null>(null);
  /** The model being switched to while `switchModel` runs; the header badge and Settings → Model show it as busy. */
  const [switchingModel, setSwitchingModel] = useState<string | null>(null);
  const switching = useRef(false);
  const toast = useToast();

  const refreshList = useCallback(async () => {
    const list = await api.listConversations();
    setConversations(list);
    return list;
  }, []);

  const refreshProjects = useCallback(async () => setProjects(await api.listProjects()), []);

  const refreshFolders = useCallback(
    () =>
      api
        .listAllowedFolders()
        .then(setFolders)
        .catch(() => {}),
    [],
  );

  const refreshPrefs = useCallback(
    () =>
      Promise.all([
        api.getSetting("show_skill_chips").catch(() => null),
        api.getSetting("thinking").catch(() => null),
        api.getSetting("web_search").catch(() => null),
      ]).then(([chips, think, search]) => {
        setShowSkillChips(chips === "on");
        setThinking(think !== "off");
        setWebSearch(search === "on");
      }),
    [],
  );

  const refreshSkills = useCallback(
    () =>
      api
        .listSkills()
        .then(setSkills)
        .catch(() => {}),
    [],
  );

  useEffect(() => {
    refreshFolders();
    refreshPrefs();
    refreshSkills();
  }, [refreshFolders, refreshPrefs, refreshSkills]);

  const { threads, live, errors, load, reload, send: sendTo, stream, forget } = useThreads(refreshList);
  const drafts = useDrafts();

  useEffect(() => {
    refreshProjects();
    refreshList().then((list) => list[0] && setActiveId(list[0].id));
  }, [refreshList, refreshProjects]);

  useEffect(() => {
    if (activeId) load(activeId);
  }, [activeId, load]);

  const select = (id: string) => {
    setAreaView(null);
    setActiveId(id);
  };

  const newChat = async (projectId: string | null = null): Promise<string> => {
    setAreaView(null);
    // Reuse an untouched "New chat" in the same place instead of adding another row.
    const empty = conversations.find(
      (c) => c.projectId === projectId && c.title === "New chat" && !live[c.id] && (threads[c.id]?.length ?? 0) === 0,
    );
    if (empty) {
      setActiveId(empty.id);
      return empty.id;
    }
    const c = await api.createConversation(projectId);
    await refreshList();
    setActiveId(c.id);
    return c.id;
  };

  const openArea = async (area: string) => {
    if (ext.areaViews[area]) {
      setAreaView(area);
      return;
    }
    const id = await newChat(null);
    setPendingArea({ id, area });
  };

  const send = async (text: string, attachments: Attachment[], pins: SendPins = {}) => {
    let id = activeId;
    if (!id) {
      const c = await api.createConversation(null);
      id = c.id;
      setActiveId(id);
    }
    const options: SendOptions = { ...pins };
    if (pendingArea && pendingArea.id === id) {
      options.area = pendingArea.area;
      setPendingArea(null);
    }
    const sending = sendTo(id, text, attachments, options);
    refreshList();
    await sending;
  };

  const fail = (e: unknown) => toast(errorText(e), { kind: "error" });

  /** The Think pill: flips at once, is saved for every chat, and flips back if saving fails. */
  const toggleThinking = async () => {
    const next = !thinking;
    setThinking(next);
    try {
      await api.setSetting("thinking", next ? "on" : "off");
    } catch (e) {
      setThinking(!next);
      fail(e);
    }
  };

  /** The Search pill: flips at once, is saved for every chat, and flips back if saving fails. */
  const toggleWebSearch = async () => {
    const next = !webSearch;
    setWebSearch(next);
    try {
      await api.setSetting("web_search", next ? "on" : "off");
    } catch (e) {
      setWebSearch(!next);
      fail(e);
    }
  };

  /** Switches the chat model for the whole app; the header shows the new model as soon as it returns. */
  const switchModel = async (tag: string) => {
    if (switching.current || tag === engine.model) return;
    switching.current = true;
    setSwitchingModel(tag);
    try {
      onEngine(await api.switchModel(tag));
    } catch (e) {
      fail(e);
    } finally {
      switching.current = false;
      setSwitchingModel(null);
    }
  };

  /** What extension views and message addons may do in the workspace. */
  const workspace: WorkspaceHandle = {
    conversations,
    openChat: select,
    isLive: (id) => !!live[id],
    reloadChat: async (id) => {
      if (threads[id] && !live[id]) await reload(id);
    },
    streamReply: (id, run) => {
      const streaming = stream(id, run);
      refreshList();
      void streaming;
    },
  };

  const rename = async (id: string, title: string) => {
    await api.renameConversation(id, title).catch(fail);
    refreshList();
  };

  const remove = async (id: string) => {
    try {
      await api.deleteConversation(id);
    } catch (e) {
      fail(e);
      return;
    }
    forget(id);
    const list = await refreshList();
    if (id === activeId) setActiveId(list[0]?.id ?? null);
  };

  const move = async (id: string, projectId: string | null) => {
    await api.moveConversation(id, projectId).catch(fail);
    refreshList();
  };

  /** "Try it" in the skill editor: open the test chat and send the prompt pinned to the skill. */
  const testSkill = (conversation: Conversation, prompt: string, skillId: string) => {
    setSettingsTab(undefined);
    setConversations((xs) => (xs.some((c) => c.id === conversation.id) ? xs : [conversation, ...xs]));
    select(conversation.id);
    const sending = sendTo(conversation.id, prompt, [], { skillId });
    refreshList();
    void sending;
  };

  const saveProject = async (name: string, instructions: string, skillIds: string[]) => {
    if (editingProject) {
      await api.updateProject(editingProject.id, name, instructions, skillIds);
    } else {
      const p = await api.createProject(name, instructions, skillIds);
      const c = await api.createConversation(p.id);
      setActiveId(c.id);
    }
    await Promise.all([refreshProjects(), refreshList()]);
  };

  const deleteProject = async () => {
    if (!editingProject) return;
    await api.deleteProject(editingProject.id);
    await Promise.all([refreshProjects(), refreshList()]);
  };

  const active = conversations.find((c) => c.id === activeId) ?? null;
  const activeProject = projects.find((p) => p.id === active?.projectId) ?? null;
  const liveIds = useMemo(() => new Set(Object.keys(live)), [live]);
  const activeThread = activeId ? threads[activeId] : undefined;
  const activeLive = activeId ? (live[activeId] ?? null) : null;
  const activeError = activeId ? (errors[activeId] ?? null) : null;
  const activeArea = areaView ?? (pendingArea && pendingArea.id === activeId ? pendingArea.area : null);
  const AreaView = areaView ? ext.areaViews[areaView] : undefined;

  return (
    <div className="app">
      <Sidebar
        conversations={conversations}
        projects={projects}
        activeId={activeId}
        liveIds={liveIds}
        onSelect={select}
        onNew={newChat}
        onRename={rename}
        onDelete={remove}
        onMove={move}
        onNewProject={() => setEditingProject(null)}
        onEditProject={setEditingProject}
        onOpenSettings={(tab = "general") => setSettingsTab(tab)}
        onOpenArea={openArea}
        activeArea={activeArea}
        folders={folders}
      />
      {AreaView ? (
        <AreaView workspace={workspace} />
      ) : (
        <ChatView
          conversation={active}
          project={activeProject}
          onOpenProject={() => activeProject && setEditingProject(activeProject)}
          messages={activeThread ?? []}
          loading={!!activeId && !activeThread}
          live={activeLive}
          error={activeError}
          engine={engine}
          onEngineRetry={onEngineRetry}
          switchingModel={switchingModel}
          onSwitchModel={switchModel}
          drafts={drafts}
          showSkillChips={showSkillChips}
          skills={skills}
          folders={folders}
          thinking={thinking}
          onToggleThinking={toggleThinking}
          webSearch={webSearch}
          onToggleWebSearch={toggleWebSearch}
          onSend={send}
          onStop={() => activeId && api.stopGeneration(activeId).catch(() => {})}
          workspace={workspace}
        />
      )}
      {settingsTab !== undefined && (
        <SettingsDialog
          engine={engine}
          switchingModel={switchingModel}
          onSwitchModel={switchModel}
          theme={theme}
          onTheme={setTheme}
          initialTab={settingsTab}
          onChanged={() => {
            refreshFolders();
            refreshPrefs();
            refreshSkills();
          }}
          onTestSkill={testSkill}
          onClose={() => setSettingsTab(undefined)}
        />
      )}
      {editingProject !== undefined && (
        <ProjectDialog
          project={editingProject}
          chatCount={editingProject ? conversations.filter((c) => c.projectId === editingProject.id).length : 0}
          onSave={saveProject}
          onDelete={editingProject ? deleteProject : undefined}
          onClose={() => setEditingProject(undefined)}
        />
      )}
    </div>
  );
}

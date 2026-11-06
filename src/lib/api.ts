import { Channel, invoke } from "@tauri-apps/api/core";

export interface EngineStatus {
  state: "starting" | "preparing" | "needs_model" | "ready" | "error";
  message: string;
  progress: number;
  mode: "external" | "bundled";
  baseUrl: string;
  model: string;
  embedModel: string;
  embedReady: boolean;
  /** What the chat model can do, e.g. "completion", "vision", "tools", "thinking"; empty when unknown. */
  capabilities: string[];
}

/** A chat model installed in the engine (embedding models are left out). */
export interface InstalledModel {
  name: string;
  sizeBytes: number;
  capabilities: string[];
  /** True for the model the app chats with now. */
  current: boolean;
}

export interface Gpu {
  name: string;
  kind: "apple" | "nvidia" | "amd" | "intel" | "other";
  vramBytes: number | null;
}

export interface HardwareReport {
  os: string;
  arch: string;
  cpu: string;
  cores: number;
  totalMemoryBytes: number;
  availableMemoryBytes: number;
  gpus: Gpu[];
  freeDiskBytes: number;
}

export interface ModelOption {
  tag: string;
  label: string;
  downloadBytes: number;
  minBudgetBytes: number;
  description: string;
  fitsMemory: boolean;
  fitsDisk: boolean;
}

export interface Recommendation {
  tag: string;
  reason: string;
  budgetBytes: number;
  options: ModelOption[];
}

export interface DownloadStatus {
  state: "idle" | "downloading" | "done" | "error" | "cancelled";
  tag: string;
  completed: number;
  total: number;
  message: string;
}

export interface SetupInfo {
  hardware: HardwareReport;
  recommendation: Recommendation;
  installed: string[];
  chatModel: string;
  download: DownloadStatus;
}

export interface Conversation {
  id: string;
  title: string;
  projectId: string | null;
  createdAt: number;
  updatedAt: number;
  /** Skill this chat is pinned to (set by "Try it"); null for normal chats. */
  skillId: string | null;
}

export interface Project {
  id: string;
  name: string;
  instructions: string;
  createdAt: number;
  updatedAt: number;
  /** Skills always on in this project (at most 2). */
  skillIds: string[];
}

export interface Attachment {
  name: string;
  kind: "image" | "document";
  mime: string;
  size: number;
  /** Base64 image data (images only). */
  data?: string;
  /** Extracted text (documents only). */
  text?: string;
  truncated: boolean;
}

export interface ExtractedText {
  text: string;
  truncated: boolean;
}

/* ---------- errors ---------- */

export type ErrorCode =
  | "unauthorized"
  | "forbidden"
  | "notFound"
  | "invalid"
  | "busy"
  | "accessDenied"
  | "internal";

export interface AppErrorShape {
  code: ErrorCode;
  message: string;
}

const isErrorShape = (e: unknown): e is AppErrorShape =>
  typeof e === "object" &&
  e !== null &&
  typeof (e as { code?: unknown }).code === "string" &&
  typeof (e as { message?: unknown }).message === "string";

/** Accepts the structured backend error, a legacy string, or a thrown Error. */
export function errorOf(e: unknown): AppErrorShape {
  if (isErrorShape(e)) return e;
  if (typeof e === "string") return { code: "internal", message: e };
  if (e instanceof Error) return { code: "internal", message: e.message };
  return { code: "internal", message: String(e) };
}

export function errorText(e: unknown): string {
  return errorOf(e).message;
}

/* ---------- chat events ---------- */

/** A web page found by online search; the reply cites it as `[n]`. */
export interface Source {
  n: number;
  title: string;
  url: string;
}

export type ChatEvent =
  | { type: "thinking"; delta: string }
  | { type: "token"; delta: string }
  /** Online search is running; comes before any token. */
  | { type: "searching" }
  /** Online search finished; empty when the search failed. Saved with the reply. */
  | { type: "sources"; sources: Source[] }
  | { type: "skill"; id: string; title: string }
  | { type: "toolCall"; id: string; name: string; args: Record<string, unknown> }
  | { type: "toolResult"; id: string; ok: boolean; summary: string }
  /** The reply so far was saved mid-send; what follows streams into a new bubble. */
  | { type: "messageSaved"; message: Message }
  | { type: "error"; message: string };

/** A streamed event the core does not handle itself; kept with the live reply for the extension's message addon. */
export interface ExtEvent {
  type: string;
  [key: string]: unknown;
}

export type ToolEvent = Extract<ChatEvent, { type: "toolCall" | "toolResult" | "skill" | "sources" }>;

export interface Message {
  id: string;
  conversationId: string;
  role: "user" | "assistant" | "system";
  content: string;
  thinking: string;
  attachments: Attachment[];
  createdAt: number;
  toolEvents: ToolEvent[];
}

/** Fills in fields older rows (or an older backend) may leave out. */
export function normalizeMessage(m: Partial<Message> & Pick<Message, "id" | "conversationId" | "role" | "content">): Message {
  return {
    ...m,
    thinking: m.thinking ?? "",
    attachments: m.attachments ?? [],
    createdAt: m.createdAt ?? 0,
    toolEvents: Array.isArray(m.toolEvents) ? m.toolEvents : [],
  };
}

export interface SendOptions {
  /** The sidebar area the chat was opened from, e.g. "documents". */
  area?: string;
  skillId?: string;
  folderIds?: string[];
}

/* ---------- skills ---------- */

export interface SkillSummary {
  id: string;
  title: string;
  description: string;
  source: "builtin" | "user";
  enabled: boolean;
  /** False when this edition of the app cannot run the skill. */
  available: boolean;
  /** Slash command, e.g. "/organize". */
  slash: string;
  tools: string[];
  /** Set when the skill file could not be parsed. */
  error?: string;
}

export interface SkillDetail {
  summary: SkillSummary;
  markdown: string;
  path?: string;
}

/* ---------- folder access ---------- */

export type FolderMode = "read" | "readwrite";

export interface AllowedFolder {
  id: string;
  path: string;
  label: string;
  mode: FolderMode;
  addedAt: number;
  fileCount: number;
  available: boolean;
}

export const api = {
  setupInfo: () => invoke<SetupInfo>("setup_info"),
  startModelDownload: (tag: string, consent: boolean) => invoke<void>("start_model_download", { tag, consent }),
  cancelModelDownload: () => invoke<void>("cancel_model_download"),
  downloadStatus: () => invoke<DownloadStatus>("download_status"),
  engineStatus: () => invoke<EngineStatus>("engine_status"),
  engineRetry: () => invoke<void>("engine_retry"),
  /** Installed chat models, sorted by name. */
  listChatModels: () => invoke<InstalledModel[]>("list_chat_models"),
  /** Switches the chat model for the whole app and returns the new status; the engine keeps running. */
  switchModel: (tag: string) => invoke<EngineStatus>("switch_model", { tag }),
  /** Deletes an installed model; without the model in use, the app goes back to setup. */
  removeModel: (tag: string) => invoke<EngineStatus>("remove_model", { tag }),
  /** The license an installed model was published with; null when it has none. */
  modelLicense: (tag: string) => invoke<string | null>("model_license", { tag }),
  /** Shows the app's data folder (models, chats, logs) in Finder or Explorer. */
  revealDataFolder: () => invoke<void>("reveal_data_folder"),
  getSetting: (key: string) => invoke<string | null>("get_setting", { key }),
  setSetting: (key: string, value: string) => invoke<void>("set_setting", { key, value }),
  listConversations: () => invoke<Conversation[]>("list_conversations"),
  createConversation: (projectId: string | null = null) =>
    invoke<Conversation>("create_conversation", { projectId }),
  moveConversation: (id: string, projectId: string | null) =>
    invoke<void>("move_conversation", { id, projectId }),
  listProjects: () => invoke<Project[]>("list_projects"),
  createProject: (name: string, instructions: string, skillIds: string[] = []) =>
    invoke<Project>("create_project", { name, instructions, skillIds }),
  updateProject: (id: string, name: string, instructions: string, skillIds: string[] = []) =>
    invoke<void>("update_project", { id, name, instructions, skillIds }),
  deleteProject: (id: string) => invoke<void>("delete_project", { id }),
  renameConversation: (id: string, title: string) =>
    invoke<void>("rename_conversation", { id, title }),
  deleteConversation: (id: string) => invoke<void>("delete_conversation", { id }),
  getMessages: async (conversationId: string) =>
    (await invoke<Message[]>("get_messages", { conversationId })).map(normalizeMessage),
  stopGeneration: (conversationId: string) => invoke<void>("stop_generation", { conversationId }),
  sendMessage: (
    conversationId: string,
    text: string,
    attachments: Attachment[],
    options: SendOptions = {},
    onEvent: (e: ChatEvent) => void = () => {},
  ) => {
    const channel = new Channel<ChatEvent>();
    channel.onmessage = onEvent;
    return invoke<Message | null>("send_message", {
      conversationId,
      text,
      attachments,
      options,
      onEvent: channel,
    }).then((m) => (m ? normalizeMessage(m) : m));
  },
  /** File bytes go as the raw IPC body to avoid JSON-encoding large files. */
  readDocument: async (file: File) =>
    invoke<ExtractedText>("read_document", new Uint8Array(await file.arrayBuffer()), {
      headers: { "x-file-name": encodeURIComponent(file.name) },
    }),
  /** Rust opens the native picker; null means the user cancelled. */
  pickFolder: () => invoke<AllowedFolder | null>("pick_folder"),
  listAllowedFolders: () => invoke<AllowedFolder[]>("list_allowed_folders"),
  removeAllowedFolder: (id: string) => invoke<void>("remove_allowed_folder", { id }),
  updateAllowedFolder: (id: string, label: string, mode: FolderMode) =>
    invoke<AllowedFolder>("update_allowed_folder", { id, label, mode }),

  /** Shows the file in Finder / Explorer. */
  revealPath: (path: string) => invoke<void>("reveal_path", { path }),

  /* skills */
  listSkills: () => invoke<SkillSummary[]>("list_skills"),
  getSkill: (id: string) => invoke<SkillDetail>("get_skill", { id }),
  /** `id` null creates a new skill from the SKILL.md text. */
  saveSkill: (id: string | null, markdown: string) => invoke<SkillSummary>("save_skill", { id, markdown }),
  /** Rust opens the native picker; null means the user cancelled. */
  importSkill: () => invoke<SkillSummary | null>("import_skill"),
  deleteSkill: (id: string) => invoke<void>("delete_skill", { id }),
  setSkillEnabled: (id: string, enabled: boolean) => invoke<void>("set_skill_enabled", { id, enabled }),
  /** Creates a chat pinned to the skill; the caller then sends `prompt` through the normal path. */
  testSkill: (id: string, prompt: string) => invoke<Conversation>("test_skill", { id, prompt }),
};

export function formatBytes(bytes: number): string {
  const gb = bytes / 1024 ** 3;
  return gb >= 1 ? `${gb.toFixed(1)} GB` : `${Math.round(bytes / 1024 ** 2)} MB`;
}

/** Always in GB with one decimal, e.g. "2.3 GB"; same unit as `formatBytes`. */
export function formatGb(bytes: number): string {
  return `${(bytes / 1024 ** 3).toFixed(1)} GB`;
}

import type { ComponentType, ReactNode } from "react";
import type { AllowedFolder, ChatEvent, Conversation, ExtEvent, FolderMode, Message } from "../lib/api";

/*
 * Extension points the core renders. `./index.tsx` fills them; an edition that needs none
 * leaves the optional ones out and the lists empty, and the app works the same without them.
 */

/** What the workspace lends to extension views and message addons. */
export interface WorkspaceHandle {
  conversations: Conversation[];
  openChat: (id: string) => void;
  /** True while a reply streams into the chat. */
  isLive: (id: string) => boolean;
  /** Re-reads a chat's messages if they are loaded and no reply is streaming into it. */
  reloadChat: (id: string) => Promise<void>;
  /** Streams a reply into a chat the way sending does; `run` hands the event handler to the backend. */
  streamReply: (id: string, run: (onEvent: (e: ChatEvent) => void) => Promise<unknown>) => void;
}

export interface SidebarAreasProps {
  /** The open area, if any. */
  active: string | null;
  /** Opens an area: its view from `areaViews`, or else a new chat that sends with the area's id. */
  onOpen: (id: string) => void;
}

export interface MessageAddonProps {
  /** The saved message; null for the reply that is streaming. */
  message: Message | null;
  /** Streamed events the core does not handle, oldest first; empty for saved messages. */
  events: ExtEvent[];
  workspace: WorkspaceHandle;
}

export interface FolderControlProps {
  folder: AllowedFolder;
  onMode: (mode: FolderMode) => void;
}

/** A choice under "What may it do?" in the skill editor. */
export interface SkillPermission {
  id: string;
  label: string;
  /** Tool names the backend knows. */
  tools: string[];
  /** Shown under the choices. */
  help?: string;
}

export interface Extension {
  /** Wraps the app; may show its own screen before the workspace. */
  Gate: ComponentType<{ children: ReactNode }>;

  /** Edition name in Settings → About. */
  editionName: string;
  /** More about the edition in Settings → About. */
  EditionInfo?: ComponentType;

  /** Sidebar areas listed after Documents. */
  SidebarAreas?: ComponentType<SidebarAreasProps>;
  /** Area views shown in place of the chat, by area id. */
  areaViews: Record<string, ComponentType<{ workspace: WorkspaceHandle }>>;
  /** Replaces the folder summary under the sidebar areas; `children` is the core's own. */
  WorkspaceStatus?: ComponentType<{ folders: AllowedFolder[]; children: ReactNode }>;

  /** Shown in each assistant message, after its sources. */
  MessageAddon?: ComponentType<MessageAddonProps>;
  /** Plain names for tools the core does not describe, e.g. `{ my_tool: "Did my thing" }`. */
  toolLabels: Record<string, string>;
  /** Choices added under "What may it do?" in the skill editor. */
  skillPermissions: SkillPermission[];

  /** Settings → Access, above the folders. */
  AccessSection?: ComponentType<{ onChanged?: () => void }>;
  /** Replaces the help text above the folder list. */
  FolderHelp?: ComponentType;
  /** A control in each folder row, between its name and Remove. */
  FolderControl?: ComponentType<FolderControlProps>;
}

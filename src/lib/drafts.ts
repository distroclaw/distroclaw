import { useCallback, useState } from "react";
import { PendingAttachment } from "./attachments";

/** Unsent composer text and attachments, kept per conversation so switching chats never mixes them. */
export interface Draft {
  text: string;
  files: PendingAttachment[];
  /** Skill pinned for the next message (from "/" or the + menu). */
  skillId: string | null;
  /** Folders pinned for the next message. */
  folderIds: string[];
}

/** Key for a chat that has no conversation row yet. */
export const NEW_DRAFT_KEY = "new";

export const draftKey = (conversationId: string | null | undefined) => conversationId ?? NEW_DRAFT_KEY;

const EMPTY: Draft = { text: "", files: [], skillId: null, folderIds: [] };

export type FilesUpdater = (files: PendingAttachment[]) => PendingAttachment[];

export function useDrafts() {
  const [drafts, setDrafts] = useState<Record<string, Draft>>({});

  const get = useCallback((key: string): Draft => drafts[key] ?? EMPTY, [drafts]);

  const setText = useCallback((key: string, text: string) => {
    setDrafts((d) => ({ ...d, [key]: { ...(d[key] ?? EMPTY), text } }));
  }, []);

  const setFiles = useCallback((key: string, fn: FilesUpdater) => {
    setDrafts((d) => {
      const cur = d[key] ?? EMPTY;
      return { ...d, [key]: { ...cur, files: fn(cur.files) } };
    });
  }, []);

  const setSkill = useCallback((key: string, skillId: string | null) => {
    setDrafts((d) => ({ ...d, [key]: { ...(d[key] ?? EMPTY), skillId } }));
  }, []);

  const setFolders = useCallback((key: string, fn: (ids: string[]) => string[]) => {
    setDrafts((d) => {
      const cur = d[key] ?? EMPTY;
      return { ...d, [key]: { ...cur, folderIds: fn(cur.folderIds) } };
    });
  }, []);

  const clear = useCallback((key: string) => {
    setDrafts((d) => {
      if (!(key in d)) return d;
      const next = { ...d };
      delete next[key];
      return next;
    });
  }, []);

  return { get, setText, setFiles, setSkill, setFolders, clear };
}

export type Drafts = ReturnType<typeof useDrafts>;

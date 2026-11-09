import { ToolEvent } from "../../lib/api";
import { LiveEvent } from "../../lib/useThreads";
import { ext } from "../../ext";
import { CheckIcon, XIcon } from "../Icons";

/** Events from a saved message or from a reply still streaming. */
export type ReplyEvent = ToolEvent | LiveEvent;

const basename = (v: unknown): string => {
  if (typeof v !== "string") return "";
  const trimmed = v.replace(/[\\/]+$/, "");
  return trimmed.split(/[\\/]/).pop() || trimmed;
};

const str = (v: unknown): string => (typeof v === "string" ? v : "");

/** The site an address points to, without a leading "www."; empty when it does not parse. */
const domainOf = (v: unknown): string => {
  try {
    return new URL(str(v)).hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
};

/** One plain sentence per tool call; raw arguments never show outside the expanded body. */
export function describeTool(name: string, args: Record<string, unknown>): string {
  switch (name) {
    case "list_folder":
      return `Looked at ${basename(args.path ?? args.folder) || "a folder"}`;
    case "search_files": {
      const q = str(args.name) || str(args.extension);
      return q ? `Searched for “${q}”` : "Searched files";
    }
    case "read_file":
      return `Read ${basename(args.path) || "a file"}`;
    case "file_info":
      return `Checked ${basename(args.path) || "a file"}`;
    case "get_allowed_folders":
      return "Checked allowed folders";
    case "web_search":
      return str(args.query) ? `Searched the web for “${str(args.query)}”` : "Searched the web";
    case "read_web_page":
      return `Read ${domainOf(args.url) || "a web page"}`;
    default: {
      const label = ext.toolLabels[name];
      return typeof label === "string" ? label : name.replace(/_/g, " ");
    }
  }
}

interface Row {
  id: string;
  title: string;
  ok?: boolean;
  summary?: string;
}

/** Pairs each toolCall with its toolResult by id, keeping call order. */
export function toolRows(events: ReplyEvent[]): Row[] {
  const rows: Row[] = [];
  const byId = new Map<string, Row>();
  for (const e of events) {
    if (e.type === "toolCall") {
      const row: Row = { id: e.id, title: describeTool(e.name, e.args ?? {}) };
      byId.set(e.id, row);
      rows.push(row);
    } else if (e.type === "toolResult") {
      const row = byId.get(e.id);
      if (row) {
        row.ok = e.ok;
        row.summary = e.summary;
      } else {
        rows.push({ id: e.id, title: "Tool result", ok: e.ok, summary: e.summary });
      }
    }
  }
  return rows;
}

function ToolRow({ row }: { row: Row }) {
  const state = row.ok === undefined ? "" : row.ok ? "ok" : "failed";
  return (
    <details className={`tool-row ${state}`}>
      <summary>
        {row.ok === undefined ? (
          <span className="spinner" aria-hidden="true" />
        ) : row.ok ? (
          <CheckIcon />
        ) : (
          <XIcon />
        )}
        <span className="sr-only">{row.ok === undefined ? "Working: " : row.ok ? "Done: " : "Failed: "}</span>
        <span>{row.title}</span>
      </summary>
      <div className="thinking-body">{row.summary || "Working…"}</div>
    </details>
  );
}

interface ToolCallsProps {
  events: ReplyEvent[];
  /** Open while the reply streams, collapsed once it is saved. */
  live?: boolean;
}

/** Compact list of what DistroClaw looked at while answering. */
export function ToolCalls({ events, live = false }: ToolCallsProps) {
  const rows = toolRows(events);
  if (rows.length === 0) return null;
  const pending = rows.some((r) => r.ok === undefined);
  const label = pending && live ? "Working…" : rows.length === 1 ? "Used 1 tool" : `Used ${rows.length} tools`;
  return (
    <details className="tool-group" open={live}>
      <summary>{label}</summary>
      {rows.map((r) => (
        <ToolRow key={r.id} row={r} />
      ))}
    </details>
  );
}

/** Shown under a reply when "Show what DistroClaw used" is on. */
export function SkillChip({ title }: { title: string }) {
  return <span className="skill-chip">Used: {title}</span>;
}


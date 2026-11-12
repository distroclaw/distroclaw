/** Thousands separated with an apostrophe: 12'345. */
export const formatCount = (n: number) => String(n).replace(/\B(?=(\d{3})+(?!\d))/g, "'");

const two = (n: number) => String(n).padStart(2, "0");

/** 24-hour time then day.month.year: "10:15 24.04.2026". Takes milliseconds since the epoch. */
export function formatDateTime(ms: number): string {
  const d = new Date(ms);
  return `${two(d.getHours())}:${two(d.getMinutes())} ${two(d.getDate())}.${two(d.getMonth() + 1)}.${d.getFullYear()}`;
}

/** Keeps both ends of a long string and drops the middle. */
export function truncateMiddle(s: string, max = 52): string {
  if (s.length <= max) return s;
  const head = Math.ceil((max - 1) / 2);
  const tail = max - 1 - head;
  return `${s.slice(0, head)}…${s.slice(s.length - tail)}`;
}

/** Last path segment, with a trailing separator ignored. */
export function basename(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  return trimmed.split(/[\\/]/).pop() || trimmed;
}

/** Everything before the last separator; "" when there is none. */
export function dirname(path: string): string {
  const trimmed = path.replace(/[\\/]+$/, "");
  const i = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  return i <= 0 ? (i === 0 ? "/" : "") : trimmed.slice(0, i);
}

/** The separator a path uses: "\" for Windows paths (C:\Users\…), "/" otherwise. */
const separator = (path: string) => (path.includes("\\") && !path.includes("/") ? "\\" : "/");

/** The deepest folder that contains every path; "" when they share nothing useful. */
export function commonParent(paths: string[]): string {
  if (paths.length === 0) return "";
  const split = paths.map((p) => dirname(p).split(/[\\/]/));
  const first = split[0];
  let n = first.length;
  for (const parts of split.slice(1)) {
    let i = 0;
    while (i < n && i < parts.length && parts[i] === first[i]) i += 1;
    n = i;
  }
  // Fewer than two real segments (e.g. "/" or "/Users") is not worth hiding.
  const kept = first.slice(0, n);
  if (kept.filter(Boolean).length < 2) return "";
  return kept.join(separator(paths[0])) || "/";
}

/** `path` relative to `parent`, or the full path when it is not inside. */
export function relativeTo(path: string, parent: string): string {
  if (!parent) return path;
  const sep = separator(parent);
  const prefix = parent.endsWith(sep) ? parent : `${parent}${sep}`;
  return path.startsWith(prefix) ? path.slice(prefix.length) : path;
}

/** An http or https address, the only kind the app opens in the browser. */
export const isWebUrl = (url: string) => /^https?:\/\//i.test(url);

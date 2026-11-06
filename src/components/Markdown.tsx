import { ReactNode, isValidElement, memo, useEffect, useMemo, useRef, useState } from "react";
import ReactMarkdown, { Options } from "react-markdown";
import remarkGfm from "remark-gfm";
import rehypeHighlight from "rehype-highlight";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Source } from "../lib/api";
import { isWebUrl } from "../lib/format";
import { CheckIcon, CopyIcon } from "./Icons";

type Plugins = NonNullable<Options["rehypePlugins"]>;

// Module-level arrays keep plugin identity stable between renders, so react-markdown
// does not rebuild its processor on every streamed token.
const REMARK_PLUGINS: NonNullable<Options["remarkPlugins"]> = [remarkGfm];
/** Language detection is skipped while a reply streams; the final render detects. */
const LIVE_PLUGINS: Plugins = [[rehypeHighlight, { detect: false, ignoreMissing: true }]];
const FINAL_PLUGINS: Plugins = [[rehypeHighlight, { detect: true, ignoreMissing: true }]];

function textOf(node: ReactNode): string {
  if (typeof node === "string" || typeof node === "number") return String(node);
  if (Array.isArray(node)) return node.map(textOf).join("");
  if (isValidElement<{ children?: ReactNode }>(node)) return textOf(node.props.children);
  return "";
}

function CodeBlock({ children }: { children?: ReactNode }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);
  const code = isValidElement<{ className?: string; children?: ReactNode }>(children)
    ? children
    : null;
  const lang = code?.props.className?.match(/language-(\S+)/)?.[1] ?? "";
  const copy = async () => {
    await navigator.clipboard.writeText(textOf(children).replace(/\n$/, ""));
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), 1500);
  };
  return (
    <div className="code-block">
      <div className="code-header">
        <span>{lang || "code"}</span>
        <button className="icon-btn" onClick={copy} title="Copy code" aria-label="Copy code">
          {copied ? <CheckIcon /> : <CopyIcon />}
          <span>{copied ? "Copied" : "Copy"}</span>
        </button>
      </div>
      <pre>{children}</pre>
    </div>
  );
}

/**
 * Code first (fenced blocks, closed or still streaming, and inline spans), so a citation inside it
 * is consumed and left alone; otherwise a run of bare citations like `[1]` or `[1][2]`.
 */
const CITATION_RUN = /(```[\s\S]*?(?:```|$)|~~~[\s\S]*?(?:~~~|$)|`[^`\n]*`)|((?:\[\d{1,3}\])+)/g;

/** Rewrites each bare `[n]` that names a web source as a link to it, so the `a` handler opens it. */
function linkCitations(text: string, sources: Source[] | undefined): string {
  if (!sources?.length || !text.includes("[")) return text;
  const links = new Map<number, string>();
  for (const s of sources) {
    if (!isWebUrl(s.url)) continue;
    const url = s.url.replace(/[\s<>]/g, (c) => encodeURIComponent(c));
    const title = (s.title ?? "").replace(/\s+/g, " ").trim().replace(/["\\]/g, "\\$&");
    links.set(s.n, title ? `(<${url}> "${title}")` : `(<${url}>)`);
  }
  if (links.size === 0) return text;
  return text.replace(CITATION_RUN, (match: string, code: string | undefined, run: string | undefined, offset: number) => {
    if (code !== undefined || run === undefined) return match;
    // After `]` it is the label of `[text][1]`; after `!` or `\` it is an image or escaped text.
    if (/[\]!\\]/.test(text[offset - 1] ?? "")) return match;
    const items = run.match(/\[\d+\]/g) ?? [];
    // In `[1][2](…)` the last one is already a link's text.
    const linkable = text[offset + match.length] === "(" ? items.length - 1 : items.length;
    return items
      .map((item, i) => {
        const link = i < linkable ? links.get(Number(item.slice(1, -1))) : undefined;
        return link ? item + link : item;
      })
      .join("");
  });
}

const COMPONENTS: Options["components"] = {
  pre: CodeBlock,
  a: ({ href, title, children }) => {
    // A link whose whole text is a short number is a source citation.
    const n = textOf(children);
    const cite = /^\d{1,3}$/.test(n);
    return (
      <a
        href={href}
        title={title}
        className={cite ? "cite" : undefined}
        aria-label={cite ? `Source ${n}` : undefined}
        onClick={(e) => {
          e.preventDefault();
          if (href && /^(https?:|mailto:)/.test(href)) openUrl(href).catch(() => {});
        }}
      >
        {children}
      </a>
    );
  },
};

interface MarkdownProps {
  text: string;
  live?: boolean;
  /** Online search results; a bare `[n]` in the text links to source n. */
  sources?: Source[];
}

export const Markdown = memo(function Markdown({ text, live = false, sources }: MarkdownProps) {
  const linked = useMemo(() => linkCitations(text, sources), [text, sources]);
  return (
    <div className="markdown">
      <ReactMarkdown
        remarkPlugins={REMARK_PLUGINS}
        rehypePlugins={live ? LIVE_PLUGINS : FINAL_PLUGINS}
        components={COMPONENTS}
      >
        {linked}
      </ReactMarkdown>
    </div>
  );
});

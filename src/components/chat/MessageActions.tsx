import { useEffect, useRef, useState } from "react";
import { CheckIcon, CopyIcon } from "../Icons";

const COPIED_MS = 1500;

/** Actions under a saved assistant reply; shown on hover or focus. Copies the Markdown source. */
export function MessageActions({ content }: { content: string }) {
  const [copied, setCopied] = useState(false);
  const timer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(timer.current), []);

  const copy = async () => {
    try {
      await navigator.clipboard.writeText(content);
    } catch {
      return;
    }
    setCopied(true);
    window.clearTimeout(timer.current);
    timer.current = window.setTimeout(() => setCopied(false), COPIED_MS);
  };

  return (
    <div className="msg-actions">
      <button type="button" className="icon-btn" onClick={copy} title="Copy reply" aria-label={copied ? "Copied" : "Copy reply"}>
        {copied ? <CheckIcon /> : <CopyIcon />}
        <span>{copied ? "Copied" : "Copy"}</span>
      </button>
    </div>
  );
}

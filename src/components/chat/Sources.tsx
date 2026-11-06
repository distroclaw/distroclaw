import { useId } from "react";
import { openUrl } from "@tauri-apps/plugin-opener";
import { Source } from "../../lib/api";
import { isWebUrl } from "../../lib/format";

/** The site a link points to, without a leading "www."; empty when the url does not parse. */
const domainOf = (url: string): string => {
  try {
    return new URL(url).hostname.replace(/^www\./, "");
  } catch {
    return "";
  }
};

/** The numbered pages online search found for a reply; the `[n]` citations in the text point here. */
export function Sources({ sources }: { sources: Source[] }) {
  const headingId = useId();
  if (sources.length === 0) return null;
  return (
    <div className="sources">
      <h3 className="sources-title" id={headingId}>
        Sources
      </h3>
      {/* An explicit role keeps list semantics in WebKit once the markers are hidden. */}
      <ol className="sources-list" role="list" aria-labelledby={headingId}>
        {sources.map((s, i) => {
          const domain = domainOf(s.url);
          const body = (
            <>
              <span className="sources-n">{s.n}.</span>
              <span className="sources-name">{s.title || domain || s.url}</span>
              {domain && <span className="sources-domain">— {domain}</span>}
            </>
          );
          return (
            <li key={`${s.n}-${i}`}>
              {isWebUrl(s.url) ? (
                <a
                  href={s.url}
                  title={s.url}
                  onClick={(e) => {
                    e.preventDefault();
                    openUrl(s.url).catch(() => {});
                  }}
                >
                  {body}
                </a>
              ) : (
                <span className="sources-item">{body}</span>
              )}
            </li>
          );
        })}
      </ol>
    </div>
  );
}

import { useCallback, useEffect, useState } from "react";
import { api, errorText, OlympCheck, OlympStatus } from "../../lib/api";
import { ConfirmDialog } from "../ui/ConfirmDialog";
import { Toggle } from "../ui/Toggle";

/** Olymp's sources, as the backend registers them (`olymp_<key>` switches each, on unless "off"). */
const SOURCES: { group: string; items: { key: string; label: string; help: string; defaultOff?: boolean }[] }[] = [
  {
    group: "General web",
    items: [
      { key: "searxng", label: "SearXNG on this computer", help: "Google, Bing, Yahoo, Startpage, Brave, Qwant and more, when you run SearXNG." },
      { key: "duckduckgo", label: "DuckDuckGo", help: "Web search, also for public documents." },
      { key: "wikipedia", label: "Wikipedia", help: "In English, German, French and Italian." },
    ],
  },
  {
    group: "News",
    items: [
      { key: "gdelt", label: "GDELT", help: "News from around the world. Off at first: it is often busy and slow.", defaultOff: true },
      { key: "hackernews", label: "Hacker News", help: "Technology news and discussions." },
    ],
  },
  {
    group: "Research papers",
    items: [
      { key: "openalex", label: "OpenAlex", help: "Scholarly works, with open-access PDFs." },
      { key: "arxiv", label: "arXiv", help: "Preprints in science and engineering." },
      { key: "crossref", label: "Crossref", help: "Published articles and reports." },
    ],
  },
  {
    group: "Developer",
    items: [
      { key: "stackexchange", label: "Stack Exchange", help: "Questions and answers from Stack Overflow." },
      { key: "github", label: "GitHub", help: "Code repositories." },
    ],
  },
  {
    group: "With your own key",
    items: [
      { key: "brave", label: "Brave Search", help: "Used when a key is saved below." },
      { key: "google", label: "Google Programmable Search", help: "Used when a key and search engine ID are saved below." },
    ],
  },
];

const CHECK_TEXT: Record<string, string> = {
  ok: "Running",
  off: "Off",
  not_running: "Not running at this address",
  no_json: "Running, but its JSON format is off (see the setup guide)",
  error: "Could not be checked",
};

interface Props {
  /** Online search is on (General); Olymp only works then. */
  webSearch: boolean;
  /** After install or uninstall, so the skill list and composer update. */
  onChanged?: () => void;
}

const DOCKER_TEXT: Record<string, string> = {
  missing: "Docker isn't installed on this computer.",
  stopped: "Docker is installed but not running. Start Docker Desktop first.",
  ready: "Docker is running.",
};

/** Settings › Olymp: install or uninstall the add-on; once installed, its sources, services and keys. */
export function ResearchPanel({ webSearch, onChanged }: Props) {
  const [status, setStatus] = useState<OlympStatus | null>(null);
  const [statusError, setStatusError] = useState<string | null>(null);
  const [busy, setBusy] = useState(false);
  const [confirmUninstall, setConfirmUninstall] = useState(false);

  const refresh = useCallback(async () => {
    try {
      setStatus(await api.olympStatus());
      setStatusError(null);
    } catch (e) {
      setStatusError(errorText(e));
    }
  }, []);

  useEffect(() => {
    refresh();
  }, [refresh]);

  // While services are set up or removed, follow the task.
  useEffect(() => {
    if (status?.task.state !== "working") return;
    const t = window.setInterval(refresh, 1500);
    return () => window.clearInterval(t);
  }, [status?.task.state, refresh]);

  const act = async (fn: () => Promise<void>) => {
    setBusy(true);
    setStatusError(null);
    try {
      await fn();
      await refresh();
      onChanged?.();
    } catch (e) {
      setStatusError(errorText(e));
    } finally {
      setBusy(false);
    }
  };

  if (!status) {
    return statusError ? <p className="error-text">{statusError}</p> : <p className="field-help">Loading…</p>;
  }

  if (!status.installed) {
    return (
      <section className="olymp-card">
        <h3>Olymp</h3>
        <p className="olymp-tagline">A research agent that searches many sources at once and answers with sources.</p>
        <ul className="olymp-points">
          <li>Searches the web, news, research papers and developer sites in one go</li>
          <li>Finds and reads public PDF, Word, Excel and PowerPoint files</li>
          <li>Checks facts in more than one source and cites every one</li>
          <li>Keeps your files out of searches, and respects each website's rules</li>
        </ul>
        <p className="field-help">
          Olymp is an add-on: it does nothing until you install it, and uninstalling removes it again with its settings.
          It uses your local model and needs Online search to research.
        </p>
        <div className="label-row">
          <button className="btn primary" disabled={busy} onClick={() => act(api.olympInstall)}>
            {busy ? "Installing…" : "Install Olymp"}
          </button>
        </div>
        {status.task.state === "done" && status.task.message && <p className="field-help" role="status">{status.task.message}</p>}
        {statusError && <p className="error-text">{statusError}</p>}
      </section>
    );
  }

  return (
    <>
      <InstalledPanel webSearch={webSearch} />

      <section className="field">
        <label>More sources on this computer</label>
        <p className="field-help">
          SearXNG reaches Google, Bing, Yahoo and more; Crawl4AI reads pages that need JavaScript. The app can set them up
          with Docker, on this computer only (127.0.0.1). The first setup downloads about 2 GB.
        </p>
        {status.services ? (
          <p className="field-help">Set up by DistroClaw. Uninstalling Olymp removes them, images included.</p>
        ) : (
          <>
            <p className="field-help">{DOCKER_TEXT[status.docker] ?? status.docker}</p>
            <div className="label-row">
              <button
                className="btn"
                disabled={busy || status.docker !== "ready" || status.task.state === "working"}
                onClick={() => act(api.olympStartServices)}
              >
                Set up SearXNG and Crawl4AI
              </button>
            </div>
          </>
        )}
        {status.task.state !== "idle" && status.task.message && (
          <p className={status.task.state === "error" ? "error-text" : "field-help"} role="status">
            {status.task.state === "working" && <span className="spinner" aria-hidden="true" />} {status.task.message}
          </p>
        )}
      </section>

      <section className="field">
        <label>Uninstall</label>
        <p className="field-help">Removes Olymp, its settings, keys and source switches for everyone, and the services the app set up.</p>
        <div className="label-row">
          <button className="btn danger" disabled={busy || status.task.state === "working"} onClick={() => setConfirmUninstall(true)}>
            Uninstall Olymp
          </button>
        </div>
      </section>
      {statusError && <p className="error-text">{statusError}</p>}

      {confirmUninstall && (
        <ConfirmDialog
          title="Uninstall Olymp?"
          message="Olymp disappears from the app. Your chats stay as they are."
          confirmLabel="Uninstall"
          danger
          onConfirm={async () => {
            await api.olympUninstall();
            setConfirmUninstall(false);
            await refresh();
            onChanged?.();
          }}
          onCancel={() => setConfirmUninstall(false)}
        >
          <ul>
            <li>Its settings, search keys and everyone's source switches are deleted.</li>
            {status.services && <li>SearXNG and Crawl4AI are stopped and removed, with their images.</li>}
          </ul>
        </ConfirmDialog>
      )}
    </>
  );
}

/** Once installed: which sources Olymp asks, the local SearXNG and Crawl4AI addresses, and search keys. */
function InstalledPanel({ webSearch }: { webSearch: boolean }) {
  const [off, setOff] = useState<Set<string>>(new Set());
  const [searxng, setSearxng] = useState("");
  const [crawl4ai, setCrawl4ai] = useState("");
  const [googleCx, setGoogleCx] = useState("");
  const [braveKeySet, setBraveKeySet] = useState(false);
  const [googleKeySet, setGoogleKeySet] = useState(false);
  const [braveKey, setBraveKey] = useState("");
  const [googleKey, setGoogleKey] = useState("");
  const [check, setCheck] = useState<OlympCheck | null>(null);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const [saved, setSaved] = useState<string | null>(null);

  const load = useCallback(async () => {
    try {
      const items = SOURCES.flatMap((g) => g.items);
      const values = await Promise.all(items.map((i) => api.getSetting(`olymp_${i.key}`)));
      // Unset means the source's default: on, except the ones marked defaultOff.
      setOff(new Set(items.filter((it, i) => values[i] === "off" || (values[i] == null && it.defaultOff)).map((it) => it.key)));
      setSearxng((await api.getSetting("olymp_searxng_url")) ?? "");
      setCrawl4ai((await api.getSetting("olymp_crawl4ai_url")) ?? "");
      setGoogleCx((await api.getSetting("olymp_google_cx")) ?? "");
      setBraveKeySet((await api.getSetting("olymp_brave_key")) === "set");
      setGoogleKeySet((await api.getSetting("olymp_google_key")) === "set");
    } catch (e) {
      setError(errorText(e));
    }
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const run = async (what: string, fn: () => Promise<void>) => {
    setError(null);
    setSaved(null);
    try {
      await fn();
      setSaved(what);
    } catch (e) {
      setError(errorText(e));
    }
  };

  const toggle = (key: string, on: boolean) =>
    run("", async () => {
      await api.setSetting(`olymp_${key}`, on ? "on" : "off");
      setOff((s) => {
        const next = new Set(s);
        if (on) next.delete(key);
        else next.add(key);
        return next;
      });
    });

  const test = async () => {
    setChecking(true);
    setError(null);
    try {
      setCheck(await api.olympCheck());
    } catch (e) {
      setError(errorText(e));
    } finally {
      setChecking(false);
    }
  };

  return (
    <>
      <p className="field-help research-intro">
        Olymp researches across many sources at once: type <span className="mono">/olymp</span> in a chat.
        {webSearch ? " " : " It works while Online search is on (General). "}
        Each source receives the search words and your IP address, never your files.
      </p>

      {SOURCES.map((g) => (
        <section className="field" key={g.group}>
          <label>{g.group}</label>
          <ul className="item-list research-sources">
            {g.items.map((s) => (
              <li key={s.key} className="research-source">
                <div className="skill-text">
                  <b>{s.label}</b>
                  <span className="field-help">{s.help}</span>
                </div>
                <Toggle inline label={s.label} checked={!off.has(s.key)} disabled={!webSearch} onChange={(on) => toggle(s.key, on)} />
              </li>
            ))}
          </ul>
        </section>
      ))}

      <section className="field">
        <label htmlFor="olymp-searxng">Services on this computer</label>
        <p className="field-help">
          Addresses of a SearXNG and a Crawl4AI you run yourself, written as 127.0.0.1. Leave SearXNG empty for its usual
          address, or write "off". Crawl4AI renders pages that need JavaScript; it stays off until you set it. Only an
          admin can change these.
        </p>
        <div className="label-row">
          <input
            id="olymp-searxng"
            className="input"
            value={searxng}
            placeholder="SearXNG: http://127.0.0.1:8888"
            onChange={(e) => setSearxng(e.target.value)}
          />
        </div>
        <div className="label-row">
          <input
            className="input"
            aria-label="Crawl4AI address"
            value={crawl4ai}
            placeholder="Crawl4AI: http://127.0.0.1:11235 (off)"
            onChange={(e) => setCrawl4ai(e.target.value)}
          />
        </div>
        <div className="label-row">
          <button
            className="btn"
            onClick={() =>
              run("Addresses saved.", async () => {
                await api.setSetting("olymp_searxng_url", searxng.trim());
                await api.setSetting("olymp_crawl4ai_url", crawl4ai.trim());
              })
            }
          >
            Save addresses
          </button>
          <button className="btn" onClick={test} disabled={checking}>
            {checking ? "Testing…" : "Test"}
          </button>
        </div>
        {check && (
          <p className="field-help" role="status">
            SearXNG: {CHECK_TEXT[check.searxng] ?? check.searxng} · Crawl4AI: {CHECK_TEXT[check.crawl4ai] ?? check.crawl4ai}
          </p>
        )}
      </section>

      <section className="field">
        <label htmlFor="olymp-brave">Search keys</label>
        <p className="field-help">
          Optional. Keys are stored on this computer and never shown again. Each service limits free use.
        </p>
        <div className="label-row">
          <input
            id="olymp-brave"
            className="input"
            type="password"
            autoComplete="off"
            value={braveKey}
            placeholder={braveKeySet ? "Brave Search key is saved" : "Brave Search API key"}
            onChange={(e) => setBraveKey(e.target.value)}
          />
          <button
            className="btn"
            disabled={!braveKey.trim()}
            onClick={() =>
              run("Key saved.", async () => {
                await api.setSetting("olymp_brave_key", braveKey.trim());
                setBraveKey("");
                setBraveKeySet(true);
              })
            }
          >
            Save
          </button>
          {braveKeySet && (
            <button className="btn" onClick={() => run("Key removed.", async () => { await api.setSetting("olymp_brave_key", ""); setBraveKeySet(false); })}>
              Remove
            </button>
          )}
        </div>
        <div className="label-row">
          <input
            className="input"
            type="password"
            autoComplete="off"
            aria-label="Google API key"
            value={googleKey}
            placeholder={googleKeySet ? "Google key is saved" : "Google Programmable Search API key"}
            onChange={(e) => setGoogleKey(e.target.value)}
          />
          <button
            className="btn"
            disabled={!googleKey.trim()}
            onClick={() =>
              run("Key saved.", async () => {
                await api.setSetting("olymp_google_key", googleKey.trim());
                setGoogleKey("");
                setGoogleKeySet(true);
              })
            }
          >
            Save
          </button>
          {googleKeySet && (
            <button className="btn" onClick={() => run("Key removed.", async () => { await api.setSetting("olymp_google_key", ""); setGoogleKeySet(false); })}>
              Remove
            </button>
          )}
        </div>
        <div className="label-row">
          <input
            className="input"
            aria-label="Google search engine ID"
            value={googleCx}
            placeholder="Google search engine ID (cx)"
            onChange={(e) => setGoogleCx(e.target.value)}
            onBlur={() => run("Search engine ID saved.", () => api.setSetting("olymp_google_cx", googleCx.trim()))}
          />
        </div>
      </section>

      {saved && <p className="field-help" role="status">{saved}</p>}
      {error && <p className="error-text">{error}</p>}
    </>
  );
}

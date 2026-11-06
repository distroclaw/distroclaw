import { useCallback, useEffect, useState } from "react";
import { api, DownloadStatus, errorText, formatBytes, SetupInfo } from "../lib/api";
import { Logo } from "./Icons";

/** Shown when no chat model is installed: recommends one for this computer and downloads it
 *  only after the user agrees. The app moves on by itself once the engine has the model. */
export function ModelSetup() {
  const [info, setInfo] = useState<SetupInfo | null>(null);
  const [choice, setChoice] = useState("");
  const [consent, setConsent] = useState(false);
  const [download, setDownload] = useState<DownloadStatus | null>(null);
  const [error, setError] = useState<string | null>(null);

  const load = useCallback(() => {
    setError(null);
    api
      .setupInfo()
      .then((i) => {
        setInfo(i);
        setChoice(i.recommendation.tag);
        setDownload(i.download);
      })
      .catch((e) => setError(errorText(e)));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  useEffect(() => {
    if (download?.state !== "downloading") return;
    const timer = window.setInterval(() => api.downloadStatus().then(setDownload).catch(() => {}), 500);
    return () => window.clearInterval(timer);
  }, [download?.state]);

  if (!info) {
    return (
      <div className="center-screen">
        <div className="card setup">
          <Logo size={48} />
          <h2>{error ? "Could not check this computer" : "Checking this computer…"}</h2>
          {error && <p className="error-text">{error}</p>}
          {error && (
            <button className="btn primary" onClick={load}>
              Try again
            </button>
          )}
        </div>
      </div>
    );
  }

  const { hardware: hw, recommendation: rec } = info;
  const chosen = rec.options.find((o) => o.tag === choice) ?? rec.options[0];
  const gpu = hw.gpus[0]?.name ?? "No supported graphics processor";
  const busy = download?.state === "downloading" || download?.state === "done";

  const start = async () => {
    setError(null);
    try {
      await api.startModelDownload(chosen.tag, consent);
      setDownload(await api.downloadStatus());
    } catch (e) {
      setError(errorText(e));
    }
  };

  const percent = download && download.total > 0 ? Math.round((download.completed / download.total) * 100) : 0;

  return (
    <div className="center-screen">
      <div className="card setup">
        <Logo size={48} />
        <h2>Set up your local AI</h2>
        <p className="muted">DistroClaw picked the model that suits this computer best.</p>

        <dl className="hw">
          <div><dt>Processor</dt><dd>{hw.cpu || hw.arch} · {hw.cores} cores</dd></div>
          <div><dt>Memory</dt><dd>{formatBytes(hw.totalMemoryBytes)}</dd></div>
          <div><dt>Graphics</dt><dd>{gpu}</dd></div>
          <div><dt>Free disk</dt><dd>{formatBytes(hw.freeDiskBytes)}</dd></div>
        </dl>

        <div className="model-options" role="radiogroup" aria-label="AI model">
          {rec.options.map((o) => (
            <label key={o.tag} className={`model-option ${o.tag === chosen.tag ? "on" : ""} ${!o.fitsDisk ? "disabled" : ""}`}>
              <input
                type="radio"
                name="model"
                value={o.tag}
                checked={o.tag === chosen.tag}
                disabled={busy || !o.fitsDisk}
                onChange={() => setChoice(o.tag)}
              />
              <span className="model-name">
                {o.label}
                {o.tag === rec.tag && <span className="value-pill">Recommended</span>}
                {!o.fitsMemory && <span className="warn-pill">May be slow</span>}
                {!o.fitsDisk && <span className="warn-pill">Not enough disk space</span>}
              </span>
              <span className="model-size">{formatBytes(o.downloadBytes)}</span>
              <span className="model-desc">{o.description}</span>
            </label>
          ))}
        </div>
        <p className="hint">{rec.reason}</p>

        {download?.state === "downloading" || download?.state === "done" ? (
          <div className="download">
            <div className="progress">
              <div
                className={download.total > 0 ? "progress-bar" : "progress-bar indeterminate"}
                style={download.total > 0 ? { width: `${percent}%` } : undefined}
              />
            </div>
            <p className="hint">
              {download.state === "done"
                ? "Download complete. Starting your AI…"
                : download.total > 0
                  ? `${download.message} ${formatBytes(download.completed)} of ${formatBytes(download.total)} (${percent}%)`
                  : download.message}
            </p>
            {download.state === "downloading" && (
              <button className="btn" onClick={() => api.cancelModelDownload().catch((e) => setError(errorText(e)))}>
                Cancel
              </button>
            )}
          </div>
        ) : (
          <>
            <label className="consent">
              <input type="checkbox" checked={consent} onChange={(e) => setConsent(e.target.checked)} />
              <span>
                Download {chosen.label} (about {formatBytes(chosen.downloadBytes)}) from the DistroClaw model library.
                Only the model is downloaded; your chats and files are never sent.
              </span>
            </label>
            <button className="btn primary" onClick={start} disabled={!consent}>
              {download?.state === "error" || download?.state === "cancelled" ? "Try again" : "Download and start"}
            </button>
            {(download?.state === "error" || download?.state === "cancelled") && <p className="error-text">{download.message}</p>}
          </>
        )}
        {error && <p className="error-text">{error}</p>}
      </div>
    </div>
  );
}

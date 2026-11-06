import { EngineStatus } from "../lib/api";
import { Logo } from "./Icons";

interface Props {
  status: EngineStatus | null;
  onRetry: () => void;
}

export function EngineGate({ status, onRetry }: Props) {
  const failed = status?.state === "error";
  const preparing = status?.state === "preparing";
  return (
    <div className="center-screen">
      <div className="card engine-gate">
        <Logo size={48} />
        <h2>{failed ? "Local AI unavailable" : "Getting your local AI ready"}</h2>
        <p className="muted">{status?.message ?? "Starting…"}</p>
        {!failed && (
          <div className="progress">
            <div
              className={preparing ? "progress-bar" : "progress-bar indeterminate"}
              style={preparing ? { width: `${Math.round(status!.progress * 100)}%` } : undefined}
            />
          </div>
        )}
        {failed && (
          <button className="btn primary" onClick={onRetry}>
            Try again
          </button>
        )}
        <p className="hint">Everything runs on this computer. No internet connection is needed.</p>
      </div>
    </div>
  );
}

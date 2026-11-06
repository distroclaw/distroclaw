import { useCallback, useEffect, useState } from "react";
import { api, EngineStatus, errorText, formatBytes, InstalledModel } from "../../lib/api";
import { FolderIcon, GearIcon, TrashIcon } from "../Icons";
import { ConfirmDialog } from "../ui/ConfirmDialog";

export const CONTEXT_STOPS = [4096, 8192, 16384, 32768, 65536, 131072, 262144];
export const DEFAULT_CONTEXT = 32768;
const label = (tokens: number) => `${tokens / 1024}k`;

interface Props {
  engine: EngineStatus;
  /** The model being switched to, while a switch runs. */
  switchingModel: string | null;
  /** Switches the chat model at once, not on Save. */
  onSwitchModel: (tag: string) => void;
  contextIndex: number;
  onContextIndex: (i: number) => void;
}

export function ModelPanel({ engine, switchingModel, onSwitchModel, contextIndex, onContextIndex }: Props) {
  const tokens = CONTEXT_STOPS[contextIndex];
  const engineNote = engine.mode === "bundled" ? "built-in engine" : "external engine";
  const [models, setModels] = useState<InstalledModel[] | null>(null);
  const [listError, setListError] = useState<string | null>(null);
  /** The license of the model in use: undefined while loading or unknown, null when it has none. */
  const [license, setLicense] = useState<string | null | undefined>(undefined);
  const [removing, setRemoving] = useState<InstalledModel | null>(null);
  const [error, setError] = useState<string | null>(null);

  const loadModels = useCallback(() => {
    let alive = true;
    api
      .listChatModels()
      .then((list) => alive && setModels(list))
      .catch((e) => alive && setListError(errorText(e)));
    return () => {
      alive = false;
    };
  }, []);

  useEffect(loadModels, [loadModels]);

  useEffect(() => {
    let alive = true;
    setLicense(undefined);
    api
      .modelLicense(engine.model)
      .then((text) => alive && setLicense(text))
      // Not "has none": the engine could not say, so nothing is shown.
      .catch(() => {});
    return () => {
      alive = false;
    };
  }, [engine.model]);

  const remove = async (m: InstalledModel) => {
    await api.removeModel(m.name);
    setRemoving(null);
    loadModels();
  };

  const showDataFolder = () => {
    setError(null);
    api.revealDataFolder().catch((e) => setError(errorText(e)));
  };

  // The current model stays choosable even before the list arrives or if it is missing from it.
  const names = models?.map((m) => m.name) ?? [];
  const options = names.includes(engine.model) ? names : [engine.model, ...names];

  return (
    <>
      <section className="field">
        <label htmlFor="chat-model">Model</label>
        <div className="label-row">
          <select
            id="chat-model"
            className="input model-select"
            value={switchingModel ?? engine.model}
            aria-busy={switchingModel !== null || undefined}
            aria-describedby="chat-model-help"
            onChange={(e) => onSwitchModel(e.target.value)}
          >
            {options.map((name) => (
              <option key={name} value={name}>
                {name}
              </option>
            ))}
          </select>
          <span className="field-help">· {engineNote}</span>
          <span className="field-help" role="status">
            {switchingModel !== null ? "Switching…" : ""}
          </span>
        </div>
        <p id="chat-model-help" className="field-help">
          The first reply after switching is slower while the model loads.
        </p>
        {listError && <p className="error-text">{listError}</p>}
        {license === null && <p className="field-help">This model comes without license information.</p>}
        {license && (
          <details className="license">
            <summary className="field-help">License of {engine.model}</summary>
            <pre className="license-text">{license}</pre>
          </details>
        )}
      </section>

      <section className="field">
        <label>Installed models</label>
        {models && models.length > 0 && (
          <ul className="item-list">
            {models.map((m) => (
              <li key={m.name} className="model-row">
                <span className="model-row-name">
                  <b>{m.name}</b>
                  {m.current && <span className="value-pill">In use</span>}
                </span>
                <span>{formatBytes(m.sizeBytes)}</span>
                <button className="icon-btn" onClick={() => setRemoving(m)} aria-label={`Remove ${m.name}`} title="Remove">
                  <TrashIcon />
                </button>
              </li>
            ))}
          </ul>
        )}
        <div className="label-row">
          <button className="btn" onClick={showDataFolder}>
            <FolderIcon /> Show data folder
          </button>
        </div>
        {error && <p className="error-text">{error}</p>}
      </section>

      <section className="field">
        <label htmlFor="context-length" className="label-row">
          <GearIcon /> Context length <span className="value-pill">{label(tokens)}</span>
        </label>
        <p className="field-help">
          How much of the conversation and attached files the model can remember. Higher values use more memory
          and can be slower; the first reply after a change takes longer while the model reloads.
        </p>
        <input
          id="context-length"
          className="slider"
          type="range"
          min={0}
          max={CONTEXT_STOPS.length - 1}
          step={1}
          value={contextIndex}
          aria-valuetext={label(tokens)}
          onChange={(e) => onContextIndex(Number(e.target.value))}
          style={{ ["--fill" as string]: `${(contextIndex / (CONTEXT_STOPS.length - 1)) * 100}%` }}
        />
        <div className="slider-ticks">
          {CONTEXT_STOPS.map((t, i) => (
            <button key={t} type="button" className={i === contextIndex ? "on" : ""} aria-pressed={i === contextIndex} onClick={() => onContextIndex(i)}>
              {label(t)}
            </button>
          ))}
        </div>
      </section>

      {removing && (
        <ConfirmDialog
          title={`Remove ${removing.name}?`}
          message={
            removing.current
              ? `This frees about ${formatBytes(removing.sizeBytes)} of disk space. DistroClaw chats with this model now; afterwards you pick a model to download.`
              : `This frees about ${formatBytes(removing.sizeBytes)} of disk space.`
          }
          confirmLabel="Remove"
          danger
          onConfirm={() => remove(removing)}
          onCancel={() => setRemoving(null)}
        />
      )}
    </>
  );
}

import { useCallback, useEffect, useState } from "react";
import { AllowedFolder, api, AppErrorShape, errorOf, FolderMode } from "../../lib/api";
import { formatCount, truncateMiddle } from "../../lib/format";
import { ext } from "../../ext";
import { PlusIcon, TrashIcon } from "../Icons";
import { ConfirmDialog } from "../ui/ConfirmDialog";

const { AccessSection, FolderHelp, FolderControl } = ext;

interface Props {
  /** Called after any change so the sidebar status line can refresh. */
  onChanged?: () => void;
}

function FolderRow({
  folder,
  onUpdate,
  onRemove,
}: {
  folder: AllowedFolder;
  onUpdate: (label: string, mode: FolderMode) => Promise<void>;
  onRemove: () => void;
}) {
  const [label, setLabel] = useState(folder.label);
  useEffect(() => setLabel(folder.label), [folder.label]);

  const commitLabel = () => {
    const next = label.trim();
    if (!next || next === folder.label) {
      setLabel(folder.label);
      return;
    }
    onUpdate(next, folder.mode);
  };

  return (
    <li className="folder-row">
      <div className="label-row">
        <input
          className="input"
          value={label}
          aria-label="Folder name"
          onChange={(e) => setLabel(e.target.value)}
          onBlur={commitLabel}
          onKeyDown={(e) => e.key === "Enter" && (e.target as HTMLInputElement).blur()}
        />
        {FolderControl && <FolderControl folder={folder} onMode={(mode) => onUpdate(folder.label, mode)} />}
        <button className="icon-btn" onClick={onRemove} aria-label={`Remove ${folder.label}`} title="Remove">
          <TrashIcon />
        </button>
      </div>
      <p className="field-help">
        <span className="mono" title={folder.path}>
          {truncateMiddle(folder.path)}
        </span>
        {" · "}
        {formatCount(folder.fileCount)} {folder.fileCount === 1 ? "file" : "files"}
      </p>
    </li>
  );
}

export function AccessPanel({ onChanged }: Props) {
  const [folders, setFolders] = useState<AllowedFolder[] | null>(null);
  const [error, setError] = useState<AppErrorShape | null>(null);
  const [busy, setBusy] = useState(false);
  const [removing, setRemoving] = useState<AllowedFolder | null>(null);

  const load = useCallback(() => {
    setError(null);
    api
      .listAllowedFolders()
      .then(setFolders)
      .catch((e) => setError(errorOf(e)));
  }, []);

  useEffect(() => {
    load();
  }, [load]);

  const run = async (action: () => Promise<void>) => {
    setBusy(true);
    setError(null);
    try {
      await action();
      onChanged?.();
    } catch (e) {
      setError(errorOf(e));
    } finally {
      setBusy(false);
    }
  };

  const add = () =>
    run(async () => {
      const f = await api.pickFolder();
      if (!f) return;
      setFolders((xs) => [...(xs ?? []).filter((x) => x.id !== f.id), f]);
    });

  const update = (id: string, label: string, mode: FolderMode) =>
    run(async () => {
      const f = await api.updateAllowedFolder(id, label, mode);
      setFolders((xs) => (xs ?? []).map((x) => (x.id === id ? f : x)));
    });

  const remove = async (f: AllowedFolder) => {
    await api.removeAllowedFolder(f.id);
    setFolders((xs) => (xs ?? []).filter((x) => x.id !== f.id));
    setRemoving(null);
    onChanged?.();
  };

  return (
    <>
      {AccessSection && <AccessSection onChanged={onChanged} />}

      <section className="field">
        <label>Folders DistroClaw may use</label>
        <p className="field-help">{FolderHelp ? <FolderHelp /> : "DistroClaw reads only these folders and never changes your files."}</p>
        {folders === null && !error && <p className="muted small">Loading…</p>}
        {folders && folders.length === 0 && <p className="muted small">No folders yet.</p>}
        {folders && folders.length > 0 && (
          <ul className="item-list">
            {folders.map((f) => (
              <FolderRow key={f.id} folder={f} onUpdate={(label, mode) => update(f.id, label, mode)} onRemove={() => setRemoving(f)} />
            ))}
          </ul>
        )}
        <div className="label-row">
          <button className="btn" onClick={add} disabled={busy || folders === null}>
            <PlusIcon /> Add folder…
          </button>
        </div>
      </section>

      {error && (
        <div className="error-banner" role="alert">
          {error.message}
          {error.code === "accessDenied" && (
            <> Allow DistroClaw in System Settings → Privacy &amp; Security → Files and Folders.</>
          )}
          {folders === null && (
            <>
              {" "}
              <button className="text-link" onClick={load}>
                Try again
              </button>
            </>
          )}
        </div>
      )}

      {removing && (
        <ConfirmDialog
          title={`Remove ${removing.label}?`}
          message="DistroClaw forgets this folder. No files are changed."
          confirmLabel="Remove"
          danger
          onConfirm={() => remove(removing)}
          onCancel={() => setRemoving(null)}
        />
      )}
    </>
  );
}

import { ReactNode, useState } from "react";
import { errorText } from "../../lib/api";
import { Modal } from "../Modal";

interface Props {
  title: string;
  message: string;
  confirmLabel: string;
  danger?: boolean;
  /** Extra points under the message, typically a bulleted list. */
  children?: ReactNode;
  /** When set, a checkbox with this text must be ticked before the confirm button works. */
  consentLabel?: string;
  onConfirm: () => void | Promise<void>;
  onCancel: () => void;
}

/** A yes/no question built on Modal. Errors from onConfirm stay in the dialog. */
export function ConfirmDialog({ title, message, confirmLabel, danger = false, children, consentLabel, onConfirm, onCancel }: Props) {
  const [busy, setBusy] = useState(false);
  const [consented, setConsented] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const canConfirm = !busy && (!consentLabel || consented);

  const confirm = async () => {
    if (!canConfirm) return;
    setBusy(true);
    setError(null);
    try {
      await onConfirm();
    } catch (e) {
      setError(errorText(e));
      setBusy(false);
    }
  };

  return (
    <Modal
      title={title}
      onClose={onCancel}
      footer={
        <>
          <button className="btn" onClick={onCancel} disabled={busy}>
            Cancel
          </button>
          <button className={`btn ${danger ? "danger" : "primary"}`} onClick={confirm} disabled={!canConfirm}>
            {confirmLabel}
          </button>
        </>
      }
    >
      <div className="consent-dialog">
        <p className="field-help" style={{ fontSize: 14, color: "var(--text)" }}>
          {message}
        </p>
        {children}
        {consentLabel && (
          <label className="consent">
            <input type="checkbox" checked={consented} disabled={busy} onChange={(e) => setConsented(e.target.checked)} />
            <span>{consentLabel}</span>
          </label>
        )}
        {error && <p className="error-text">{error}</p>}
      </div>
    </Modal>
  );
}

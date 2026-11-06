import { Attachment } from "../lib/api";
import { formatSize } from "../lib/attachments";
import { FileIcon, XIcon } from "./Icons";

interface Props {
  name: string;
  attachment?: Attachment;
  status?: "reading" | "ready" | "error";
  error?: string;
  onRemove?: () => void;
}

export function AttachmentChip({ name, attachment, status = "ready", error, onRemove }: Props) {
  const image = attachment?.kind === "image" && attachment.data;
  const detail =
    status === "reading"
      ? "Reading…"
      : status === "error"
        ? error
        : attachment
          ? `${formatSize(attachment.size)}${attachment.truncated ? " · truncated" : ""}`
          : "";
  return (
    <div className={`chip ${status}`} title={status === "error" ? error : name}>
      {image ? (
        <img className="chip-thumb" src={`data:${attachment.mime};base64,${attachment.data}`} alt="" />
      ) : (
        <span className="chip-icon">{status === "reading" ? <span className="spinner" /> : <FileIcon />}</span>
      )}
      <span className="chip-text">
        <span className="chip-name">{name}</span>
        <span className="chip-detail">{detail}</span>
      </span>
      {onRemove && (
        <button className="chip-remove" onClick={onRemove} title="Remove" aria-label={`Remove ${name}`}>
          <XIcon />
        </button>
      )}
    </div>
  );
}

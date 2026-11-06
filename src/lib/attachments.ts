import { useCallback, useEffect, useRef, useState } from "react";
import { api, Attachment, errorText } from "./api";

const MAX_IMAGE_BYTES = 25 * 1024 * 1024;
const MAX_DOCUMENT_BYTES = 50 * 1024 * 1024;
export const MAX_IMAGES = 10;
export const MAX_DOCUMENTS = 10;
/** Longest image side sent to the model (~1k vision tokens), so 10 images fit in the context. */
const MAX_IMAGE_SIDE = 1024;

export interface PendingAttachment {
  key: number;
  name: string;
  kind: "image" | "document";
  status: "reading" | "ready" | "error";
  attachment?: Attachment;
  error?: string;
}

const isImage = (f: File) => f.type.startsWith("image/");

function readDataUrl(blob: Blob): Promise<string> {
  return new Promise((resolve, reject) => {
    const reader = new FileReader();
    reader.onload = () => resolve(String(reader.result));
    reader.onerror = () => reject(reader.error);
    reader.readAsDataURL(blob);
  });
}

// Loaded from a data: URL, not a blob: URL — the CSP only allows data: images.
function loadImage(dataUrl: string, name: string): Promise<HTMLImageElement> {
  return new Promise((resolve, reject) => {
    const img = new Image();
    img.onload = () => resolve(img);
    img.onerror = () => reject(new Error(`Could not open image "${name}".`));
    img.src = dataUrl;
  });
}

const base64Of = (dataUrl: string) => dataUrl.split(",", 2)[1] ?? "";

/** PNG/JPEG within size are sent as-is; anything else is re-encoded (and scaled) via canvas. */
async function imageAttachment(file: File): Promise<Attachment> {
  if (file.size > MAX_IMAGE_BYTES) throw new Error(`"${file.name}" is larger than 25 MB.`);
  const dataUrl = await readDataUrl(file);
  const img = await loadImage(dataUrl, file.name);
  const native = file.type === "image/png" || file.type === "image/jpeg";
  const scale = Math.min(1, MAX_IMAGE_SIDE / Math.max(img.naturalWidth, img.naturalHeight));
  if (native && scale === 1) {
    return { name: file.name, kind: "image", mime: file.type, size: file.size, data: base64Of(dataUrl), truncated: false };
  }
  const canvas = document.createElement("canvas");
  canvas.width = Math.round(img.naturalWidth * scale);
  canvas.height = Math.round(img.naturalHeight * scale);
  canvas.getContext("2d")!.drawImage(img, 0, 0, canvas.width, canvas.height);
  // Only JPEG sources stay JPEG; anything else keeps PNG so transparency (WebP, GIF) does not turn black.
  const mime = file.type === "image/jpeg" ? "image/jpeg" : "image/png";
  const data = base64Of(canvas.toDataURL(mime, 0.9));
  return { name: file.name, kind: "image", mime, size: Math.round((data.length * 3) / 4), data, truncated: false };
}

async function documentAttachment(file: File): Promise<Attachment> {
  if (file.size > MAX_DOCUMENT_BYTES) throw new Error(`"${file.name}" is larger than 50 MB.`);
  const { text, truncated } = await api.readDocument(file);
  return { name: file.name, kind: "document", mime: file.type || "application/octet-stream", size: file.size, text, truncated };
}

let nextKey = 1;

type Updater = (xs: PendingAttachment[]) => PendingAttachment[];

/** State held outside the hook (for example a per-conversation draft). */
export interface PendingState {
  items: PendingAttachment[];
  setItems: (fn: Updater) => void;
}

/** Attachments being prepared for the next message. Pass `external` to keep the list elsewhere. */
export function usePendingAttachments(external?: PendingState) {
  const [local, setLocal] = useState<PendingAttachment[]>([]);
  const items = external ? external.items : local;
  const setItems = external ? external.setItems : setLocal;

  // Mirrors items so `add` can enforce the limits without side effects in a state updater.
  const current = useRef<PendingAttachment[]>(items);
  useEffect(() => {
    current.current = items;
  }, [items]);
  const update = useCallback(
    (fn: Updater) => {
      setItems((xs) => {
        const next = fn(xs);
        current.current = next;
        return next;
      });
    },
    [setItems],
  );

  const add = useCallback(
    (files: Iterable<File>) => {
      const used = (kind: PendingAttachment["kind"]) =>
        current.current.filter((x) => x.kind === kind && x.status !== "error").length;
      const room = { image: MAX_IMAGES - used("image"), document: MAX_DOCUMENTS - used("document") };
      const pending: PendingAttachment[] = [];
      const skipped = { image: 0, document: 0 };
      for (const file of Array.from(files)) {
        const kind = isImage(file) ? "image" : "document";
        if (room[kind] <= 0) {
          skipped[kind] += 1;
          continue;
        }
        room[kind] -= 1;
        const key = nextKey++;
        const name = file.name || (kind === "image" ? "pasted-image.png" : "file");
        const named = name === file.name ? file : new File([file], name, { type: file.type });
        (kind === "image" ? imageAttachment(named) : documentAttachment(named))
          .then((attachment) =>
            update((xs) => xs.map((x) => (x.key === key ? { ...x, status: "ready", attachment } : x))),
          )
          .catch((e) =>
            update((xs) => xs.map((x) => (x.key === key ? { ...x, status: "error", error: errorText(e) } : x))),
          );
        pending.push({ key, name, kind, status: "reading" });
      }
      for (const kind of ["image", "document"] as const) {
        if (skipped[kind] === 0) continue;
        const limit = kind === "image" ? `${MAX_IMAGES} images` : `${MAX_DOCUMENTS} documents`;
        pending.push({
          key: nextKey++,
          name: `${skipped[kind]} ${kind === "image" ? "image" : "file"}${skipped[kind] > 1 ? "s" : ""} not added`,
          kind,
          status: "error",
          error: `Up to ${limit} per message.`,
        });
      }
      current.current = [...current.current, ...pending];
      update((xs) => [...xs, ...pending]);
    },
    [update],
  );

  const remove = useCallback((key: number) => update((xs) => xs.filter((x) => x.key !== key)), [update]);
  const clear = useCallback(() => update(() => []), [update]);

  const ready = items.flatMap((x) => (x.status === "ready" && x.attachment ? [x.attachment] : []));
  const reading = items.some((x) => x.status === "reading");

  return { items, ready, reading, add, remove, clear };
}

export function formatSize(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 ** 2) return `${(bytes / 1024).toFixed(0)} KB`;
  return `${(bytes / 1024 ** 2).toFixed(1)} MB`;
}

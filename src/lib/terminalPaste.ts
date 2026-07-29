import type { PasteImageMimeType } from "./api";

export function clipboardHasUsableText(data: DataTransfer | null): boolean {
  if (!data) return false;
  try {
    return data.getData("text/plain").length > 0;
  } catch {
    return false;
  }
}

const NEEDS_QUOTING = /[\s!"#$&'()*;<>?[\\\]^`{|}~]/;

export function shellQuotePath(path: string): string {
  if (!NEEDS_QUOTING.test(path)) return path;
  return `'${path.split("'").join("'\\''")}'`;
}

export function formatPastedPaths(paths: string[]): string {
  return paths.map(shellQuotePath).join(" ");
}

export function normalizePasteImageMime(
  type: string,
): PasteImageMimeType | null {
  switch (type.trim().toLowerCase()) {
    case "image/png":
      return "image/png";
    case "image/jpeg":
    case "image/jpg":
      return "image/jpeg";
    default:
      return null;
  }
}

export function inferPasteImageMime(
  itemType: string,
  file: File,
): PasteImageMimeType | null {
  const fromType =
    normalizePasteImageMime(itemType) ?? normalizePasteImageMime(file.type);
  if (fromType) return fromType;

  const name = file.name.toLowerCase();
  if (name.endsWith(".png")) return "image/png";
  if (name.endsWith(".jpg") || name.endsWith(".jpeg")) return "image/jpeg";
  return null;
}

function findPasteImage(
  data: DataTransfer | null,
): { file: File; mimeType: PasteImageMimeType } | null {
  const items = data?.items;
  if (!items) return null;
  for (let i = 0; i < items.length; i += 1) {
    const item = items[i];
    if (item.kind !== "file") continue;
    const file = item.getAsFile();
    if (!file) continue;
    const mimeType = inferPasteImageMime(item.type, file);
    if (mimeType) return { file, mimeType };
  }
  return null;
}

export interface TerminalPasteEffects {
  clipboardFilePaths: () => Promise<string[]>;
  injectStdin: (text: string) => Promise<void>;
  pasteImage: (bytes: Uint8Array, mimeType: PasteImageMimeType) => Promise<void>;
  onError: (message: string) => void;
}

export function handleTerminalPaste(
  event: ClipboardEvent,
  effects: TerminalPasteEffects,
): Promise<void> | null {
  const image = findPasteImage(event.clipboardData);
  if (image) {
    event.preventDefault();
    event.stopImmediatePropagation();
    return (async () => {
      try {
        const buffer = await image.file.arrayBuffer();
        await effects.pasteImage(new Uint8Array(buffer), image.mimeType);
        await effects.injectStdin("\x16");
      } catch (error) {
        effects.onError(String(error));
      }
    })();
  }

  if (clipboardHasUsableText(event.clipboardData)) return null;

  event.preventDefault();
  event.stopImmediatePropagation();
  return (async () => {
    try {
      const paths = await effects.clipboardFilePaths();
      if (paths.length > 0) {
        await effects.injectStdin(formatPastedPaths(paths));
      }
    } catch (error) {
      effects.onError(String(error));
    }
  })();
}

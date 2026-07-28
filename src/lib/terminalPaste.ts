// Paste handling for the terminal surface (features 55 and #79).
//
// Copying a file puts no text on the clipboard — only `public.file-url`
// flavors — so xterm's default paste inserts nothing. Runner intercepts
// and inserts the POSIX path instead, which is what every native terminal
// does. Copying image *bytes* takes the other branch and keeps #79's
// attach flow.
//
// The orchestration lives here rather than in the component so it can be
// tested without mounting xterm: the ordering it encodes (commit before
// the round-trip, read the event synchronously, prefer a file reference
// over image bytes) is the whole feature, and every one of those is a
// silent failure if it regresses.

import type { PasteImageMimeType } from "./api";

/**
 * Whether the event carries text xterm can paste on its own.
 *
 * This is the gate, and it has to be answered synchronously: the handler
 * commits (`preventDefault`) before it knows whether the pasteboard holds
 * any file, because `preventDefault` after an `await` is a no-op — the
 * browser has already run the default action by the time an IPC round-trip
 * resolves. Every ordinary paste answers true here and costs no IPC.
 *
 * Deliberately keyed on text *absence*, not on `item.kind === "file"`:
 * whether WKWebView exposes an arbitrary non-image file as a `file` item is
 * unverified, and a `kind`-based trigger that never fires would make the
 * feature silently do nothing.
 */
export function clipboardHasUsableText(data: DataTransfer | null): boolean {
  if (!data) return false;
  try {
    return data.getData("text/plain").length > 0;
  } catch {
    return false;
  }
}

/**
 * Characters that make a bare path unsafe to hand a shell: whitespace and
 * the metacharacters. Letters, digits, non-ASCII, and the punctuation a
 * POSIX path routinely carries (`_ @ % + = : , . / -`) stay bare.
 */
const NEEDS_QUOTING = /[\s!"#$&'()*;<>?[\\\]^`{|}~]/;

/**
 * Quote a path only when it needs it. iTerm2 quotes unconditionally
 * because its panes are shell prompts; Runner's are usually *agent*
 * prompts, where `'/Users/jason/foo.go'` is noise and defeats `@`-style
 * file completion.
 */
export function shellQuotePath(path: string): string {
  if (!NEEDS_QUOTING.test(path)) return path;
  // Single-quote, with each embedded `'` closing the quote, escaping
  // itself, and reopening: `'` → `'\''`.
  return `'${path.split("'").join("'\\''")}'`;
}

/** Paths as they go into the terminal: quoted as needed, space-separated. */
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

/** First image the event carries as a `File`, or null. Read synchronously:
 *  `clipboardData` is only valid during dispatch. */
function findPasteImage(
  data: DataTransfer | null,
): { file: File; mimeType: PasteImageMimeType } | null {
  const items = data?.items;
  if (!items) return null;
  for (let i = 0; i < items.length; i += 1) {
    const it = items[i];
    if (it.kind !== "file") continue;
    const file = it.getAsFile();
    if (!file) continue;
    const mimeType = inferPasteImageMime(it.type, file);
    if (!mimeType) continue;
    return { file, mimeType };
  }
  return null;
}

/** Backend calls the paste handler makes, injected so the orchestration is
 *  testable without a live session. */
export interface TerminalPasteEffects {
  clipboardFilePaths: () => Promise<string[]>;
  /** Raw stdin — never `inject_paste`, which appends Enter. */
  injectStdin: (text: string) => Promise<void>;
  pasteImage: (bytes: Uint8Array, mimeType: PasteImageMimeType) => Promise<void>;
  onError: (message: string) => void;
}

/**
 * Decide and run a terminal paste.
 *
 * Returns null when the event was left alone for xterm to paste — an
 * ordinary text paste, which costs no IPC. Otherwise the event has been
 * committed (`preventDefault` + `stopImmediatePropagation`) and the
 * returned promise resolves when the injection finishes.
 *
 * The commit is synchronous by necessity: `preventDefault` after an
 * `await` does nothing, because the browser has already run the default
 * action by the time an IPC round-trip resolves. So this cannot "call the
 * command, then decide" — it decides from `clipboardData`, commits, and
 * only then goes async. Feature 55 decision 2.
 */
export function handleTerminalPaste(
  e: ClipboardEvent,
  effects: TerminalPasteEffects,
): Promise<void> | null {
  const data = e.clipboardData;
  // Ordinary paste — xterm inserts it, no interception and no IPC.
  if (clipboardHasUsableText(data)) return null;

  // Read the event before committing: it goes stale once we go async.
  const image = findPasteImage(data);

  e.preventDefault();
  e.stopImmediatePropagation();
  return (async () => {
    try {
      // A file *reference* beats image bytes: a Finder-copied `shot.png`
      // carries `public.file-url` and pastes its path, while a screenshot
      // or browser copy carries bytes only and keeps #79's attach flow.
      // Feature 55 decision 3.
      const paths = await effects.clipboardFilePaths();
      if (paths.length > 0) {
        await effects.injectStdin(formatPastedPaths(paths));
        return;
      }
      if (!image) return;
      const buf = await image.file.arrayBuffer();
      await effects.pasteImage(new Uint8Array(buf), image.mimeType);
      // Ctrl-V: claude-code / codex see it as they would in a host
      // terminal and attach with their native `[Image x]` placeholder.
      await effects.injectStdin("\x16");
    } catch (err) {
      effects.onError(String(err));
    }
  })();
}

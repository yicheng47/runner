import { describe, expect, it, vi } from "vitest";

import type { PasteImageMimeType } from "./api";
import {
  clipboardHasUsableText,
  formatPastedPaths,
  handleTerminalPaste,
  shellQuotePath,
} from "./terminalPaste";

function clipboard(text: string | null): DataTransfer {
  return {
    getData: (type: string) => (type === "text/plain" ? (text ?? "") : ""),
  } as unknown as DataTransfer;
}

describe("clipboardHasUsableText", () => {
  it("is true for an ordinary text paste, so xterm keeps handling it", () => {
    expect(clipboardHasUsableText(clipboard("/tmp/note.txt"))).toBe(true);
  });

  it("treats whitespace-only text as usable — it is still text to insert", () => {
    expect(clipboardHasUsableText(clipboard("  "))).toBe(true);
  });

  it("is false for a file copy, which puts no text on the clipboard", () => {
    expect(clipboardHasUsableText(clipboard(""))).toBe(false);
  });

  it("is false when the event carries no clipboardData at all", () => {
    expect(clipboardHasUsableText(null)).toBe(false);
  });

  it("is false when reading text/plain throws", () => {
    const hostile = {
      getData: () => {
        throw new Error("no access");
      },
    } as unknown as DataTransfer;
    expect(clipboardHasUsableText(hostile)).toBe(false);
  });
});

describe("shellQuotePath", () => {
  it("leaves an ordinary path bare so @-style completion still works", () => {
    expect(shellQuotePath("/Users/jason/go/src/runner/main.go")).toBe(
      "/Users/jason/go/src/runner/main.go",
    );
  });

  it("leaves the punctuation paths routinely carry bare", () => {
    expect(shellQuotePath("/tmp/a-b_c.d+e=f%g@h:i,j")).toBe(
      "/tmp/a-b_c.d+e=f%g@h:i,j",
    );
  });

  it("leaves non-ASCII bare — not a shell metacharacter", () => {
    expect(shellQuotePath("/Users/jason/文档/笔记.md")).toBe(
      "/Users/jason/文档/笔记.md",
    );
  });

  it("quotes a path with spaces", () => {
    expect(shellQuotePath("/Users/jason/My Documents/a.txt")).toBe(
      "'/Users/jason/My Documents/a.txt'",
    );
  });

  it("quotes shell metacharacters", () => {
    expect(shellQuotePath("/tmp/a$b.txt")).toBe("'/tmp/a$b.txt'");
    expect(shellQuotePath("/tmp/a;b.txt")).toBe("'/tmp/a;b.txt'");
    expect(shellQuotePath("/tmp/a&b.txt")).toBe("'/tmp/a&b.txt'");
    expect(shellQuotePath("/tmp/a*b.txt")).toBe("'/tmp/a*b.txt'");
    expect(shellQuotePath("/tmp/a(b).txt")).toBe("'/tmp/a(b).txt'");
    expect(shellQuotePath("/tmp/a`b`.txt")).toBe("'/tmp/a`b`.txt'");
    expect(shellQuotePath("/tmp/a|b.txt")).toBe("'/tmp/a|b.txt'");
  });

  it("escapes an embedded single quote by closing, escaping, reopening", () => {
    expect(shellQuotePath("/tmp/jason's file.txt")).toBe(
      "'/tmp/jason'\\''s file.txt'",
    );
  });

  it("quotes a path whose only oddity is a single quote", () => {
    expect(shellQuotePath("/tmp/it's.txt")).toBe("'/tmp/it'\\''s.txt'");
  });
});

describe("formatPastedPaths", () => {
  it("joins multiple paths with single spaces", () => {
    expect(formatPastedPaths(["/tmp/a.go", "/tmp/b.go"])).toBe(
      "/tmp/a.go /tmp/b.go",
    );
  });

  it("quotes only the paths that need it", () => {
    expect(formatPastedPaths(["/tmp/a.go", "/tmp/b c.go"])).toBe(
      "/tmp/a.go '/tmp/b c.go'",
    );
  });

  it("is empty for an empty pasteboard, so nothing gets injected", () => {
    expect(formatPastedPaths([])).toBe("");
  });
});

function imageFile(name: string, type: string, bytes = [1, 2, 3]): File {
  const buf = new Uint8Array(bytes).buffer;
  return {
    name,
    type,
    arrayBuffer: () => Promise.resolve(buf),
  } as unknown as File;
}

/** A ClipboardEvent stand-in: `text` is what text/plain yields, `files`
 *  are the `kind: "file"` items WKWebView materializes. */
function pasteEvent(
  text: string,
  files: { file: File; type: string }[] = [],
): ClipboardEvent & { preventDefault: ReturnType<typeof vi.fn> } {
  const items = files.map(({ file, type }) => ({
    kind: "file" as const,
    type,
    getAsFile: () => file,
  }));
  return {
    clipboardData: {
      getData: (t: string) => (t === "text/plain" ? text : ""),
      items: Object.assign(items, { length: items.length }),
    },
    preventDefault: vi.fn(),
    stopImmediatePropagation: vi.fn(),
  } as unknown as ClipboardEvent & {
    preventDefault: ReturnType<typeof vi.fn>;
  };
}

function effects(paths: string[]) {
  return {
    clipboardFilePaths: vi.fn(() => Promise.resolve(paths)),
    injectStdin: vi.fn(() => Promise.resolve()),
    pasteImage:
      vi.fn<(bytes: Uint8Array, mimeType: PasteImageMimeType) => Promise<void>>(
        () => Promise.resolve(),
      ),
    onError: vi.fn(),
  };
}

describe("handleTerminalPaste", () => {
  it("leaves an ordinary text paste to xterm — no preventDefault, no IPC", async () => {
    const e = pasteEvent("some copied text");
    const fx = effects(["/tmp/should-not-be-read.go"]);

    expect(handleTerminalPaste(e, fx)).toBeNull();

    expect(e.preventDefault).not.toHaveBeenCalled();
    expect(fx.clipboardFilePaths).not.toHaveBeenCalled();
    expect(fx.injectStdin).not.toHaveBeenCalled();
  });

  it("commits before the pasteboard round-trip even starts", async () => {
    // The load-bearing ordering: preventDefault after an await is a no-op,
    // because the browser has already run the default action by then. So
    // the handler cannot "call the command, then decide" — it must have
    // committed before `clipboardFilePaths` is so much as invoked.
    const e = pasteEvent("");
    const fx = effects(["/tmp/a.go"]);
    let committedBeforeRoundTrip: boolean | null = null;
    fx.clipboardFilePaths.mockImplementation(() => {
      committedBeforeRoundTrip = e.preventDefault.mock.calls.length > 0;
      return Promise.resolve(["/tmp/a.go"]);
    });

    const pending = handleTerminalPaste(e, fx);
    // Committed synchronously, before the returned promise can settle.
    expect(pending).not.toBeNull();
    expect(e.preventDefault).toHaveBeenCalledTimes(1);
    await pending;

    expect(committedBeforeRoundTrip).toBe(true);
  });

  it("injects the quoted paths through raw stdin, with no trailing Enter", async () => {
    const e = pasteEvent("");
    const fx = effects(["/tmp/a.go", "/tmp/b c.go"]);

    await handleTerminalPaste(e, fx);

    expect(fx.injectStdin).toHaveBeenCalledTimes(1);
    expect(fx.injectStdin).toHaveBeenCalledWith("/tmp/a.go '/tmp/b c.go'");
    expect(fx.pasteImage).not.toHaveBeenCalled();
  });

  it("pastes the path of a copied image FILE instead of attaching it", async () => {
    // Finder-copied shot.png: WKWebView exposes it as an image file AND
    // the pasteboard carries public.file-url. Decision 3 — path wins.
    const e = pasteEvent("", [
      { file: imageFile("shot.png", "image/png"), type: "image/png" },
    ]);
    const fx = effects(["/Users/jason/Desktop/shot.png"]);

    await handleTerminalPaste(e, fx);

    expect(fx.injectStdin).toHaveBeenCalledWith("/Users/jason/Desktop/shot.png");
    expect(fx.pasteImage).not.toHaveBeenCalled();
  });

  it("still attaches image BYTES when no file-url is present (#79)", async () => {
    // A screenshot or browser copy: bytes on the pasteboard, no file-url.
    const e = pasteEvent("", [
      { file: imageFile("image.png", "image/png", [9, 8, 7]), type: "image/png" },
    ]);
    const fx = effects([]);

    await handleTerminalPaste(e, fx);

    expect(fx.pasteImage).toHaveBeenCalledTimes(1);
    const [bytes, mimeType] = fx.pasteImage.mock.calls[0];
    expect(Array.from(bytes)).toEqual([9, 8, 7]);
    expect(mimeType).toBe("image/png");
    // Ctrl-V, so the agent runs its own attach flow.
    expect(fx.injectStdin).toHaveBeenCalledWith("\x16");
  });

  it("attaches a jpeg inferred from the filename when the item type is vague", async () => {
    const e = pasteEvent("", [
      { file: imageFile("photo.JPG", ""), type: "application/octet-stream" },
    ]);
    const fx = effects([]);

    await handleTerminalPaste(e, fx);

    expect(fx.pasteImage.mock.calls[0][1]).toBe("image/jpeg");
  });

  it("is a silent no-op when the clipboard holds neither text, paths, nor an image", async () => {
    const e = pasteEvent("");
    const fx = effects([]);

    await handleTerminalPaste(e, fx);

    // Swallowed the paste — but it had nothing to insert anyway.
    expect(e.preventDefault).toHaveBeenCalledTimes(1);
    expect(fx.injectStdin).not.toHaveBeenCalled();
    expect(fx.pasteImage).not.toHaveBeenCalled();
    expect(fx.onError).not.toHaveBeenCalled();
  });

  it("reports a failed pasteboard read instead of throwing", async () => {
    const e = pasteEvent("");
    const fx = effects([]);
    fx.clipboardFilePaths.mockRejectedValueOnce(new Error("pasteboard gone"));

    await handleTerminalPaste(e, fx);

    expect(fx.onError).toHaveBeenCalledTimes(1);
    expect(String(fx.onError.mock.calls[0][0])).toContain("pasteboard gone");
  });

  it("reports a failed injection instead of throwing", async () => {
    const e = pasteEvent("");
    const fx = effects(["/tmp/a.go"]);
    fx.injectStdin.mockRejectedValueOnce(new Error("session not found"));

    await handleTerminalPaste(e, fx);

    expect(fx.onError).toHaveBeenCalledTimes(1);
    expect(String(fx.onError.mock.calls[0][0])).toContain("session not found");
  });
});

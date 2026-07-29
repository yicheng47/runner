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
  it("is true for an ordinary text paste", () => {
    expect(clipboardHasUsableText(clipboard("/tmp/note.txt"))).toBe(true);
  });

  it("treats whitespace-only text as usable", () => {
    expect(clipboardHasUsableText(clipboard("  "))).toBe(true);
  });

  it("is false for a file copy with no text", () => {
    expect(clipboardHasUsableText(clipboard(""))).toBe(false);
  });

  it("is false when clipboardData is unavailable", () => {
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
  it("leaves an ordinary path bare", () => {
    expect(shellQuotePath("/Users/jason/go/src/runner/main.go")).toBe(
      "/Users/jason/go/src/runner/main.go",
    );
  });

  it("leaves routine path punctuation bare", () => {
    expect(shellQuotePath("/tmp/a-b_c.d+e=f%g@h:i,j")).toBe(
      "/tmp/a-b_c.d+e=f%g@h:i,j",
    );
  });

  it("leaves non-ASCII paths bare", () => {
    expect(shellQuotePath("/Users/jason/文档/笔记.md")).toBe(
      "/Users/jason/文档/笔记.md",
    );
  });

  it("quotes whitespace and shell metacharacters", () => {
    expect(shellQuotePath("/Users/jason/My Documents/a.txt")).toBe(
      "'/Users/jason/My Documents/a.txt'",
    );
    expect(shellQuotePath("/tmp/a$b.txt")).toBe("'/tmp/a$b.txt'");
    expect(shellQuotePath("/tmp/a;b.txt")).toBe("'/tmp/a;b.txt'");
    expect(shellQuotePath("/tmp/a&b.txt")).toBe("'/tmp/a&b.txt'");
    expect(shellQuotePath("/tmp/a*b.txt")).toBe("'/tmp/a*b.txt'");
    expect(shellQuotePath("/tmp/a(b).txt")).toBe("'/tmp/a(b).txt'");
    expect(shellQuotePath("/tmp/a`b`.txt")).toBe("'/tmp/a`b`.txt'");
    expect(shellQuotePath("/tmp/a|b.txt")).toBe("'/tmp/a|b.txt'");
  });

  it("escapes an embedded single quote", () => {
    expect(shellQuotePath("/tmp/jason's file.txt")).toBe(
      "'/tmp/jason'\\''s file.txt'",
    );
    expect(shellQuotePath("/tmp/it's.txt")).toBe("'/tmp/it'\\''s.txt'");
  });
});

describe("formatPastedPaths", () => {
  it("joins paths with single spaces and quotes only as needed", () => {
    expect(formatPastedPaths(["/tmp/a.go", "/tmp/b c.go"])).toBe(
      "/tmp/a.go '/tmp/b c.go'",
    );
  });

  it("is empty for an empty pasteboard", () => {
    expect(formatPastedPaths([])).toBe("");
  });
});

function clipboardFile(name: string, type: string, bytes = [1, 2, 3]): File {
  const buffer = new Uint8Array(bytes).buffer;
  return {
    name,
    type,
    arrayBuffer: () => Promise.resolve(buffer),
  } as unknown as File;
}

function pasteEvent(
  text: string,
  files: { file: File; type: string }[] = [],
): ClipboardEvent & {
  preventDefault: ReturnType<typeof vi.fn>;
  stopImmediatePropagation: ReturnType<typeof vi.fn>;
} {
  const items = files.map(({ file, type }) => ({
    kind: "file" as const,
    type,
    getAsFile: () => file,
  }));
  return {
    clipboardData: {
      getData: (type: string) => (type === "text/plain" ? text : ""),
      items: Object.assign(items, { length: items.length }),
    },
    preventDefault: vi.fn(),
    stopImmediatePropagation: vi.fn(),
  } as unknown as ClipboardEvent & {
    preventDefault: ReturnType<typeof vi.fn>;
    stopImmediatePropagation: ReturnType<typeof vi.fn>;
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
  it("leaves ordinary text to xterm without IPC", () => {
    const event = pasteEvent("some copied text");
    const fx = effects(["/tmp/should-not-be-read.go"]);

    expect(handleTerminalPaste(event, fx)).toBeNull();
    expect(event.preventDefault).not.toHaveBeenCalled();
    expect(event.stopImmediatePropagation).not.toHaveBeenCalled();
    expect(fx.clipboardFilePaths).not.toHaveBeenCalled();
    expect(fx.injectStdin).not.toHaveBeenCalled();
  });

  it("attaches image bytes without consulting file paths", async () => {
    const event = pasteEvent("", [
      {
        file: clipboardFile("image.png", "image/png", [9, 8, 7]),
        type: "image/png",
      },
    ]);
    const fx = effects([]);

    await handleTerminalPaste(event, fx);

    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(event.stopImmediatePropagation).toHaveBeenCalledTimes(1);
    expect(fx.clipboardFilePaths).not.toHaveBeenCalled();
    expect(fx.pasteImage).toHaveBeenCalledTimes(1);
    const [bytes, mimeType] = fx.pasteImage.mock.calls[0];
    expect(Array.from(bytes)).toEqual([9, 8, 7]);
    expect(mimeType).toBe("image/png");
    expect(fx.injectStdin).toHaveBeenCalledWith("\x16");
  });

  it("attaches a Finder-copied image file even when file URLs are present", async () => {
    const event = pasteEvent("", [
      {
        file: clipboardFile("shot.png", "image/png"),
        type: "image/png",
      },
    ]);
    const fx = effects(["/Users/jason/Desktop/shot.png"]);

    await handleTerminalPaste(event, fx);

    expect(fx.clipboardFilePaths).not.toHaveBeenCalled();
    expect(fx.pasteImage).toHaveBeenCalledTimes(1);
    expect(fx.injectStdin).toHaveBeenCalledTimes(1);
    expect(fx.injectStdin).toHaveBeenCalledWith("\x16");
  });

  it("keeps image-first precedence when the event also exposes text", async () => {
    const event = pasteEvent("file:///Users/jason/Desktop/shot.png", [
      {
        file: clipboardFile("shot.png", "", [4, 5, 6]),
        type: "application/octet-stream",
      },
    ]);
    const fx = effects(["/Users/jason/Desktop/shot.png"]);

    await handleTerminalPaste(event, fx);

    expect(fx.clipboardFilePaths).not.toHaveBeenCalled();
    expect(fx.pasteImage).toHaveBeenCalledTimes(1);
    expect(fx.pasteImage.mock.calls[0][1]).toBe("image/png");
    expect(fx.injectStdin).toHaveBeenCalledWith("\x16");
  });

  it("commits before the pasteboard round-trip starts", async () => {
    const event = pasteEvent("");
    const fx = effects(["/tmp/a.go"]);
    let committedBeforeRoundTrip = false;
    fx.clipboardFilePaths.mockImplementation(() => {
      committedBeforeRoundTrip = event.preventDefault.mock.calls.length > 0;
      return Promise.resolve(["/tmp/a.go"]);
    });

    const pending = handleTerminalPaste(event, fx);

    expect(pending).not.toBeNull();
    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(event.stopImmediatePropagation).toHaveBeenCalledTimes(1);
    await pending;
    expect(committedBeforeRoundTrip).toBe(true);
  });

  it("injects non-image file paths through raw stdin without Enter", async () => {
    const event = pasteEvent("", [
      {
        file: clipboardFile("main.go", "application/octet-stream"),
        type: "application/octet-stream",
      },
    ]);
    const fx = effects(["/tmp/a.go", "/tmp/b c.go"]);

    await handleTerminalPaste(event, fx);

    expect(fx.injectStdin).toHaveBeenCalledTimes(1);
    expect(fx.injectStdin).toHaveBeenCalledWith("/tmp/a.go '/tmp/b c.go'");
    expect(fx.pasteImage).not.toHaveBeenCalled();
  });

  it("is a committed no-op for an empty pasteboard", async () => {
    const event = pasteEvent("");
    const fx = effects([]);

    await handleTerminalPaste(event, fx);

    expect(event.preventDefault).toHaveBeenCalledTimes(1);
    expect(fx.clipboardFilePaths).toHaveBeenCalledTimes(1);
    expect(fx.injectStdin).not.toHaveBeenCalled();
    expect(fx.pasteImage).not.toHaveBeenCalled();
    expect(fx.onError).not.toHaveBeenCalled();
  });

  it("reports pasteboard and injection failures without throwing", async () => {
    const pasteboardEvent = pasteEvent("");
    const pasteboardFx = effects([]);
    pasteboardFx.clipboardFilePaths.mockRejectedValueOnce(
      new Error("pasteboard gone"),
    );

    await handleTerminalPaste(pasteboardEvent, pasteboardFx);

    expect(pasteboardFx.onError).toHaveBeenCalledWith(
      expect.stringContaining("pasteboard gone"),
    );

    const injectionEvent = pasteEvent("");
    const injectionFx = effects(["/tmp/a.go"]);
    injectionFx.injectStdin.mockRejectedValueOnce(
      new Error("session not found"),
    );

    await handleTerminalPaste(injectionEvent, injectionFx);

    expect(injectionFx.onError).toHaveBeenCalledWith(
      expect.stringContaining("session not found"),
    );
  });
});

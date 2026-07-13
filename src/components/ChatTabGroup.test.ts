import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { DirectSessionEntry } from "../lib/api";
import { applyPresetPure } from "../lib/paneLayout";
import { ChatTabGroup } from "./ChatTabGroup";

const session = {
  session_id: "A",
  handle: "coder",
  display_name: "Coder",
  title: null,
  pinned: false,
} as DirectSessionEntry;

describe("ChatTabGroup", () => {
  it("shows the layout pane count when some panes are empty", () => {
    const layout = {
      ...applyPresetPure("cols-3", "A", ["A"]),
      id: "tab-1",
    };
    const html = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [session],
        active: false,
        attention: null,
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );

    expect(html).toContain("lucide-columns-3");
    expect(html).toContain(">3</span>");
    expect(html).toContain('draggable="true"');
  });

  it("renders working and unread states in the fixed trailing slot", () => {
    const layout = {
      ...applyPresetPure("cols-2", "A", ["A"]),
      id: "tab-1",
    };
    const working = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [session],
        active: false,
        attention: "working",
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );
    expect(working).toContain('aria-label="Agent working"');
    expect(working).toContain("animate-spin");
    expect(working).toContain("origin-center");
    expect(working).toContain("text-fg-3");
    expect(working).not.toContain("text-accent");
    expect(working.indexOf('aria-label="Agent working"')).toBeLessThan(
      working.indexOf(">2</span>"),
    );

    const unread = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [session],
        active: false,
        attention: "unread",
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );
    expect(unread).toContain('aria-label="Completed — not viewed"');
  });
});

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
    const html = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout: applyPresetPure("cols-3", "A", ["A"]),
        members: [session],
        active: false,
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );

    expect(html).toContain("lucide-columns-3");
    expect(html).toContain(">3</span>");
  });
});

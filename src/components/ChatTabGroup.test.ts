import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { describe, expect, it } from "vitest";

import type { DirectSessionEntry } from "../lib/api";
import { applyPresetPure } from "../lib/paneLayout";
import { ChatTabGroup } from "./ChatTabGroup";
import { ChatAttentionIndicator } from "./SidebarTabRow";

const session = {
  session_id: "A",
  handle: "coder",
  display_name: "Coder",
  title: null,
  pinned: false,
  status: "running",
} as DirectSessionEntry;

const reviewerSession = {
  ...session,
  session_id: "B",
  handle: "reviewer",
  display_name: "Reviewer",
} as DirectSessionEntry;

describe("ChatTabGroup", () => {
  it("marks a multi-pane tab with a split icon and no count badge", () => {
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
    expect(html).not.toContain(">3</span>");
    expect(html).not.toContain("draggable=");
    expect(html).toContain('fill="none"');
  });

  it("colors a live tab icon without filling it", () => {
    const layout = {
      ...applyPresetPure("single", "A", ["A"]),
      id: "tab-1",
    };
    const html = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [session],
        active: true,
        attention: null,
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );

    expect(html).toContain('fill="none"');
    expect(html).toContain("text-accent");
  });

  it("keeps a live multi-pane icon outlined", () => {
    const layout = {
      ...applyPresetPure("cols-2", "A", ["A"]),
      id: "tab-1",
    };
    const html = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [session],
        active: true,
        attention: null,
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );

    expect(html).toContain("lucide-columns-2");
    expect(html).toContain('fill="none"');
    expect(html).toContain("text-accent");
    expect(html).not.toContain('fill="var(--color-accent)"');
  });

  it("renders the durable tab name as a controlled inline rename", () => {
    const layout = {
      ...applyPresetPure("cols-2", "A", ["A", "B"], "Review pair"),
      id: "tab-1",
    };
    const html = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [session, reviewerSession],
        active: false,
        attention: null,
        renaming: true,
        onActivate: () => {},
        onContextMenu: () => {},
        onRenameSubmit: () => {},
        onRenameCancel: () => {},
      }),
    );

    expect(html).toContain('value="Review pair"');
    expect(html).toContain('placeholder="@coder + @reviewer"');
    expect(html).toContain("lucide-columns-2");
  });

  it("shows the derived member label when the durable name is clear", () => {
    const layout = {
      ...applyPresetPure("cols-2", "A", ["A", "B"]),
      id: "tab-1",
    };
    const html = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [session, reviewerSession],
        active: false,
        attention: null,
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );

    expect(html).toContain("@coder + @reviewer");
  });

  it("mutes a fully stopped tab icon", () => {
    const layout = {
      ...applyPresetPure("single", "A", ["A"]),
      id: "tab-1",
    };
    const html = renderToStaticMarkup(
      createElement(ChatTabGroup, {
        layout,
        members: [{ ...session, status: "stopped" }],
        active: true,
        attention: null,
        onActivate: () => {},
        onContextMenu: () => {},
      }),
    );

    expect(html).toContain('fill="none"');
    expect(html).toContain("text-fg-2");
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

  it("supports a mission-specific working label", () => {
    const html = renderToStaticMarkup(
      createElement(ChatAttentionIndicator, {
        state: "working",
        workingLabel: "Mission working",
      }),
    );

    expect(html).toContain('aria-label="Mission working"');
    expect(html).toContain('title="Mission working"');
  });
});

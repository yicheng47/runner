import { createElement } from "react";
import { renderToStaticMarkup } from "react-dom/server";
import { Folder } from "lucide-react";
import { describe, expect, it } from "vitest";

import type { MissionSummary } from "../lib/types";
import { projectIdForTab } from "../lib/projectScope";
import { MissionRow, NewFolderRow } from "./Sidebar";
import { SidebarTabIcon } from "./SidebarTabRow";

const mission = {
  title: "Ship release",
  crew_name: "Build squad",
  any_session_live: true,
  all_sessions_live: true,
  activity: "busy",
  pinned_at: null,
} as MissionSummary;

const renderMissionRow = (
  renaming: boolean,
  selected = false,
  overrides: Partial<MissionSummary> = {},
) =>
  renderToStaticMarkup(
    createElement(MissionRow, {
      mission: { ...mission, ...overrides },
      selected,
      renaming,
      onClick: () => {},
      onContextMenu: () => {},
      onRenameSubmit: () => {},
      onRenameCancel: () => {},
    }),
  );

describe("MissionRow", () => {
  it("renders a leading flag before the title and trailing status", () => {
    const html = renderMissionRow(false);

    const flagIndex = html.indexOf("lucide-flag");
    const titleIndex = html.indexOf("Ship release");
    const statusIndex = html.indexOf('aria-label="Mission working"');
    expect(flagIndex).toBeGreaterThanOrEqual(0);
    expect(flagIndex).toBeLessThan(titleIndex);
    expect(titleIndex).toBeLessThan(statusIndex);
    expect(html).toContain('fill="none"');
    expect(html).not.toContain('fill="var(--color-accent)"');
  });

  it("keeps the flag while renaming", () => {
    const html = renderMissionRow(true);

    expect(html).toContain("lucide-flag");
    expect(html).toContain('value="Ship release"');
  });

  it("colors a live mission flag without filling it", () => {
    const html = renderMissionRow(false, true);

    expect(html).toContain('fill="none"');
    expect(html).toContain("text-accent");
  });

  it("mutes a partially resumed mission flag", () => {
    const html = renderMissionRow(false, true, {
      any_session_live: true,
      all_sessions_live: false,
    });

    expect(html).toContain("lucide-flag h-3 w-3 shrink-0 text-fg-2");
  });
});

describe("SidebarTabIcon", () => {
  it("colors a live-status icon without filling it", () => {
    const live = renderToStaticMarkup(
      createElement(SidebarTabIcon, {
        icon: Folder,
        active: true,
      }),
    );
    const stopped = renderToStaticMarkup(
      createElement(SidebarTabIcon, {
        icon: Folder,
        active: false,
      }),
    );

    expect(live).toContain('fill="none"');
    expect(live).toContain("text-accent");
    expect(stopped).toContain('fill="none"');
    expect(stopped).toContain("text-fg-2");
  });

  it("uses a muted outline while creating an empty folder", () => {
    const html = renderToStaticMarkup(
      createElement(NewFolderRow, {
        onSubmit: async () => {},
        onCancel: () => {},
      }),
    );

    expect(html).toContain("lucide-folder");
    expect(html).toContain('fill="none"');
    expect(html).toContain("text-fg-2");
  });
});

describe("projectIdForTab", () => {
  it("groups a tab only when every pane shares the same project", () => {
    expect(
      projectIdForTab([{ project_id: "project-a" }, { project_id: "project-a" }]),
    ).toBe("project-a");
    expect(
      projectIdForTab([{ project_id: "project-a" }, { project_id: null }]),
    ).toBeNull();
    expect(projectIdForTab([{ project_id: null }])).toBeNull();
  });
});

import { describe, expect, it } from "vitest";

import {
  buildChatListItems,
  derivedChatTabTitle,
  type ChatListItem,
} from "./chatTabs";
import { applyPresetPure } from "./paneLayout";

function rows(...ids: string[]): { session_id: string }[] {
  return ids.map((session_id) => ({ session_id }));
}

function titleMembers(
  ...members: {
    title?: string | null;
    handle?: string | null;
    display_name?: string;
  }[]
) {
  return members.map((member, index) => ({
    title: member.title ?? null,
    handle: member.handle ?? null,
    display_name: member.display_name ?? `Runner ${index + 1}`,
  }));
}

// Group → "[A,B]" (members in slot order); loose → "A".
function shape(items: ChatListItem<{ session_id: string }>[]): string[] {
  return items.map((item) =>
    item.kind === "group"
      ? `[${item.members.map((m) => m.session_id).join(",")}]`
      : item.row.session_id,
  );
}

describe("buildChatListItems", () => {
  it("emits a ≥2-member tab as a group in slot order, the rest loose", () => {
    const input = rows("A", "B", "X");
    const tab = applyPresetPure("cols-2", "A", ["A", "B"]);
    expect(shape(buildChatListItems(input, [tab]))).toEqual(["[A,B]", "X"]);
  });

  it("renders a single-member tab (one chat + empty pane) as a leaf", () => {
    const input = rows("A", "B");
    const tab = applyPresetPure("cols-2", "A", ["A"]);
    expect(shape(buildChatListItems(input, [tab]))).toEqual(["A", "B"]);
  });

  it("anchors the group at its best-sorted member, loose rows unchanged", () => {
    // Sort: P, B, X, A. Group [A,B] anchors at B (index 1, best member).
    const input = rows("P", "B", "X", "A");
    const tab = applyPresetPure("cols-2", "A", ["A", "B"]);
    expect(shape(buildChatListItems(input, [tab]))).toEqual([
      "P",
      "[A,B]",
      "X",
    ]);
  });

  it("keeps an unpinned group below the pinned cluster", () => {
    const input = rows("P1", "P2", "A", "X", "B");
    const tab = applyPresetPure("cols-2", "A", ["A", "B"]);
    expect(shape(buildChatListItems(input, [tab]))).toEqual([
      "P1",
      "P2",
      "[A,B]",
      "X",
    ]);
  });

  it("groups every multi-member tab, each anchored independently", () => {
    const input = rows("A", "B", "C", "D");
    const tabAB = applyPresetPure("cols-2", "A", ["A", "B"]);
    const tabCD = applyPresetPure("cols-2", "C", ["C", "D"]);
    expect(shape(buildChatListItems(input, [tabAB, tabCD]))).toEqual([
      "[A,B]",
      "[C,D]",
    ]);
  });

  it("lists a session once even if two tabs claim it", () => {
    // Pathological overlap: the first tab claims A, so the second drops to a
    // single member and falls back to a loose leaf. A is never double-listed.
    const input = rows("A", "B", "C");
    const tabAB = applyPresetPure("cols-2", "A", ["A", "B"]);
    const tabAC = applyPresetPure("cols-2", "A", ["A", "C"]);
    expect(shape(buildChatListItems(input, [tabAB, tabAC]))).toEqual([
      "[A,B]",
      "C",
    ]);
  });

  it("leaves the list flat when no tab has two members present", () => {
    const input = rows("A", "B", "C");
    expect(shape(buildChatListItems(input, []))).toEqual(["A", "B", "C"]);
  });

  it("ignores tab members that aren't in the row list", () => {
    // Archived / not-yet-loaded member: the tab drops to one present member.
    const input = rows("A", "X");
    const tab = applyPresetPure("cols-2", "A", ["A", "GONE"]);
    expect(shape(buildChatListItems(input, [tab]))).toEqual(["A", "X"]);
  });
});

describe("derivedChatTabTitle", () => {
  it("combines default-created chat titles for multi-chat tabs", () => {
    expect(
      derivedChatTabTitle(
        titleMembers(
          { title: "Chat with Codex", handle: "codex" },
          { title: "Chat with Claude", handle: "claude" },
        ),
      ),
    ).toBe("Chat with Codex + Chat with Claude");
  });

  it("falls back to handle then display name for unnamed chats", () => {
    expect(
      derivedChatTabTitle(
        titleMembers(
          { handle: "coder", display_name: "Codex" },
          { display_name: "Runtime chat" },
        ),
      ),
    ).toBe("@coder + Runtime chat");
  });
});

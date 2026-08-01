import { describe, expect, it } from "vitest";

import {
  INITIAL_MISSION_COMPOSER_STATE,
  missionComposerKeyDown,
  missionComposerMentionOptions,
  missionComposerMentionQuery,
  updateMissionComposerDraft,
  type MissionComposerRosterEntry,
} from "./missionComposer";

const roster: MissionComposerRosterEntry[] = [
  { handle: "coder", role: "coder", runtime: "codex" },
  { handle: "reviewer", role: "reviewer", runtime: "claude-code" },
];

describe("mission composer mention picker", () => {
  it("opens only for an @ typed at position zero", () => {
    const leading = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@",
    );
    expect(missionComposerMentionQuery(leading)).toBe("");
    expect(missionComposerMentionOptions(leading, roster)).toEqual(roster);

    const midText = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "hello @",
    );
    expect(missionComposerMentionQuery(midText)).toBeNull();
    expect(missionComposerMentionOptions(midText, roster)).toEqual([]);
  });

  it("filters handles by prefix without fuzzy matching", () => {
    const state = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@rev",
    );
    expect(
      missionComposerMentionOptions(state, roster).map((entry) => entry.handle),
    ).toEqual(["reviewer"]);

    const fuzzy = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@view",
    );
    expect(missionComposerMentionOptions(fuzzy, roster)).toEqual([]);
  });

  it("moves with arrow keys and commits the active row with Enter", () => {
    const open = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@",
    );
    const moved = missionComposerKeyDown(open, roster, "ArrowDown");
    expect(moved.preventDefault).toBe(true);
    expect(moved.state.activeIndex).toBe(1);

    const committed = missionComposerKeyDown(
      moved.state,
      roster,
      "Enter",
    );
    expect(committed.preventDefault).toBe(true);
    expect(committed.post).toBeNull();
    expect(committed.state.target).toBe("reviewer");
    expect(committed.state.draft).toBe("");
  });

  it("commits the filtered row with Tab", () => {
    const open = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@cod",
    );
    const committed = missionComposerKeyDown(open, roster, "Tab");
    expect(committed.preventDefault).toBe(true);
    expect(committed.state.target).toBe("coder");
    expect(committed.post).toBeNull();
  });

  it("commits an exact handle when typing through with Space", () => {
    const exact = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@ReViEwEr",
    );
    const committed = missionComposerKeyDown(exact, roster, " ");
    expect(committed.preventDefault).toBe(true);
    expect(committed.state.target).toBe("reviewer");
    expect(committed.state.draft).toBe("");

    const withText = updateMissionComposerDraft(
      committed.state,
      "fix the naming",
    );
    expect(missionComposerKeyDown(withText, roster, "Enter").post).toEqual({
      text: "fix the naming",
      to: "reviewer",
    });
  });

  it("lets partial mentions and Shift+Tab fall through", () => {
    const partial = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@rev",
    );
    expect(missionComposerKeyDown(partial, roster, " ")).toMatchObject({
      preventDefault: false,
      post: null,
    });
    expect(missionComposerKeyDown(partial, roster, "Tab", true)).toMatchObject(
      { preventDefault: false, post: null },
    );
  });

  it("leaves Shift+Enter to the textarea while the picker is open", () => {
    const open = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@rev",
    );
    const transition = missionComposerKeyDown(
      open,
      roster,
      "Enter",
      true,
    );
    expect(transition).toMatchObject({ preventDefault: false, post: null });
    expect(transition.state.target).toBeNull();
  });

  it("dismisses with Escape and keeps the typed text", () => {
    const open = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@rev",
    );
    const dismissed = missionComposerKeyDown(open, roster, "Escape");
    expect(dismissed.preventDefault).toBe(true);
    expect(dismissed.state.draft).toBe("@rev");
    expect(missionComposerMentionQuery(dismissed.state)).toBeNull();
    expect(missionComposerMentionOptions(dismissed.state, roster)).toEqual([]);
  });

  it("posts an unrecognized leading @word as plain broadcast text", () => {
    const unknown = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@nobody",
    );
    const transition = missionComposerKeyDown(unknown, roster, "Enter");
    expect(transition.preventDefault).toBe(true);
    expect(transition.post).toEqual({
      text: "@nobody",
      to: null,
    });
  });

  it("posts selected targets and leaves Shift+Enter to the textarea", () => {
    const open = updateMissionComposerDraft(
      INITIAL_MISSION_COMPOSER_STATE,
      "@reviewer",
    );
    const targeted = missionComposerKeyDown(open, roster, "Enter").state;
    const withText = updateMissionComposerDraft(
      targeted,
      "  Fix the naming\n",
    );

    expect(missionComposerKeyDown(withText, roster, "Enter").post).toEqual({
      text: "Fix the naming",
      to: "reviewer",
    });
    expect(
      missionComposerKeyDown(withText, roster, "Enter", true),
    ).toMatchObject({ preventDefault: false, post: null });
  });
});

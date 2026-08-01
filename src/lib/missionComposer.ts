export interface MissionComposerRosterEntry {
  handle: string;
  role: string;
  runtime: string;
}

export interface MissionComposerState {
  draft: string;
  target: string | null;
  pickerDismissed: boolean;
  activeIndex: number;
}

export const INITIAL_MISSION_COMPOSER_STATE: MissionComposerState = {
  draft: "",
  target: null,
  pickerDismissed: false,
  activeIndex: 0,
};

export interface MissionComposerPost {
  text: string;
  to: string | null;
}

export interface MissionComposerKeyTransition {
  state: MissionComposerState;
  preventDefault: boolean;
  post: MissionComposerPost | null;
}

function leadingMentionQuery(draft: string): string | null {
  const match = /^@([^\s]*)$/.exec(draft);
  return match ? match[1] : null;
}

export function missionComposerMentionQuery(
  state: MissionComposerState,
): string | null {
  if (state.target || state.pickerDismissed) return null;
  return leadingMentionQuery(state.draft);
}

export function missionComposerMentionOptions(
  state: MissionComposerState,
  roster: MissionComposerRosterEntry[],
): MissionComposerRosterEntry[] {
  const query = missionComposerMentionQuery(state);
  if (query === null) return [];
  const prefix = query.toLowerCase();
  return roster.filter((entry) =>
    entry.handle.toLowerCase().startsWith(prefix),
  );
}

export function updateMissionComposerDraft(
  state: MissionComposerState,
  draft: string,
): MissionComposerState {
  const stayedInMention =
    leadingMentionQuery(state.draft) !== null &&
    leadingMentionQuery(draft) !== null;
  return {
    ...state,
    draft,
    pickerDismissed: stayedInMention ? state.pickerDismissed : false,
    activeIndex: 0,
  };
}

export function selectMissionComposerTarget(
  handle: string,
): MissionComposerState {
  return {
    draft: "",
    target: handle,
    pickerDismissed: false,
    activeIndex: 0,
  };
}

export function missionComposerKeyDown(
  state: MissionComposerState,
  roster: MissionComposerRosterEntry[],
  key: string,
  shiftKey = false,
): MissionComposerKeyTransition {
  const mentionQuery = missionComposerMentionQuery(state);
  const options = missionComposerMentionOptions(state, roster);
  const pickerOpen = options.length > 0;
  const exactMention =
    mentionQuery === null
      ? null
      : roster.find(
          (entry) => entry.handle.toLowerCase() === mentionQuery.toLowerCase(),
        ) ?? null;

  if (pickerOpen && key === " " && exactMention) {
    return {
      state: selectMissionComposerTarget(exactMention.handle),
      preventDefault: true,
      post: null,
    };
  }

  if (pickerOpen && (key === "ArrowDown" || key === "ArrowUp")) {
    const offset = key === "ArrowDown" ? 1 : -1;
    const activeIndex =
      (Math.min(state.activeIndex, options.length - 1) +
        offset +
        options.length) %
      options.length;
    return {
      state: { ...state, activeIndex },
      preventDefault: true,
      post: null,
    };
  }

  if (
    pickerOpen &&
    ((key === "Enter" && !shiftKey) || (key === "Tab" && !shiftKey))
  ) {
    const option = options[Math.min(state.activeIndex, options.length - 1)];
    return {
      state: selectMissionComposerTarget(option.handle),
      preventDefault: true,
      post: null,
    };
  }

  if (pickerOpen && key === "Escape") {
    return {
      state: { ...state, pickerDismissed: true },
      preventDefault: true,
      post: null,
    };
  }

  if (key === "Backspace" && state.target && state.draft === "") {
    return {
      state: { ...state, target: null },
      preventDefault: true,
      post: null,
    };
  }

  if (key === "Enter" && !shiftKey) {
    return {
      state,
      preventDefault: true,
      post: state.draft.trim()
        ? { text: state.draft.trim(), to: state.target }
        : null,
    };
  }

  return { state, preventDefault: false, post: null };
}

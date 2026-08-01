import { useEffect, useMemo, useRef, useState } from "react";

import { api } from "../lib/api";
import {
  INITIAL_MISSION_COMPOSER_STATE,
  missionComposerKeyDown,
  missionComposerMentionOptions,
  missionComposerMentionQuery,
  selectMissionComposerTarget,
  updateMissionComposerDraft,
  type MissionComposerPost,
  type MissionComposerRosterEntry,
} from "../lib/missionComposer";

const MAX_TEXTAREA_HEIGHT = 240;

export function MissionInput({
  missionId,
  roster,
  onError,
}: {
  missionId: string;
  roster: MissionComposerRosterEntry[];
  onError?: (message: string) => void;
}) {
  const [state, setState] = useState(INITIAL_MISSION_COMPOSER_STATE);
  const [posting, setPosting] = useState(false);
  const [textareaScrollable, setTextareaScrollable] = useState(false);
  const textareaRef = useRef<HTMLTextAreaElement | null>(null);
  const options = useMemo(
    () => missionComposerMentionOptions(state, roster),
    [roster, state],
  );
  const pickerOpen =
    missionComposerMentionQuery(state) !== null && options.length > 0;
  const activeIndex = Math.min(state.activeIndex, options.length - 1);

  useEffect(() => {
    const textarea = textareaRef.current;
    if (!textarea) return;
    textarea.style.height = "auto";
    textarea.style.height = `${Math.min(
      textarea.scrollHeight,
      MAX_TEXTAREA_HEIGHT,
    )}px`;
    setTextareaScrollable(textarea.scrollHeight > MAX_TEXTAREA_HEIGHT);
  }, [state.draft]);

  const focusTextarea = () => {
    window.requestAnimationFrame(() => textareaRef.current?.focus());
  };

  const selectTarget = (handle: string) => {
    setState(selectMissionComposerTarget(handle));
    focusTextarea();
  };

  const post = async (message: MissionComposerPost) => {
    if (posting) return;
    setPosting(true);
    try {
      await api.mission.postMessage(
        missionId,
        message.text,
        message.to ?? undefined,
      );
      setState(INITIAL_MISSION_COMPOSER_STATE);
    } catch (error) {
      onError?.(String(error));
    } finally {
      setPosting(false);
      focusTextarea();
    }
  };

  const sendDraft = () => {
    const text = state.draft.trim();
    if (!text) return;
    void post({ text, to: state.target });
  };

  return (
    <div className="shrink-0 border-t border-line bg-bg px-10 pb-5 pt-3.5">
      <div className="relative">
        {pickerOpen ? (
          <div
            id="mission-composer-roster"
            role="listbox"
            aria-label="Mission roster"
            className="absolute bottom-full left-0 z-30 mb-2 flex w-[380px] max-w-full flex-col gap-0.5 rounded-lg border border-line-strong bg-panel p-1.5 shadow-[0_4px_16px_rgba(0,0,0,0.4)]"
          >
            <div className="px-2 pb-0.5 pt-1 text-[10px] font-semibold tracking-[1.5px] text-fg-3">
              ROSTER
            </div>
            {options.map((entry, index) => {
              const active = index === activeIndex;
              return (
                <button
                  id={`mission-composer-option-${entry.handle}`}
                  key={entry.handle}
                  type="button"
                  role="option"
                  aria-selected={active}
                  onMouseDown={(event) => event.preventDefault()}
                  onClick={() => selectTarget(entry.handle)}
                  className={`flex cursor-pointer items-center gap-2 rounded px-2 py-1.5 text-left ${
                    active
                      ? "border border-line-strong bg-raised"
                      : "border border-transparent hover:bg-raised/60"
                  }`}
                >
                  <span className="font-mono text-[12px] font-semibold text-accent">
                    @{entry.handle}
                  </span>
                  <span className="min-w-0 truncate text-[11px] text-fg-2">
                    {entry.role} · {entry.runtime}
                  </span>
                  {active ? (
                    <span className="ml-auto font-mono text-[10px] text-fg-3">
                      ↵
                    </span>
                  ) : null}
                </button>
              );
            })}
          </div>
        ) : null}

        <div className="flex items-center gap-3 rounded-lg border border-line bg-panel px-4 py-3 focus-within:border-line-strong">
          {state.target ? (
            <button
              type="button"
              title={`Remove @${state.target} target`}
              onClick={() => {
                setState((current) => ({ ...current, target: null }));
                focusTextarea();
              }}
              className="shrink-0 cursor-pointer rounded bg-accent/15 px-1.5 py-0.5 font-mono text-[12px] font-semibold text-accent"
            >
              @{state.target}
            </button>
          ) : null}
          <textarea
            ref={textareaRef}
            aria-label="Message the crew"
            aria-autocomplete="list"
            aria-controls={pickerOpen ? "mission-composer-roster" : undefined}
            aria-expanded={pickerOpen}
            aria-activedescendant={
              pickerOpen
                ? `mission-composer-option-${options[activeIndex].handle}`
                : undefined
            }
            rows={1}
            disabled={posting}
            value={state.draft}
            placeholder="Message the crew — @handle to address one runner"
            onChange={(event) =>
              setState((current) =>
                updateMissionComposerDraft(current, event.target.value),
              )
            }
            onKeyDown={(event) => {
              const transition = missionComposerKeyDown(
                state,
                roster,
                event.key,
                event.shiftKey,
              );
              if (transition.preventDefault) event.preventDefault();
              if (transition.state !== state) setState(transition.state);
              if (transition.post) void post(transition.post);
            }}
            className={`min-w-0 flex-1 resize-none overflow-x-hidden bg-transparent text-[13px] leading-6 text-fg outline-none placeholder:text-fg-3 disabled:cursor-not-allowed disabled:opacity-60 ${
              textareaScrollable ? "overflow-y-auto pr-2" : "overflow-y-hidden"
            }`}
            style={{
              minHeight: "24px",
              maxHeight: `${MAX_TEXTAREA_HEIGHT}px`,
            }}
          />
          <button
            type="button"
            title="Send message (Enter)"
            disabled={!state.draft.trim() || posting}
            onClick={sendDraft}
            className="shrink-0 rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-accent-ink transition-opacity disabled:cursor-not-allowed disabled:opacity-50"
          >
            Send
          </button>
        </div>
      </div>
    </div>
  );
}

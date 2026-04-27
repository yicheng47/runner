// Slack-style channel input docked at the bottom of the workspace center
// column. Submitting always emits a `human_said` *signal* (not a message
// event) so the router wakes the recipient — per arch §5.5.0, messages
// are pull-based and never trigger router actions.
//
// Recipient semantics: default is `@<lead>`. Switching the chip to a
// non-lead handle scopes `payload.target` to that worker; clearing the
// chip with the × icon broadcasts (omits payload.target — the router
// then defaults to the lead anyway, so for v0 the behaviour is the same
// as the default).

import { useEffect, useRef, useState } from "react";

import { api } from "../lib/api";

interface MissionInputProps {
  missionId: string;
  /** Stable lead handle — default recipient. */
  leadHandle: string;
  /** All non-human handles in the roster. The recipient picker offers
   *  these; the human can also broadcast (clear the chip). */
  handles: string[];
  /** Set when the mission is no longer running. The input still renders
   *  (so the feed below it stays visible) but submission is disabled. */
  disabled?: boolean;
  onError?: (msg: string) => void;
}

export function MissionInput({
  missionId,
  leadHandle,
  handles,
  disabled,
  onError,
}: MissionInputProps) {
  const [text, setText] = useState("");
  const [target, setTarget] = useState<string | null>(leadHandle);
  const [pickerOpen, setPickerOpen] = useState(false);
  const [submitting, setSubmitting] = useState(false);
  const pickerRef = useRef<HTMLDivElement | null>(null);

  // Sync with new lead if the prop changes (rare — but harmless to keep).
  useEffect(() => {
    setTarget((cur) => (cur === null ? null : leadHandle));
  }, [leadHandle]);

  useEffect(() => {
    if (!pickerOpen) return;
    const onDoc = (e: MouseEvent) => {
      if (!pickerRef.current?.contains(e.target as Node)) setPickerOpen(false);
    };
    window.addEventListener("mousedown", onDoc);
    return () => window.removeEventListener("mousedown", onDoc);
  }, [pickerOpen]);

  async function submit() {
    const body = text.trim();
    if (!body || submitting || disabled) return;
    setSubmitting(true);
    try {
      const payload = target ? { text: body, target } : { text: body };
      await api.mission.postHumanSignal({
        mission_id: missionId,
        signal_type: "human_said",
        payload,
      });
      setText("");
    } catch (e) {
      onError?.(String(e));
    } finally {
      setSubmitting(false);
    }
  }

  function onKeyDown(e: React.KeyboardEvent<HTMLTextAreaElement>) {
    // Enter sends, Shift-Enter inserts a newline. Mirrors the Slack /
    // Linear chat affordance the design references in frame `cFfYe`.
    if (e.key === "Enter" && !e.shiftKey) {
      e.preventDefault();
      void submit();
    }
  }

  return (
    <div className="flex flex-col gap-2.5 border-t border-line bg-bg px-10 pb-5 pt-3.5">
      <div className="flex items-center gap-2.5 text-[11px] text-fg-2">
        <span>Post to mission log as</span>
        <span className="rounded bg-warn/20 px-2 py-0.5 text-[11px] font-semibold text-warn">
          @human
        </span>
        <span className="text-fg-3">· visible to the crew</span>
      </div>
      <div className="flex flex-wrap items-center gap-2 text-[11px]" ref={pickerRef}>
        <span className="text-fg-3">to:</span>
        <button
          type="button"
          onClick={() => setPickerOpen((v) => !v)}
          className="inline-flex items-center gap-1.5 rounded border border-line bg-panel px-2.5 py-1 font-mono text-[11px] text-fg hover:border-line-strong"
        >
          {target ? (
            <>
              @{target}
              {target === leadHandle ? (
                <span className="text-fg-3">(lead)</span>
              ) : null}
            </>
          ) : (
            <span className="text-fg-2">broadcast</span>
          )}
          <span className="text-fg-3">▾</span>
        </button>
        {target ? (
          <button
            type="button"
            onClick={() => setTarget(null)}
            title="Clear recipient (broadcast)"
            className="text-fg-3 hover:text-fg"
          >
            ×
          </button>
        ) : null}
        {pickerOpen ? (
          <div className="relative">
            <div className="absolute left-0 top-1.5 z-20 flex w-44 flex-col overflow-hidden rounded border border-line-strong bg-panel py-1 shadow-xl">
              {handles.map((h) => (
                <button
                  key={h}
                  type="button"
                  onClick={() => {
                    setTarget(h);
                    setPickerOpen(false);
                  }}
                  className={`flex w-full items-center justify-between px-3 py-1.5 text-left font-mono text-[11px] hover:bg-raised ${
                    h === target ? "text-accent" : "text-fg"
                  }`}
                >
                  <span>@{h}</span>
                  {h === leadHandle ? (
                    <span className="text-[10px] text-fg-3">lead</span>
                  ) : null}
                </button>
              ))}
              <div className="border-t border-line my-1" />
              <button
                type="button"
                onClick={() => {
                  setTarget(null);
                  setPickerOpen(false);
                }}
                className="px-3 py-1.5 text-left text-[11px] text-fg-2 hover:bg-raised"
              >
                broadcast
              </button>
            </div>
          </div>
        ) : null}
      </div>
      <div className="flex items-end gap-3 rounded-lg border border-line bg-panel px-4 py-3">
        <textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={onKeyDown}
          placeholder={
            disabled
              ? "Mission stopped — input disabled."
              : "Talk to the crew…"
          }
          disabled={disabled}
          rows={1}
          className="flex-1 resize-none bg-transparent text-[13px] text-fg placeholder:text-fg-3 focus:outline-none disabled:cursor-not-allowed"
          style={{ minHeight: "24px", maxHeight: "240px" }}
        />
        <button
          type="button"
          onClick={() => void submit()}
          disabled={!text.trim() || submitting || disabled}
          className="rounded-md bg-accent px-3 py-1.5 text-[12px] font-semibold text-accent-ink transition-opacity disabled:cursor-not-allowed disabled:opacity-50"
        >
          Send
        </button>
      </div>
      <div className="flex flex-wrap gap-3.5 text-[11px] text-fg-3">
        <span>↵ send</span>
        <span>⇧↵ newline</span>
      </div>
    </div>
  );
}

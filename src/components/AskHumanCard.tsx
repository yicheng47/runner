// One human-input card. The router emits a `human_question` signal in
// response to a worker's `ask_human`; the workspace replays the feed,
// finds those events, and renders a card per still-pending question.
//
// Clicking a choice posts a `human_response` signal back through the
// event log (router -> asker injection happens parent-side).

import { useState } from "react";

import { api } from "../lib/api";
import type { HumanQuestionPayload } from "../lib/types";

interface AskHumanCardProps {
  missionId: string;
  /** The `human_question` event id — used as `payload.question_id` per
   *  arch §5.5.0 (the card id is the canonical question_id). */
  questionId: string;
  /** Handle of the runner that emitted the originating `ask_human`. The
   *  router routes the response back to this handle's PTY. */
  asker: string;
  payload: HumanQuestionPayload;
  ts: string;
  /** When set, the card has been answered already (a `human_response`
   *  with matching question_id was found in the feed). The card stays
   *  visible read-only so reviewers can see the historical choice. */
  resolvedChoice?: string | null;
  onError?: (msg: string) => void;
}

export function AskHumanCard({
  missionId,
  questionId,
  asker,
  payload,
  ts,
  resolvedChoice,
  onError,
}: AskHumanCardProps) {
  const [submitting, setSubmitting] = useState(false);
  const [pickedChoice, setPickedChoice] = useState<string | null>(null);
  const choices = payload.choices && payload.choices.length > 0
    ? payload.choices
    : ["yes", "no"]; // sensible fallback so the card always has buttons

  const onBehalf = payload.on_behalf_of;
  // Attribution chain: when an `ask_human` carries `on_behalf_of`, the
  // worker who originally asked is shown alongside the lead so the human
  // sees `*@impl → @architect → you*` (per design frame `Z7Dbo`).
  const chain = onBehalf ? `@${onBehalf} → @${asker} → you` : `@${asker} → you`;

  const resolved = resolvedChoice != null;

  async function submit(choice: string) {
    if (submitting || resolved) return;
    setSubmitting(true);
    setPickedChoice(choice);
    try {
      await api.mission.postHumanSignal({
        mission_id: missionId,
        signal_type: "human_response",
        payload: { question_id: questionId, choice },
      });
    } catch (e) {
      setPickedChoice(null);
      onError?.(String(e));
    } finally {
      setSubmitting(false);
    }
  }

  return (
    <div className="rounded-lg border border-warn/60 bg-warn/10 p-4">
      <div className="flex items-baseline justify-between gap-2">
        <div className="flex items-baseline gap-2 min-w-0">
          <span className="text-[12px] font-semibold text-warn">
            needs your input
          </span>
          <span className="text-[11px] font-mono text-fg-2 truncate">
            {chain}
          </span>
        </div>
        <span className="text-[11px] text-fg-3">{formatTs(ts)}</span>
      </div>
      <p className="mt-2 text-[13px] leading-relaxed text-fg">
        {payload.prompt || "(no prompt)"}
      </p>
      <div className="mt-3 flex flex-wrap items-center gap-2">
        {choices.map((c, idx) => {
          const isPrimary = idx === 0;
          const isPicked = resolved
            ? c === resolvedChoice
            : pickedChoice === c;
          return (
            <button
              key={c}
              type="button"
              onClick={() => submit(c)}
              disabled={submitting || resolved}
              className={
                isPrimary
                  ? `rounded-md px-3.5 py-1.5 text-[12px] font-semibold transition-colors ${
                      isPicked
                        ? "bg-accent text-accent-ink"
                        : "bg-accent text-accent-ink hover:bg-accent/90"
                    } disabled:cursor-not-allowed disabled:opacity-60`
                  : `rounded-md border border-line bg-panel px-3.5 py-1.5 text-[12px] font-medium text-fg transition-colors ${
                      isPicked ? "border-accent text-accent" : ""
                    } hover:border-line-strong disabled:cursor-not-allowed disabled:opacity-60`
              }
            >
              {c}
            </button>
          );
        })}
        {resolved ? (
          <span className="ml-1 text-[11px] text-fg-3">
            answered: <span className="text-fg-2">{resolvedChoice}</span>
          </span>
        ) : null}
      </div>
    </div>
  );
}

function formatTs(ts: string): string {
  // Display the local-time HH:MM:SS slice only — the workspace feed is
  // always anchored to "now" so the date is implied.
  try {
    const d = new Date(ts);
    return d.toLocaleTimeString();
  } catch {
    return ts;
  }
}

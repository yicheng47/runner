// One human-input card. The router emits a `human_question` signal in
// response to a worker's `ask_human`; the workspace replays the feed,
// finds those events, and renders a card per still-pending question.
//
// Clicking a choice posts a `human_response` signal back through the
// event log (router -> asker injection happens parent-side).

import { useState } from "react";

import { api } from "../lib/api";
import type { HumanQuestionPayload } from "../lib/types";
import { MessageBody } from "./MessageBody";
import { RunnerAvatar, hueForSeed } from "./ui/RunnerAvatar";

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
  const chain = onBehalf ? `@${onBehalf} → @${asker} → you` : "→ you";

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
    <div className="flex gap-3 px-4">
      <RunnerAvatar seed={asker} size={35} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2">
          <span
            className="truncate font-mono text-[13px] font-semibold"
            style={{ color: hueForSeed(asker) }}
          >
            @{asker}
          </span>
          <span className="shrink-0 rounded bg-warn/10 px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-[0.06em] text-warn">
            Needs your input
          </span>
          <span className="truncate font-mono text-[11px] text-fg-2">
            {chain}
          </span>
          <span className="shrink-0 text-[11px] text-fg-3">
            {formatTs(ts)}
          </span>
        </div>
        <div className="mt-1.5 rounded-lg border border-warn/60 bg-warn/10 p-4">
          <div className="text-[13px] leading-relaxed text-fg">
            {payload.prompt ? (
              <MessageBody text={payload.prompt} />
            ) : (
              <span className="text-fg-3">(no prompt)</span>
            )}
          </div>
          <div className="mt-3 flex flex-col gap-1.5">
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
                      ? `flex w-full cursor-pointer items-center rounded-md px-3.5 py-2.5 text-left text-[12px] font-semibold transition-all ${
                          isPicked
                            ? "bg-accent text-accent-ink"
                            : "bg-accent text-accent-ink hover:bg-accent/90 hover:shadow-[0_0_0_1px_var(--color-accent)] hover:-translate-y-px"
                        } disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:translate-y-0 disabled:hover:shadow-none`
                      : `flex w-full cursor-pointer items-center rounded-md border border-line bg-panel px-3.5 py-2.5 text-left text-[12px] font-medium text-fg transition-all ${
                          isPicked ? "border-accent text-accent" : ""
                        } hover:border-fg-3 hover:bg-raised hover:text-fg disabled:cursor-not-allowed disabled:opacity-60 disabled:hover:bg-panel disabled:hover:border-line`
                  }
                >
                  {c}
                </button>
              );
            })}
            {resolved ? (
              <span className="mt-1 text-[11px] text-fg-3">
                answered: <span className="text-fg-2">{resolvedChoice}</span>
              </span>
            ) : null}
          </div>
        </div>
      </div>
    </div>
  );
}

function formatTs(ts: string): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

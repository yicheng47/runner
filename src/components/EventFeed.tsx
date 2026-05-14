// Scrollable feed that renders one row per appended event. Three variants
// align with arch §5.2 + design frame `5DIkl`:
//   - message rows (kind: message): from / to / payload.text
//   - signal rows (kind: signal): from / signal type, JSON-ish payload
//   - ask-human cards (signal type: human_question): rich card variant
//     consumed by AskHumanCard
//
// Router-internal noise (`mission_warning`, `inbox_read`) is muted: the
// rows are still present but de-emphasized so they don't dominate the
// feed. We never silently drop events — the audit-trail invariant means
// every line in the log surfaces somewhere.

import { useEffect, useRef } from "react";

import { AskHumanCard } from "./AskHumanCard";
import { MessageBody } from "./MessageBody";
import type { Event, HumanQuestionPayload } from "../lib/types";

interface EventFeedProps {
  missionId: string;
  events: Event[];
  /** question_id → choice. When a question has been answered, the card
   *  goes read-only with the choice surfaced. */
  resolvedAsks: Record<string, string>;
  /** asker handle for each pending `human_question`, derived in the
   *  workspace by walking ask_human → human_question chains. */
  askersByQuestion: Record<string, string>;
  /** Whether this pane is the visible tab. When the pane flips from
   *  hidden → visible we re-anchor to the bottom if the user was parked
   *  there before tab-switching away. `onScroll` can't fire while
   *  `display: none`, so `wasNearBottomRef` is still the pre-flip value. */
  active: boolean;
  onError?: (msg: string) => void;
}

export function EventFeed({
  missionId,
  events,
  resolvedAsks,
  askersByQuestion,
  active,
  onError,
}: EventFeedProps) {
  const scrollRef = useRef<HTMLDivElement | null>(null);
  const wasNearBottomRef = useRef(true);

  // Auto-stick to the bottom unless the user has scrolled away. Without
  // this the feed just keeps appending offscreen and the workspace looks
  // dead from the human's perspective.
  useEffect(() => {
    const el = scrollRef.current;
    if (!el) return;
    if (wasNearBottomRef.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [events.length]);

  // Re-anchor on tab-back: events that arrived while the pane was
  // `display: none` don't trigger the append effect's scroll write
  // because layout was stale; once we're visible again we land the user
  // at the bottom if that's where they were parked.
  useEffect(() => {
    if (!active) return;
    const el = scrollRef.current;
    if (!el) return;
    if (wasNearBottomRef.current) {
      el.scrollTop = el.scrollHeight;
    }
  }, [active]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    wasNearBottomRef.current = distance < 80;
  };

  return (
    <div
      ref={scrollRef}
      onScroll={onScroll}
      className="flex flex-1 flex-col gap-4 overflow-y-auto px-10 py-6"
    >
      {events.length === 0 ? (
        <p className="text-[12px] text-fg-3">No events yet.</p>
      ) : (
        events.map((ev) => (
          <EventRow
            key={ev.id}
            event={ev}
            missionId={missionId}
            resolvedAsks={resolvedAsks}
            askersByQuestion={askersByQuestion}
            onError={onError}
          />
        ))
      )}
    </div>
  );
}

function EventRow({
  event,
  missionId,
  resolvedAsks,
  askersByQuestion,
  onError,
}: {
  event: Event;
  missionId: string;
  resolvedAsks: Record<string, string>;
  askersByQuestion: Record<string, string>;
  onError?: (msg: string) => void;
}) {
  // human_question — render the rich card. Asker derived from the
  // originating ask_human (via askersByQuestion). Fall back to the
  // event's `from` (which is "router") if we haven't paired it yet.
  if (event.kind === "signal" && event.type === "human_question") {
    const payload = event.payload as HumanQuestionPayload;
    const asker = askersByQuestion[event.id] ?? "?";
    const resolvedChoice = resolvedAsks[event.id] ?? null;
    return (
      <AskHumanCard
        missionId={missionId}
        questionId={event.id}
        asker={asker}
        payload={payload}
        ts={event.ts}
        resolvedChoice={resolvedChoice}
        onError={onError}
      />
    );
  }

  if (event.kind === "message") {
    const text = (event.payload as { text?: string })?.text ?? "";
    return (
      <div className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2 text-[11px] text-fg-3">
          <span className="font-mono text-[12px] font-semibold text-accent">
            @{event.from}
          </span>
          <span>message</span>
          {event.to ? (
            <>
              <span>→</span>
              <span className="font-mono text-fg-2">@{event.to}</span>
            </>
          ) : null}
          <span>·</span>
          <span>{formatTs(event.ts)}</span>
        </div>
        <div className="text-[13px] leading-relaxed text-fg">
          <MessageBody text={text} />
        </div>
      </div>
    );
  }

  // Default signal row.
  const isQuiet =
    event.type === "inbox_read" ||
    event.type === "mission_warning" ||
    event.type === "runner_status";
  return (
    <div
      className={`flex flex-col gap-1 ${
        isQuiet ? "opacity-60" : ""
      }`}
    >
      <div className="flex items-baseline gap-2 text-[11px] text-fg-3">
        <span className="font-mono text-[12px] font-semibold text-accent">
          @{event.from}
        </span>
        <span>signal · {event.type ?? "?"}</span>
        <span>·</span>
        <span>{formatTs(event.ts)}</span>
      </div>
      <div className="rounded-md border border-line bg-bg p-3 font-mono text-[12px] leading-snug text-fg-2">
        {renderPayload(event)}
      </div>
    </div>
  );
}

function renderPayload(event: Event): React.ReactNode {
  // Common signal shapes get shorter inline renderings; everything else
  // falls back to formatted JSON. The v0 router never emits opaque blobs
  // so this stays readable.
  const p = event.payload as Record<string, unknown> | null | undefined;
  if (!p || typeof p !== "object") {
    return <span>{String(p ?? "")}</span>;
  }
  if (event.type === "mission_goal" || event.type === "human_said") {
    const text = typeof p.text === "string" ? p.text : "";
    const target = typeof p.target === "string" ? p.target : null;
    return (
      <div className="text-fg">
        {text ? <MessageBody text={text} /> : <span>(no text)</span>}
        {target ? (
          <div className="mt-1 text-fg-3">→ @{target}</div>
        ) : null}
      </div>
    );
  }
  if (event.type === "ask_lead") {
    const q = typeof p.question === "string" ? p.question : "";
    return <span className="text-fg">{q}</span>;
  }
  if (event.type === "runner_status") {
    const state = typeof p.state === "string" ? p.state : "?";
    const note = typeof p.note === "string" ? ` — ${p.note}` : "";
    return (
      <span>
        {state}
        {note}
      </span>
    );
  }
  if (event.type === "human_response") {
    const choice = typeof p.choice === "string" ? p.choice : "";
    const qid = typeof p.question_id === "string" ? p.question_id : "";
    return (
      <span>
        chose <span className="text-fg">{choice || "?"}</span>
        {qid ? <span className="text-fg-3"> · q={qid.slice(-6)}</span> : null}
      </span>
    );
  }
  return <pre className="whitespace-pre-wrap break-all">{JSON.stringify(p, null, 2)}</pre>;
}

function formatTs(ts: string): string {
  try {
    const d = new Date(ts);
    return d.toLocaleTimeString();
  } catch {
    return ts;
  }
}

// Scrollable feed that renders one row per appended event. Three variants
// align with arch §5.2 + design frame `5DIkl`:
//   - message rows (kind: message): from / to / payload.text
//   - signal rows (kind: signal): from / signal type, JSON-ish payload
//   - ask-human cards (signal type: human_question): rich card variant
//     consumed by AskHumanCard
//
// User-authored signals (`human_said`, `human_response`) render as
// message rows so they look like normal chat turns in the feed.
//
// Router-internal plumbing (`inbox_read`, `runner_status`) is hidden by
// default — the audit trail lives in `<mission_dir>/events.ndjson`, and
// the feed is a reading surface for humans collaborating with runners.
// `mission_warning` is intentionally kept (and rendered at full strength):
// it's a diagnostic the user should see. See spec
// `docs/features/08-hide-system-signals-from-feed.md`.

import { useEffect, useRef, useState } from "react";

import { AskHumanCard } from "./AskHumanCard";
import { MessageBody } from "./MessageBody";
import type {
  Event,
  HumanQuestionPayload,
  HumanResponsePayload,
  HumanSaidPayload,
} from "../lib/types";

// Events authored by the human via MissionInput / AskHumanCard. When one
// of these appends we always commit to the bottom — pressing send on a
// chat surface should always land you at your own message, regardless of
// where you'd scrolled.
function isHumanAuthored(ev: Event): boolean {
  return (
    ev.kind === "signal" &&
    (ev.type === "human_said" || ev.type === "human_response")
  );
}

// Router-internal plumbing rows that the reader never needs to see in
// the feed. `runner_status` is already projected onto the RunnersRail
// busy/idle badge; `inbox_read` is just a watermark advance. The full
// stream still lives in NDJSON on disk, and the parent workspace keeps
// receiving every event so projections (status map, watermark) stay
// intact.
function isHiddenSystemSignal(ev: Event): boolean {
  return (
    ev.kind === "signal" &&
    (ev.type === "inbox_read" || ev.type === "runner_status")
  );
}

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
  // Tail id of the last event we processed in the append effect. Without
  // this we can't distinguish a true append from a re-render with the
  // same events array — both fire the effect under StrictMode.
  const lastSeenIdRef = useRef<string | null>(null);
  const [hasNewSinceLeftBottom, setHasNewSinceLeftBottom] = useState(false);

  // Single decision tree on append. The three branches map to the three
  // chat-surface behaviors: human-authored always commits to bottom;
  // crew-emitted commits to bottom only if the user was parked there;
  // otherwise we light the "New messages" pill instead of yanking the
  // viewport.
  useEffect(() => {
    if (events.length === 0) return;
    const tail = events[events.length - 1];
    const isNew = lastSeenIdRef.current !== tail.id;
    lastSeenIdRef.current = tail.id;
    if (!isNew) return;

    const el = scrollRef.current;
    if (!el) return;

    if (isHumanAuthored(tail)) {
      el.scrollTop = el.scrollHeight;
      wasNearBottomRef.current = true;
      setHasNewSinceLeftBottom(false);
      return;
    }

    if (wasNearBottomRef.current) {
      el.scrollTop = el.scrollHeight;
      return;
    }

    setHasNewSinceLeftBottom(true);
  }, [events]);

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
      setHasNewSinceLeftBottom(false);
    }
  }, [active]);

  const onScroll = () => {
    const el = scrollRef.current;
    if (!el) return;
    const distance = el.scrollHeight - el.scrollTop - el.clientHeight;
    const near = distance < 80;
    wasNearBottomRef.current = near;
    if (near) setHasNewSinceLeftBottom(false);
  };

  const onPillClick = () => {
    const el = scrollRef.current;
    if (!el) return;
    // Plain synchronous write — `scrollTo({ behavior: "smooth" })` fires
    // `onScroll` per-frame during the animation, each frame sees
    // `distance > 80` and overwrites `wasNearBottomRef = false`,
    // which races append events arriving mid-animation.
    el.scrollTop = el.scrollHeight;
    wasNearBottomRef.current = true;
    setHasNewSinceLeftBottom(false);
  };

  return (
    <div className="relative flex min-h-0 flex-1 flex-col">
      <div
        ref={scrollRef}
        onScroll={onScroll}
        className="flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-10 py-6"
      >
        {events.length === 0 ? (
          <p className="text-[12px] text-fg-3">No events yet.</p>
        ) : (
          events
            .filter((ev) => !isHiddenSystemSignal(ev))
            .map((ev) => (
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
      {hasNewSinceLeftBottom ? (
        <button
          type="button"
          onClick={onPillClick}
          className="absolute bottom-4 left-1/2 -translate-x-1/2 cursor-pointer rounded-full bg-accent px-3 py-1.5 text-[12px] font-medium text-bg shadow-md transition-opacity hover:opacity-90"
        >
          New messages ↓
        </button>
      ) : null}
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

  // User-authored signals render as message rows (header + plain text
  // body via MessageBody), so a `human_said` from MissionInput and a
  // `human_response` from AskHumanCard look like normal chat turns
  // instead of a JSON-y signal box. Target derivation differs by type:
  // human_said carries `payload.target`; human_response is paired back
  // to the original asker via askersByQuestion[question_id].
  if (
    event.kind === "signal" &&
    (event.type === "human_said" || event.type === "human_response")
  ) {
    let target: string | null;
    let text: string;
    if (event.type === "human_said") {
      const p = (event.payload ?? {}) as Partial<HumanSaidPayload>;
      text = p.text ?? "";
      target = p.target ?? null;
    } else {
      const p = (event.payload ?? {}) as Partial<HumanResponsePayload>;
      text = p.choice ?? "";
      target = p.question_id ? askersByQuestion[p.question_id] ?? "?" : "?";
    }
    return (
      <div className="flex flex-col gap-1">
        <div className="flex items-baseline gap-2 text-[11px] text-fg-3">
          <span className="font-mono text-[12px] font-semibold text-accent">
            @{event.from}
          </span>
          <span>message</span>
          {target ? (
            <>
              <span>→</span>
              <span className="font-mono text-fg-2">@{target}</span>
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

  // Default signal row. `inbox_read` / `runner_status` are filtered out
  // upstream (see isHiddenSystemSignal); `mission_warning` reaches here
  // and renders at full strength so the diagnostic stands out.
  return (
    <div className="flex flex-col gap-1">
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
  if (event.type === "mission_goal") {
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

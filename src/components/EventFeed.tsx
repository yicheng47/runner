import { ChevronDown, Zap } from "lucide-react";
import { useEffect, useMemo, useRef, useState, type ReactNode } from "react";

import { AskHumanCard } from "./AskHumanCard";
import { MessageBody } from "./MessageBody";
import { RunnerAvatar, hueForSeed } from "./ui/RunnerAvatar";
import {
  groupFeedBlocks,
  isHumanAuthored,
  type FeedBlock,
} from "../lib/eventFeed";
import type {
  Event,
  HumanQuestionPayload,
  HumanResponsePayload,
  HumanSaidPayload,
} from "../lib/types";

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
  const blocks = useMemo(() => groupFeedBlocks(events), [events]);
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
        className="flex min-h-0 flex-1 flex-col gap-[18px] overflow-y-auto px-6 py-6"
      >
        {blocks.length === 0 ? (
          <p className="px-4 text-[12px] text-fg-3">No events yet.</p>
        ) : (
          blocks.map((block) => (
            <FeedBlockRow
              key={
                block.kind === "message-group"
                  ? block.events[0].id
                  : block.event.id
              }
              block={block}
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

function FeedBlockRow({
  block,
  missionId,
  resolvedAsks,
  askersByQuestion,
  onError,
}: {
  block: FeedBlock;
  missionId: string;
  resolvedAsks: Record<string, string>;
  askersByQuestion: Record<string, string>;
  onError?: (msg: string) => void;
}) {
  if (block.kind === "divider") {
    return <MissionDivider event={block.event} />;
  }

  if (block.kind === "message-group") {
    return (
      <MessageGroup block={block} askersByQuestion={askersByQuestion} />
    );
  }

  if (block.kind === "ask-card") {
    const event = block.event;
    return (
      <AskHumanCard
        missionId={missionId}
        questionId={event.id}
        asker={askersByQuestion[event.id] ?? "?"}
        payload={event.payload as HumanQuestionPayload}
        ts={event.ts}
        resolvedChoice={resolvedAsks[event.id] ?? null}
        onError={onError}
      />
    );
  }

  return <SignalRow event={block.event} />;
}

function MissionDivider({ event }: { event: Event }) {
  return (
    <div className="flex items-center gap-2.5 px-4">
      <span className="h-px min-w-0 flex-1 bg-line" />
      <span className="shrink-0 text-[10px] font-semibold uppercase tracking-[0.08em] text-fg-3">
        Mission started · {formatTs(event.ts)}
      </span>
      <span className="h-px min-w-0 flex-1 bg-line" />
    </div>
  );
}

function MessageGroup({
  block,
  askersByQuestion,
}: {
  block: Extract<FeedBlock, { kind: "message-group" }>;
  askersByQuestion: Record<string, string>;
}) {
  const first = block.events[0];
  const human = block.author === "human";
  const target = messageTarget(first, askersByQuestion);
  const goal = first.kind === "signal" && first.type === "mission_goal";

  return (
    <div className="flex gap-3 px-4">
      <RunnerAvatar seed={block.author} size={35} />
      <div className="min-w-0 flex-1">
        <div className="flex min-w-0 items-center gap-2 text-[11px] text-fg-3">
          <span
            className="truncate font-mono text-[13px] font-semibold"
            style={{ color: hueForSeed(block.author) }}
          >
            {human ? "you" : `@${block.author}`}
          </span>
          {goal ? (
            <span className="rounded bg-raised px-1.5 py-0.5 text-[9px] font-semibold uppercase tracking-[0.06em] text-fg-2">
              Goal
            </span>
          ) : null}
          {target ? (
            <span className="truncate font-mono text-[11px] text-fg-2">
              → @{target}
            </span>
          ) : null}
          <span className="shrink-0">{formatTs(first.ts)}</span>
        </div>
        <div className="space-y-1.5 text-[13px] leading-relaxed text-fg">
          {block.events.map((event) => {
            const text = messageText(event);
            return (
              <div key={event.id}>
                {text ? (
                  <MessageBody text={text} />
                ) : event.kind === "signal" && event.type === "mission_goal" ? (
                  <span className="text-fg-3">(no text)</span>
                ) : null}
              </div>
            );
          })}
        </div>
      </div>
    </div>
  );
}

function messageText(event: Event): string {
  if (event.kind === "message") {
    return (event.payload as { text?: string } | null)?.text ?? "";
  }
  if (event.type === "human_said") {
    return ((event.payload ?? {}) as Partial<HumanSaidPayload>).text ?? "";
  }
  if (event.type === "human_response") {
    return ((event.payload ?? {}) as Partial<HumanResponsePayload>).choice ?? "";
  }
  const payload = (event.payload ?? {}) as { text?: string };
  return typeof payload.text === "string" ? payload.text : "";
}

function messageTarget(
  event: Event,
  askersByQuestion: Record<string, string>,
): string | null {
  if (event.kind === "message") return event.to;
  if (event.type === "human_said") {
    return ((event.payload ?? {}) as Partial<HumanSaidPayload>).target ?? null;
  }
  if (event.type === "human_response") {
    const payload = (event.payload ?? {}) as Partial<HumanResponsePayload>;
    return payload.question_id
      ? askersByQuestion[payload.question_id] ?? "?"
      : "?";
  }
  const payload = (event.payload ?? {}) as { target?: string };
  return typeof payload.target === "string" ? payload.target : null;
}

function SignalRow({ event }: { event: Event }) {
  const warning = event.type === "mission_warning";
  const tone = warning ? "text-danger" : "text-fg-3";

  return (
    <details className="group pr-4 pl-[63px]">
      <summary className="flex min-w-0 cursor-pointer list-none items-center gap-1.5 text-[11px] [&::-webkit-details-marker]:hidden">
        <Zap aria-hidden className={`h-3 w-3 shrink-0 ${tone}`} />
        <span
          className="shrink-0 font-mono font-semibold"
          style={{ color: hueForSeed(event.from) }}
        >
          @{event.from}
        </span>
        <span className={`min-w-0 truncate ${tone}`}>
          signal · {event.type ?? "?"}
          {event.to ? ` → @${event.to}` : ""} · {formatTs(event.ts)}
        </span>
        <span
          className={`ml-auto inline-flex shrink-0 items-center gap-0.5 text-[10px] ${tone}`}
        >
          payload
          <ChevronDown
            aria-hidden
            className="h-3 w-3 transition-transform group-open:rotate-180"
          />
        </span>
      </summary>
      <div
        className={`mt-2 ml-[18px] rounded-md border p-3 font-mono text-[12px] leading-snug ${
          warning
            ? "border-danger/30 bg-danger/5 text-danger"
            : "border-line bg-bg text-fg-2"
        }`}
      >
        {renderPayload(event)}
      </div>
    </details>
  );
}

function renderPayload(event: Event): ReactNode {
  const p = event.payload as Record<string, unknown> | null | undefined;
  if (!p || typeof p !== "object") {
    return <span>{String(p ?? "")}</span>;
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
  return (
    <pre className="whitespace-pre-wrap break-all">
      {JSON.stringify(p, null, 2)}
    </pre>
  );
}

function formatTs(ts: string): string {
  const date = new Date(ts);
  if (Number.isNaN(date.getTime())) return ts;
  return date.toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

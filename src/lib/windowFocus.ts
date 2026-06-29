// Frontend coordination layer for multi-window (impl 0018, spec 12).
//
// The backend WindowRegistry is the source of truth: it tracks which subject
// (mission / direct chat) each window holds and when each was last focused,
// and broadcasts `window_focus_map` after every mutation. This module is the
// thin React-side mirror: report this window's subject, subscribe to the map,
// and derive "am I the secondary for my subject?" — which gates terminal
// ownership in MissionWorkspace / RunnerChat.

import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";

import { api } from "./api";
import type { Subject, WindowEntry } from "./types";

// The current window's label never changes, so resolve it once at module
// load. Wrapped in try/catch for the dev browser preview (no Tauri runtime),
// where it stays null and every coordination check no-ops.
let cachedLabel: string | null = null;
try {
  cachedLabel = getCurrentWindow().label;
} catch {
  cachedLabel = null;
}

/** This window's Tauri label (`main` or `window-<ulid>`), or null off-Tauri. */
export function useCurrentWindowLabel(): string | null {
  return cachedLabel;
}

/**
 * Subscribe to the backend focus map. Hydrates once on mount via
 * `window_list_subjects` so a freshly-opened window doesn't wait for the next
 * broadcast, then tracks `window_focus_map` broadcasts live.
 */
export function useWindowFocus(): WindowEntry[] {
  const [map, setMap] = useState<WindowEntry[]>([]);
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    let cancelled = false;

    api.window
      .listSubjects()
      .then((entries) => {
        // Don't clobber a broadcast that may have already landed; only seed
        // if we're still empty.
        if (!cancelled) setMap((prev) => (prev.length === 0 ? entries : prev));
      })
      .catch(() => {
        // best-effort; the next broadcast populates the map
      });

    void listen<WindowEntry[]>("window_focus_map", (event) => {
      if (!cancelled) setMap(event.payload);
    }).then((fn) => {
      if (cancelled) {
        fn();
        return;
      }
      unlisten = fn;
    });

    return () => {
      cancelled = true;
      unlisten?.();
    };
  }, []);
  return map;
}

// Debounced subject reporting. Route changes can fire several times in quick
// succession (unmount-null then mount-subject when navigating between two
// missions); coalescing so only the final state hits the backend keeps the
// focus map from thrashing. Last write wins, which is the correct end state.
let reportTimer: ReturnType<typeof setTimeout> | null = null;
let pendingSubject: Subject | null = null;

/** Debounced wrapper over `window_report_subject`. */
export function reportSubject(subject: Subject | null): void {
  if (!cachedLabel) return; // off-Tauri: nothing to coordinate
  pendingSubject = subject;
  if (reportTimer !== null) clearTimeout(reportTimer);
  reportTimer = setTimeout(() => {
    reportTimer = null;
    void api.window.reportSubject(pendingSubject).catch((e) => {
      console.error("reportSubject failed", e);
    });
  }, 80);
}

/**
 * Report `subject` on mount and clear it (report null) on unmount. Subject
 * pages (MissionWorkspace, RunnerChat) call this; the unmount-clear is what
 * lets the focus map go empty when the user navigates to a non-subject page
 * that doesn't report anything itself.
 */
export function useReportSubject(subject: Subject | null): void {
  // Key by content, not object identity, so the effect re-fires only on a
  // real subject change rather than every render.
  const key = subject ? `${subject.type}:${subject.value}` : "null";
  useEffect(() => {
    reportSubject(subject);
    return () => {
      reportSubject(null);
    };
    // eslint-disable-next-line react-hooks/exhaustive-deps
  }, [key]);
}

function sameSubject(a: Subject | null, b: Subject | null): boolean {
  if (!a || !b) return false;
  return a.type === b.type && a.value === b.value;
}

export interface SecondaryState {
  /** True when another window holds the same subject with a later
   *  `focused_at` than this one — this window must not own the PTY. */
  secondary: boolean;
  /** Label of the primary window (the most-recently-focused other holder),
   *  for the overlay's "Focus that window" target. Null when not secondary. */
  primaryLabel: string | null;
}

/**
 * Derive whether `myLabel` is the secondary window for `subject`. A window is
 * secondary iff some *other* window holds the same subject with a strictly
 * later `focused_at`. The primary is that most-recently-focused other holder.
 *
 * `focused_at` is parsed to epoch-ms rather than string-compared: chrono's
 * RFC3339 uses variable fractional-second widths, so lexicographic order can
 * disagree with chronological order.
 */
export function isSecondaryFor(
  map: WindowEntry[],
  myLabel: string | null,
  subject: Subject | null,
): SecondaryState {
  if (!myLabel || !subject) return { secondary: false, primaryLabel: null };
  const mine = map.find((e) => e.label === myLabel);
  // Unknown self (broadcast hasn't included us yet) → treat as the earliest
  // possible focus, so any existing holder wins until we hear otherwise.
  const myFocus = mine ? Date.parse(mine.focused_at) : -Infinity;

  let primaryLabel: string | null = null;
  let primaryFocus = myFocus;
  for (const entry of map) {
    if (entry.label === myLabel) continue;
    if (!sameSubject(entry.subject, subject)) continue;
    const focus = Date.parse(entry.focused_at);
    if (focus > primaryFocus) {
      primaryFocus = focus;
      primaryLabel = entry.label;
    }
  }
  return { secondary: primaryLabel !== null, primaryLabel };
}

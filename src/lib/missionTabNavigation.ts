export type MissionTabCycleDirection = "previous" | "next";

export function missionTabInDirection(
  sessionIds: readonly string[],
  activeTab: string,
  direction: MissionTabCycleDirection,
): string | null {
  if (sessionIds.length === 0) return null;
  const tabs = ["feed", ...sessionIds];
  const currentIndex = tabs.indexOf(activeTab);
  if (currentIndex === -1) return "feed";
  const delta = direction === "next" ? 1 : -1;
  return tabs[(currentIndex + delta + tabs.length) % tabs.length];
}

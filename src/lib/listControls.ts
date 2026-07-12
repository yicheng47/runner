export const PAGE_SIZE = 8;

export type PageWindowItem = number | "ellipsis";

export function buildSearchDoc(
  fields: readonly (string | null | undefined)[],
): string {
  return fields.filter((field) => field != null).join(" ").toLowerCase();
}

export function matchesQuery(document: string, query: string): boolean {
  return document.includes(query.trim().toLowerCase());
}

export function clampPage(page: number, totalPages: number): number {
  return Math.min(Math.max(page, 1), Math.max(totalPages, 1));
}

export function pageWindow(
  currentPage: number,
  totalPages: number,
): PageWindowItem[] {
  if (totalPages <= 0) return [];
  if (totalPages <= 5) {
    return Array.from({ length: totalPages }, (_, index) => index + 1);
  }

  const current = clampPage(currentPage, totalPages);
  if (current <= 3) return [1, 2, 3, 4, "ellipsis", totalPages];
  if (current >= totalPages - 2) {
    return [
      1,
      "ellipsis",
      totalPages - 3,
      totalPages - 2,
      totalPages - 1,
      totalPages,
    ];
  }
  return [
    1,
    "ellipsis",
    current - 1,
    current,
    current + 1,
    "ellipsis",
    totalPages,
  ];
}

import type { ReactNode } from "react";

import { Pager } from "./ui/Pager";
import { SearchInput } from "./ui/SearchInput";

export function PaginatedListPage({
  title,
  description,
  action,
  error,
  loading,
  loaded,
  totalCount,
  filteredCount,
  noun,
  emptyState,
  query,
  onQueryChange,
  searchLabel,
  searchPlaceholder,
  noMatches,
  page,
  pageCount,
  onPageChange,
  children,
}: {
  title: string;
  description: ReactNode;
  action: ReactNode;
  error: string | null;
  loading: boolean;
  loaded: boolean;
  totalCount: number;
  filteredCount: number;
  noun: string;
  emptyState: ReactNode;
  query: string;
  onQueryChange: (query: string) => void;
  searchLabel: string;
  searchPlaceholder: string;
  noMatches: ReactNode;
  page: number;
  pageCount: number;
  onPageChange: (page: number) => void;
  children: ReactNode;
}) {
  return (
    <div className="flex min-h-0 flex-1 flex-col overflow-hidden">
      <div className="flex min-h-0 w-full flex-1 flex-col gap-6 px-8 pb-[18px] pt-10">
        <header className="flex items-center justify-between gap-4">
          <div className="flex flex-col gap-1">
            <h1 className="text-2xl font-bold tracking-tight text-fg">
              {title}
            </h1>
            <p className="text-sm text-fg-2">{description}</p>
          </div>
          {action}
        </header>

        {error ? (
          <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
            {error}
          </div>
        ) : null}

        {loading && !loaded ? (
          <div className="text-sm text-fg-2">Loading…</div>
        ) : !loaded ? (
          <div className="rounded border border-danger/40 bg-danger/10 px-3 py-2 text-sm text-danger">
            Failed to load {noun}.
          </div>
        ) : totalCount === 0 ? (
          emptyState
        ) : (
          <>
            <div className="flex items-center justify-between gap-4">
              <SearchInput
                value={query}
                onChange={onQueryChange}
                label={searchLabel}
                placeholder={searchPlaceholder}
              />
              <span className="shrink-0 font-mono text-[11px] text-fg-2">
                {filteredCount} of {totalCount} {noun}
              </span>
            </div>
            {filteredCount === 0 ? (
              noMatches
            ) : (
              <>
                <div className="flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto">
                  {children}
                </div>
                <div className="mt-auto flex shrink-0 justify-center border-t border-line pt-2.5">
                  <Pager
                    page={page}
                    pageCount={pageCount}
                    onPageChange={onPageChange}
                  />
                </div>
              </>
            )}
          </>
        )}
      </div>
    </div>
  );
}

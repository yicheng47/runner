import { useCallback, useEffect, useRef, useState } from "react";

import { clampPage, PAGE_SIZE } from "../lib/listControls";
import type { ListPage } from "../lib/types";

export const LIST_QUERY_DEBOUNCE_MS = 200;

export function useListControls<T>(
  loadPage: (
    page: number,
    pageSize: number,
    query: string,
  ) => Promise<ListPage<T>>,
) {
  const [query, setQueryValue] = useState("");
  const [debouncedQuery, setDebouncedQuery] = useState("");
  const [page, setPageValue] = useState(1);
  const [pageItems, setPageItems] = useState<T[]>([]);
  const [filteredCount, setFilteredCount] = useState(0);
  const [totalCount, setTotalCount] = useState(0);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const requestIdRef = useRef(0);
  const fetchPageRef = useRef<() => Promise<void>>(async () => {});

  const pageCount = Math.ceil(filteredCount / PAGE_SIZE);

  useEffect(() => {
    const timeout = window.setTimeout(
      () => setDebouncedQuery(query),
      LIST_QUERY_DEBOUNCE_MS,
    );
    return () => window.clearTimeout(timeout);
  }, [query]);

  const fetchPage = useCallback(async () => {
    const requestId = ++requestIdRef.current;
    setLoading(true);
    setError(null);
    try {
      const result = await loadPage(page, PAGE_SIZE, debouncedQuery);
      if (requestId !== requestIdRef.current) return;
      const nextPageCount = Math.ceil(result.filtered_count / PAGE_SIZE);
      const nextPage = clampPage(page, nextPageCount);
      setPageItems(result.items);
      setFilteredCount(result.filtered_count);
      setTotalCount(result.total_count);
      setLoaded(true);
      if (nextPage !== page) setPageValue(nextPage);
    } catch (e) {
      if (requestId === requestIdRef.current) setError(String(e));
    } finally {
      if (requestId === requestIdRef.current) setLoading(false);
    }
  }, [debouncedQuery, loadPage, page]);

  useEffect(() => {
    fetchPageRef.current = fetchPage;
  }, [fetchPage]);

  const refresh = useCallback(() => fetchPageRef.current(), []);

  useEffect(() => {
    if (query !== debouncedQuery) return;
    void fetchPage();
  }, [debouncedQuery, fetchPage, query]);

  useEffect(
    () => () => {
      requestIdRef.current += 1;
    },
    [],
  );

  const setQuery = useCallback((nextQuery: string) => {
    setQueryValue(nextQuery);
    setPageValue(1);
  }, []);
  const setPage = useCallback(
    (nextPage: number) => setPageValue(clampPage(nextPage, pageCount)),
    [pageCount],
  );
  const updatePageItems = useCallback(
    (update: (items: T[]) => T[]) => setPageItems(update),
    [],
  );

  return {
    query,
    setQuery,
    page,
    setPage,
    pageItems,
    updatePageItems,
    filteredCount,
    totalCount,
    pageCount,
    loaded,
    loading,
    error,
    refresh,
  };
}

import { useCallback, useEffect, useMemo, useState } from "react";

import {
  clampPage,
  matchesQuery,
  PAGE_SIZE,
} from "../lib/listControls";

export function useListControls<T>(
  items: readonly T[],
  buildDocument: (item: T) => string,
) {
  const [query, setQueryValue] = useState("");
  const [page, setPageValue] = useState(1);
  const documents = useMemo(
    () => items.map((item) => ({ item, document: buildDocument(item) })),
    [buildDocument, items],
  );
  const filteredItems = useMemo(
    () =>
      documents
        .filter(({ document }) => matchesQuery(document, query))
        .map(({ item }) => item),
    [documents, query],
  );
  const filteredCount = filteredItems.length;
  const pageCount = Math.ceil(filteredCount / PAGE_SIZE);
  const currentPage = clampPage(page, pageCount);
  const pageItems = useMemo(() => {
    const start = (currentPage - 1) * PAGE_SIZE;
    return filteredItems.slice(start, start + PAGE_SIZE);
  }, [currentPage, filteredItems]);

  useEffect(() => {
    if (page !== currentPage) setPageValue(currentPage);
  }, [currentPage, page]);

  const setQuery = useCallback((nextQuery: string) => {
    setQueryValue(nextQuery);
    setPageValue(1);
  }, []);
  const setPage = useCallback(
    (nextPage: number) => setPageValue(clampPage(nextPage, pageCount)),
    [pageCount],
  );

  return {
    query,
    setQuery,
    page: currentPage,
    setPage,
    pageItems,
    filteredCount,
    totalCount: items.length,
    pageCount,
  };
}

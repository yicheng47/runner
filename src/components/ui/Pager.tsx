import { ChevronLeft, ChevronRight } from "lucide-react";

import { pageWindow } from "../../lib/listControls";

export function Pager({
  page,
  pageCount,
  onPageChange,
}: {
  page: number;
  pageCount: number;
  onPageChange: (page: number) => void;
}) {
  const buttonClass =
    "flex h-7 w-7 items-center justify-center rounded border text-[12px] transition-colors focus:outline-none focus-visible:ring-2 focus-visible:ring-line-strong disabled:cursor-not-allowed disabled:opacity-50";

  return (
    <nav aria-label="Pagination" className="flex items-center gap-1">
      <button
        type="button"
        disabled={page === 1}
        onClick={() => onPageChange(page - 1)}
        aria-label="Previous page"
        className={`${buttonClass} cursor-pointer border-transparent text-fg-2 hover:bg-raised hover:text-fg disabled:hover:bg-transparent disabled:hover:text-fg-2`}
      >
        <ChevronLeft aria-hidden className="h-3.5 w-3.5" />
      </button>
      {pageWindow(page, pageCount).map((item, index) =>
        item === "ellipsis" ? (
          <span
            key={`ellipsis-${index}`}
            aria-hidden
            className="flex h-7 w-7 items-center justify-center text-[12px] text-fg-3"
          >
            …
          </span>
        ) : (
          <button
            key={item}
            type="button"
            onClick={() => onPageChange(item)}
            aria-label={`Page ${item}`}
            aria-current={item === page ? "page" : undefined}
            className={`${buttonClass} cursor-pointer ${
              item === page
                ? "border-line-strong bg-raised font-semibold text-fg"
                : "border-transparent text-fg-2 hover:bg-raised hover:text-fg"
            }`}
          >
            {item}
          </button>
        ),
      )}
      <button
        type="button"
        disabled={page === pageCount}
        onClick={() => onPageChange(page + 1)}
        aria-label="Next page"
        className={`${buttonClass} cursor-pointer border-transparent text-fg-2 hover:bg-raised hover:text-fg disabled:hover:bg-transparent disabled:hover:text-fg-2`}
      >
        <ChevronRight aria-hidden className="h-3.5 w-3.5" />
      </button>
    </nav>
  );
}

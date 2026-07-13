import {
  useEffect,
  useRef,
  useState,
  type ComponentType,
  type DragEventHandler,
} from "react";
import { LoaderCircle, MoreHorizontal, Pin } from "lucide-react";

import type { ChatAttentionState } from "../lib/chatAttention";

type SidebarTabIconComponent = ComponentType<{
  className?: string;
  fill?: string;
  "aria-hidden"?: boolean;
}>;

export function SidebarTabIcon({
  icon: Icon,
  active,
}: {
  icon: SidebarTabIconComponent;
  active: boolean;
}) {
  return (
    <Icon
      aria-hidden
      fill="none"
      className={`h-3 w-3 shrink-0 ${
        active ? "text-accent" : "text-fg-2"
      }`}
    />
  );
}

export function SidebarTabRow({
  selected,
  accentBar,
  label,
  icon: Icon,
  iconActive,
  onClick,
  onContextMenu,
  title,
  mono,
  dim,
  dotClassName,
  attention,
  attentionWorkingLabel,
  pinned,
  renaming,
  renameValue,
  renamePlaceholder,
  onRenameSubmit,
  onRenameCancel,
  draggable,
  dragging,
  onDragStart,
  onDragEnd,
}: {
  selected: boolean;
  accentBar?: boolean;
  label: string;
  icon?: SidebarTabIconComponent;
  iconActive?: boolean;
  onClick: () => void;
  onContextMenu?: (anchor: { x: number; y: number }) => void;
  title?: string;
  mono?: boolean;
  dim?: boolean;
  dotClassName?: string;
  attention?: ChatAttentionState;
  attentionWorkingLabel?: string;
  pinned?: boolean;
  renaming?: boolean;
  renameValue?: string;
  renamePlaceholder?: string;
  onRenameSubmit?: (next: string) => void;
  onRenameCancel?: () => void;
  draggable?: boolean;
  dragging?: boolean;
  onDragStart?: DragEventHandler<HTMLDivElement>;
  onDragEnd?: DragEventHandler<HTMLDivElement>;
}) {
  if (renaming && onRenameSubmit && onRenameCancel) {
    return (
      <SidebarTabRenameInput
        initial={renameValue ?? label}
        placeholder={renamePlaceholder ?? label}
        icon={Icon}
        iconActive={iconActive ?? selected}
        title={title}
        mono={mono}
        dim={dim}
        dotClassName={dotClassName}
        attention={attention}
        attentionWorkingLabel={attentionWorkingLabel}
        onSubmit={onRenameSubmit}
        onCancel={onRenameCancel}
      />
    );
  }

  return (
    <div
      draggable={draggable}
      onDragStart={onDragStart}
      onDragEnd={onDragEnd}
      onContextMenu={
        onContextMenu
          ? (event) => {
              event.preventDefault();
              onContextMenu({ x: event.clientX, y: event.clientY });
            }
          : undefined
      }
      className={`group relative flex w-full items-center gap-1.5 rounded border px-2.5 py-1.5 text-left text-xs transition-colors transition-opacity ${
        dragging ? "opacity-40" : ""
      } ${
        selected
          ? "border-sidebar-selected-border bg-sidebar-selected text-fg"
          : "border-transparent text-fg-2 hover:border-sidebar-selected-border hover:bg-sidebar-selected/40 hover:text-fg"
      }`}
    >
      {accentBar ? (
        <span
          aria-hidden
          className="absolute inset-y-0.5 left-0 w-0.5 rounded-full bg-accent"
        />
      ) : null}
      <button
        type="button"
        onClick={onClick}
        title={title}
        className="flex min-w-0 flex-1 cursor-pointer items-center gap-1.5 text-left"
      >
        {attention === undefined ? (
          <span
            className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${
              dotClassName ?? (dim ? "bg-fg-3" : "bg-accent")
            }`}
          />
        ) : null}
        {pinned ? (
          <Pin
            aria-hidden
            className="h-2.5 w-2.5 shrink-0 -rotate-45 text-fg-3"
          />
        ) : null}
        {Icon ? (
          <SidebarTabIcon
            icon={Icon}
            active={iconActive ?? selected}
          />
        ) : null}
        <span
          className={`min-w-0 flex-1 truncate ${
            selected ? "font-semibold" : ""
          } ${mono ? "font-mono" : ""}`}
        >
          {label}
        </span>
        {attention !== undefined ? (
          <ChatAttentionIndicator
            state={attention}
            workingLabel={attentionWorkingLabel}
          />
        ) : null}
      </button>
      {onContextMenu ? (
        <button
          type="button"
          onClick={(event) => {
            event.stopPropagation();
            onContextMenu({ x: event.clientX, y: event.clientY });
          }}
          title="More actions"
          aria-label="More actions"
          className="cursor-pointer rounded p-0.5 text-fg-3 opacity-0 transition-opacity hover:bg-raised hover:text-fg group-hover:opacity-100 focus:opacity-100"
        >
          <MoreHorizontal aria-hidden className="h-3 w-3" />
        </button>
      ) : null}
    </div>
  );
}

function SidebarTabRenameInput({
  initial,
  placeholder,
  icon: Icon,
  iconActive,
  title,
  mono,
  dim,
  dotClassName,
  attention,
  attentionWorkingLabel,
  onSubmit,
  onCancel,
}: {
  initial: string;
  placeholder: string;
  icon?: SidebarTabIconComponent;
  iconActive: boolean;
  title?: string;
  mono?: boolean;
  dim?: boolean;
  dotClassName?: string;
  attention?: ChatAttentionState;
  attentionWorkingLabel?: string;
  onSubmit: (next: string) => void;
  onCancel: () => void;
}) {
  const [draft, setDraft] = useState(initial);
  const inputRef = useRef<HTMLInputElement>(null);

  useEffect(() => {
    inputRef.current?.focus();
    inputRef.current?.select();
  }, []);

  return (
    <div
      className="flex w-full items-center gap-1.5 rounded border border-sidebar-selected-border bg-sidebar-selected px-2.5 py-1.5 text-xs"
      title={title}
    >
      {attention === undefined ? (
        <span
          className={`inline-flex h-1.5 w-1.5 shrink-0 rounded-full ${
            dotClassName ?? (dim ? "bg-fg-3" : "bg-accent")
          }`}
        />
      ) : null}
      {Icon ? (
        <SidebarTabIcon icon={Icon} active={iconActive} />
      ) : null}
      <input
        ref={inputRef}
        value={draft}
        placeholder={placeholder}
        onChange={(event) => setDraft(event.target.value)}
        onKeyDown={(event) => {
          if (event.key === "Enter") {
            event.preventDefault();
            onSubmit(draft.trim());
          } else if (event.key === "Escape") {
            event.preventDefault();
            onCancel();
          }
        }}
        onBlur={() => {
          if (draft.trim() === initial.trim()) onCancel();
          else onSubmit(draft.trim());
        }}
        className={`min-w-0 flex-1 bg-transparent text-xs text-fg outline-none placeholder:text-fg-3 ${
          mono ? "font-mono" : ""
        }`}
      />
      {attention !== undefined ? (
        <ChatAttentionIndicator
          state={attention}
          workingLabel={attentionWorkingLabel}
        />
      ) : null}
    </div>
  );
}

export function ChatAttentionIndicator({
  state,
  workingLabel = "Agent working",
}: {
  state: ChatAttentionState;
  workingLabel?: string;
}) {
  if (state === "working") {
    return (
      <span
        className="flex h-3 w-3 shrink-0 items-center justify-center"
        aria-label={workingLabel}
        title={workingLabel}
      >
        <span
          aria-hidden
          className="flex h-3 w-3 origin-center animate-spin items-center justify-center text-fg-3 motion-reduce:animate-none"
        >
          <LoaderCircle className="block h-3 w-3" />
        </span>
      </span>
    );
  }
  if (state === "unread") {
    return (
      <span
        className="flex h-3 w-3 shrink-0 items-center justify-center"
        aria-label="Completed — not viewed"
        title="Completed — not viewed"
      >
        <span aria-hidden className="h-1.5 w-1.5 rounded-full bg-accent" />
      </span>
    );
  }
  return <span aria-hidden className="h-3 w-3 shrink-0" />;
}

// MCP pane — register Runner with external MCP clients.

import { useEffect, useState } from "react";
import { Check, Copy, Plug, ShieldAlert } from "lucide-react";

import { invoke } from "@tauri-apps/api/core";

import { Toggle } from "../ui/Toggle";
import { PaneHeader } from "./shared";

type McpClientId = "claude_code" | "codex";
type McpCopyKey = McpClientId | "binding_dir";

interface McpClientStatus {
  registered: boolean;
  matches_current: boolean;
  command: string | null;
  args: string[];
  config_path: string;
  error: string | null;
}

interface McpIntegrationStatus {
  environment: string;
  binary_path: string;
  socket_path: string;
  claude_code: McpClientStatus;
  codex: McpClientStatus;
}

interface McpConfigSnippet {
  claude_code: string;
  codex: string;
}

export function McpPane() {
  const [status, setStatus] = useState<McpIntegrationStatus | null>(null);
  const [snippet, setSnippet] = useState<McpConfigSnippet | null>(null);
  const [busy, setBusy] = useState<McpClientId | null>(null);
  const [copied, setCopied] = useState<McpCopyKey | null>(null);
  const [error, setError] = useState<string | null>(null);
  const bindingDir = status?.socket_path ? parentPath(status.socket_path) : "";

  const refresh = async () => {
    try {
      const [nextStatus, nextSnippet] = await Promise.all([
        invoke<McpIntegrationStatus>("mcp_integration_status"),
        invoke<McpConfigSnippet>("mcp_config_snippet"),
      ]);
      setStatus(nextStatus);
      setSnippet(nextSnippet);
      setError(null);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  useEffect(() => {
    void refresh();
  }, []);

  const setIntegration = async (client: McpClientId, enabled: boolean) => {
    setBusy(client);
    setError(null);
    try {
      await invoke("mcp_set_integration", { client, enabled });
      await refresh();
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    } finally {
      setBusy(null);
    }
  };

  const copyMcpText = async (key: McpCopyKey, text?: string) => {
    if (!text) return;
    try {
      await navigator.clipboard.writeText(text);
      setCopied(key);
      window.setTimeout(() => setCopied(null), 1500);
    } catch (e) {
      setError(e instanceof Error ? e.message : String(e));
    }
  };

  return (
    <>
      <PaneHeader
        title="MCP"
        subtitle="Register Runner with external MCP clients."
      />
      <div className="rounded-xl border border-line bg-panel p-4">
        <div className="mb-2 flex items-center justify-between gap-3">
          <div className="flex items-center gap-2">
            <Plug aria-hidden className="h-3.5 w-3.5 text-fg-2" />
            <span className="text-[13px] font-semibold text-fg">
              Current binding
            </span>
          </div>
          <McpEnvironmentBadge environment={status?.environment} />
        </div>
        <BindingLine
          label="Binding dir"
          value={bindingDir}
          copied={copied === "binding_dir"}
          onCopy={() => void copyMcpText("binding_dir", bindingDir)}
        />
      </div>
      <McpClientRow
        title="Claude Code"
        subtitle="Writes the runner entry under ~/.claude.json."
        status={status?.claude_code ?? null}
        busy={busy === "claude_code"}
        onToggle={(enabled) => void setIntegration("claude_code", enabled)}
      />
      <McpClientRow
        title="Codex CLI"
        subtitle="Writes the runner table under ~/.codex/config.toml."
        status={status?.codex ?? null}
        busy={busy === "codex"}
        onToggle={(enabled) => void setIntegration("codex", enabled)}
      />
      <div className="rounded-xl border border-line bg-panel p-4">
        <div className="mb-2 flex items-center justify-between gap-3">
          <span className="text-[13px] font-semibold text-fg">
            Manual config
          </span>
          <div className="flex items-center gap-2">
            <CopyButton
              copied={copied === "claude_code"}
              label="Claude"
              onClick={() => void copyMcpText("claude_code", snippet?.claude_code)}
            />
            <CopyButton
              copied={copied === "codex"}
              label="Codex"
              onClick={() => void copyMcpText("codex", snippet?.codex)}
            />
          </div>
        </div>
        <p className="text-[11px] leading-[1.5] text-fg-2">
          Use these snippets for clients Runner does not update directly.
        </p>
      </div>
      <div className="flex items-start gap-2.5 rounded-xl border border-line bg-panel px-4 py-3">
        <ShieldAlert aria-hidden className="mt-0.5 h-3.5 w-3.5 shrink-0 text-accent" />
        <p className="text-[11px] leading-[1.5] text-fg-2">
          Registering replaces only the `runner` MCP entry. If the row says it
          points to another binary, replacing it will move that client to the
          binding shown above.
        </p>
      </div>
      {error ? (
        <div className="flex items-start justify-between gap-3 rounded-xl border border-red-500/30 bg-red-500/10 px-4 py-3">
          <p className="min-w-0 text-[12px] text-red-300">{error}</p>
          <button
            type="button"
            onClick={() => void refresh()}
            className="shrink-0 cursor-pointer text-[12px] font-medium text-red-200 underline"
          >
            Retry
          </button>
        </div>
      ) : null}
    </>
  );
}

function McpEnvironmentBadge({ environment }: { environment?: string }) {
  const loading = !environment;
  const development = environment?.toLowerCase().includes("dev") ?? false;

  return (
    <span
      className={`inline-flex items-center gap-1.5 rounded px-2 py-0.5 text-[10px] font-medium ${
        loading
          ? "bg-raised text-fg-3"
          : development
            ? "bg-amber-400/10 text-amber-200"
            : "bg-accent/10 text-accent"
      }`}
    >
      <span
        className={`h-1.5 w-1.5 rounded-full ${
          loading ? "bg-fg-3" : development ? "bg-amber-300" : "bg-accent"
        }`}
      />
      {environment ?? "Loading"}
    </span>
  );
}

function parentPath(path: string) {
  const index = path.lastIndexOf("/");
  return index > 0 ? path.slice(0, index) : path;
}

function BindingLine({
  label,
  value,
  copied,
  onCopy,
}: {
  label: string;
  value: string;
  copied: boolean;
  onCopy: () => void;
}) {
  return (
    <div className="grid grid-cols-[72px_minmax(0,1fr)] items-center gap-2 py-1">
      <span className="text-[11px] text-fg-3">{label}</span>
      <div className="group relative flex h-8 min-w-0 items-center gap-2 rounded bg-raised px-2">
        <span className="min-w-0 flex-1 truncate font-mono text-[11px] text-fg-2">
          {value || "Loading..."}
        </span>
        {value ? (
          <div className="pointer-events-none absolute left-0 top-full z-50 mt-1 hidden max-w-[560px] rounded border border-line bg-raised px-2 py-1.5 font-mono text-[10px] leading-[1.45] text-fg shadow-[0_8px_24px_rgba(0,0,0,0.5)] group-hover:block group-focus-within:block">
            {value}
          </div>
        ) : null}
        <button
          type="button"
          aria-label={`Copy ${label}`}
          disabled={!value}
          onClick={onCopy}
          className="flex h-5 w-5 shrink-0 cursor-pointer items-center justify-center rounded text-fg-3 transition-colors hover:bg-line/60 hover:text-fg disabled:cursor-not-allowed disabled:opacity-40"
        >
          {copied ? (
            <Check aria-hidden className="h-3 w-3" />
          ) : (
            <Copy aria-hidden className="h-3 w-3" />
          )}
        </button>
      </div>
    </div>
  );
}

function McpClientRow({
  title,
  subtitle,
  status,
  busy,
  onToggle,
}: {
  title: string;
  subtitle: string;
  status: McpClientStatus | null;
  busy: boolean;
  onToggle: (enabled: boolean) => void;
}) {
  const loading = status == null;
  const hasError = Boolean(status?.error);
  const active = status?.matches_current ?? false;
  const stateLabel = loading
    ? "Checking"
    : busy
      ? "Updating"
    : hasError
      ? "Config error"
      : !status.registered
        ? "Not registered"
        : status.matches_current
          ? ""
          : "Registered to another Runner";
  const stateClass = loading
    ? "text-fg-3"
    : hasError
      ? "text-red-300"
      : status?.matches_current
        ? "text-accent"
      : status?.registered
        ? "text-amber-300"
        : "text-fg-3";
  const configured = status?.registered
    ? `${status.command ?? "(missing command)"} ${JSON.stringify(status.args)}`
    : "";
  const showDetail = hasError || Boolean(status?.registered && !status.matches_current);

  return (
    <div className="rounded-xl border border-line bg-panel p-4">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <div className="flex items-center gap-2">
            <span className="text-[13px] font-semibold text-fg">{title}</span>
            {stateLabel ? (
              <span className={`text-[11px] ${stateClass}`}>{stateLabel}</span>
            ) : null}
          </div>
          <p className="mt-0.5 text-[11px] text-fg-2">{subtitle}</p>
        </div>
        <Toggle
          on={active}
          onChange={onToggle}
          disabled={loading || busy || hasError}
        />
      </div>
      {showDetail ? (
        <div className="mt-2 min-w-0 rounded bg-raised px-2 py-1.5">
          <McpStatusLine
            label={status?.error ? "Error" : "Configured command"}
            value={status?.error ?? configured}
          />
        </div>
      ) : null}
    </div>
  );
}

function McpStatusLine({ label, value }: { label: string; value: string }) {
  return (
    <div className="grid grid-cols-[112px_minmax(0,1fr)] gap-2">
      <span className="text-[10px] text-fg-3">{label}</span>
      <span className="break-all font-mono text-[10px] leading-[1.45] text-fg-3">
        {value}
      </span>
    </div>
  );
}

function CopyButton({
  copied,
  label,
  onClick,
}: {
  copied: boolean;
  label: string;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className="flex cursor-pointer items-center gap-1.5 rounded-md border border-line bg-raised px-2.5 py-1 text-[12px] font-medium text-fg-2 transition-colors hover:border-line-strong hover:text-fg"
    >
      {copied ? (
        <Check aria-hidden className="h-3 w-3" />
      ) : (
        <Copy aria-hidden className="h-3 w-3" />
      )}
      {copied ? "Copied" : label}
    </button>
  );
}

// Runner Detail page (C8.5) — `/runners/:handle`.
//
// Mirrors the design's `ocAFJ` frame: two columns. Left holds the
// system-prompt panel and the "Crews using this runner" list; right holds
// activity counters and immutable metadata. Header carries the breadcrumb,
// the runtime badge, and two actions: Edit (opens RunnerEditDrawer) and
// Chat now (kicks off the direct-chat flow).

import { useCallback, useEffect, useState } from "react";
import { Link, useNavigate, useParams } from "react-router-dom";

import { listen } from "@tauri-apps/api/event";

import { api } from "../lib/api";
import type {
  CrewMembership,
  Runner,
  RunnerActivity,
  RunnerActivityEvent,
} from "../lib/types";
import { AppShell } from "../components/AppShell";
import { Button } from "../components/ui/Button";
import { RunnerEditDrawer } from "../components/RunnerEditDrawer";

export default function RunnerDetail() {
  const { handle: handleParam } = useParams<{ handle: string }>();
  const handle = handleParam ?? "";
  const navigate = useNavigate();

  const [runner, setRunner] = useState<Runner | null>(null);
  const [activity, setActivity] = useState<RunnerActivity | null>(null);
  const [crews, setCrews] = useState<CrewMembership[]>([]);
  const [loading, setLoading] = useState(true);
  const [error, setError] = useState<string | null>(null);
  const [editing, setEditing] = useState(false);
  const [chatCwd, setChatCwd] = useState<string | null>(null);
  const [openingChat, setOpeningChat] = useState(false);

  const refresh = useCallback(async () => {
    if (!handle) return;
    try {
      setError(null);
      const r = await api.runner.getByHandle(handle);
      setRunner(r);
      const [act, crewList] = await Promise.all([
        api.runner.activity(r.id),
        api.runner.crews(r.id),
      ]);
      setActivity(act);
      setCrews(crewList);
    } catch (e) {
      setError(String(e));
    } finally {
      setLoading(false);
    }
  }, [handle]);

  useEffect(() => {
    void refresh();
  }, [refresh]);

  // Live activity — patch counters in place when this runner emits.
  useEffect(() => {
    let unlisten: (() => void) | null = null;
    void listen<RunnerActivityEvent>("runner/activity", (event) => {
      if (event.payload.runner_id !== runner?.id) return;
      setActivity((prev) =>
        prev
          ? {
              ...prev,
              active_sessions: event.payload.active_sessions,
              active_missions: event.payload.active_missions,
              crew_count: event.payload.crew_count,
            }
          : prev,
      );
    }).then((fn) => {
      unlisten = fn;
    });
    return () => {
      unlisten?.();
    };
  }, [runner?.id]);

  const startChat = async () => {
    if (!runner || openingChat) return;
    setOpeningChat(true);
    setError(null);
    try {
      const cwd = chatCwd?.trim() ? chatCwd.trim() : null;
      const spawned = await api.session.startDirect(runner.id, cwd);
      navigate(`/runners/${runner.handle}/chat/${spawned.id}`);
    } catch (e) {
      setError(String(e));
    } finally {
      setOpeningChat(false);
    }
  };

  return (
    <AppShell>
      <div className="flex-1 overflow-y-auto">
        <div className="mx-auto flex max-w-5xl flex-col gap-6 px-8 py-8">
          {/* Breadcrumb + actions row */}
          <header className="flex items-center justify-between gap-4">
            <div className="flex items-baseline gap-2 text-sm text-neutral-500">
              <Link to="/runners" className="hover:text-neutral-800">
                Runners
              </Link>
              <span className="text-neutral-300">›</span>
              <span className="font-mono text-base font-semibold text-neutral-900">
                @{handle}
              </span>
              {runner ? (
                <span className="rounded bg-neutral-100 px-1.5 py-0.5 text-[11px] font-medium text-neutral-500">
                  {runner.runtime}
                </span>
              ) : null}
            </div>
            <div className="flex items-center gap-2">
              <Button
                onClick={() => setEditing(true)}
                disabled={!runner}
                title="Edit runner"
              >
                Edit
              </Button>
              <Button
                variant="primary"
                onClick={() => void startChat()}
                disabled={!runner || openingChat}
                title="Start a one-on-one PTY with this runner"
              >
                {openingChat ? "Starting…" : "Chat now"}
              </Button>
            </div>
          </header>

          {error ? (
            <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
              {error}
            </div>
          ) : null}

          {loading ? (
            <div className="text-sm text-neutral-500">Loading…</div>
          ) : !runner ? (
            <div className="rounded-md border border-red-200 bg-red-50 px-3 py-2 text-sm text-red-700">
              Runner @{handle} not found.
            </div>
          ) : (
            <div className="grid grid-cols-3 gap-6">
              {/* Left column — 2/3 */}
              <div className="col-span-2 flex flex-col gap-4">
                <Card title="Default system prompt" subtitle="Used whenever this runner spawns. Override per crew/mission slots later (v0.x).">
                  {runner.system_prompt ? (
                    <pre className="whitespace-pre-wrap font-mono text-xs leading-relaxed text-neutral-800">
                      {runner.system_prompt}
                    </pre>
                  ) : (
                    <p className="text-sm italic text-neutral-400">
                      No system prompt set.
                    </p>
                  )}
                </Card>

                <Card title="Crews using this runner">
                  {crews.length === 0 ? (
                    <p className="text-sm italic text-neutral-400">
                      Not in any crew yet. Add it to one from Crew Detail.
                    </p>
                  ) : (
                    <ul className="flex flex-col divide-y divide-neutral-100">
                      {crews.map((m) => (
                        <li
                          key={m.crew_id}
                          className="flex items-center justify-between py-2 text-sm"
                        >
                          <div className="flex items-center gap-2">
                            <span className="font-medium text-neutral-900">
                              {m.crew_name}
                            </span>
                            {m.lead ? (
                              <span className="rounded bg-emerald-50 px-1.5 py-0.5 text-[10px] font-semibold uppercase tracking-wide text-emerald-700">
                                LEAD
                              </span>
                            ) : null}
                          </div>
                          <Link
                            to={`/crews/${m.crew_id}`}
                            className="text-xs text-[#0066CC] hover:underline"
                          >
                            Open →
                          </Link>
                        </li>
                      ))}
                    </ul>
                  )}
                </Card>

                <Card title="Chat now" subtitle="Spawn a one-on-one PTY for ad-hoc tasks. Direct chats don't join any mission's coordination bus.">
                  <div className="flex flex-col gap-2">
                    <label className="flex flex-col gap-1 text-xs text-neutral-600">
                      Working directory (optional)
                      <input
                        value={chatCwd ?? ""}
                        onChange={(e) => setChatCwd(e.target.value)}
                        placeholder={runner.working_dir ?? "/Users/you/projects/foo"}
                        className="rounded border border-neutral-300 bg-white p-1.5 font-mono text-xs"
                      />
                    </label>
                    <p className="text-[11px] text-neutral-400">
                      Defaults to the runner's own working directory if blank.
                    </p>
                  </div>
                </Card>
              </div>

              {/* Right column — 1/3 */}
              <div className="col-span-1 flex flex-col gap-4">
                <Card title="Activity">
                  <div className="grid grid-cols-2 gap-3">
                    <Stat
                      label="Sessions"
                      value={activity?.active_sessions ?? 0}
                      accent={(activity?.active_sessions ?? 0) > 0}
                    />
                    <Stat
                      label="Missions"
                      value={activity?.active_missions ?? 0}
                      accent={(activity?.active_missions ?? 0) > 0}
                    />
                    <Stat
                      label="Crews"
                      value={activity?.crew_count ?? 0}
                    />
                    <Stat
                      label="Last seen"
                      value={
                        activity?.last_started_at
                          ? new Date(activity.last_started_at).toLocaleDateString()
                          : "—"
                      }
                    />
                  </div>
                </Card>

                <Card title="Details">
                  <dl className="grid grid-cols-[auto_1fr] gap-x-3 gap-y-1.5 text-xs">
                    <dt className="text-neutral-500">Handle</dt>
                    <dd className="font-mono text-neutral-900">@{runner.handle}</dd>
                    <dt className="text-neutral-500">Display</dt>
                    <dd className="text-neutral-900">{runner.display_name}</dd>
                    <dt className="text-neutral-500">Role</dt>
                    <dd className="text-neutral-900">{runner.role}</dd>
                    <dt className="text-neutral-500">Runtime</dt>
                    <dd className="text-neutral-900">{runner.runtime}</dd>
                    <dt className="text-neutral-500">Command</dt>
                    <dd className="break-all font-mono text-neutral-900">
                      {runner.command}
                    </dd>
                    {runner.args.length > 0 ? (
                      <>
                        <dt className="text-neutral-500">Args</dt>
                        <dd className="break-all font-mono text-neutral-900">
                          {runner.args.join(" ")}
                        </dd>
                      </>
                    ) : null}
                    <dt className="text-neutral-500">Created</dt>
                    <dd className="text-neutral-900">
                      {new Date(runner.created_at).toLocaleString()}
                    </dd>
                    <dt className="text-neutral-500">ID</dt>
                    <dd className="break-all font-mono text-[10px] text-neutral-500">
                      {runner.id}
                    </dd>
                  </dl>
                </Card>
              </div>
            </div>
          )}
        </div>
      </div>

      <RunnerEditDrawer
        open={editing}
        runner={runner}
        onClose={() => setEditing(false)}
        onSaved={async () => {
          setEditing(false);
          await refresh();
        }}
      />
    </AppShell>
  );
}

function Card({
  title,
  subtitle,
  children,
}: {
  title: string;
  subtitle?: string;
  children: React.ReactNode;
}) {
  return (
    <section className="flex flex-col gap-2 rounded-lg border border-[#E5E5E5] bg-white p-4">
      <div className="flex flex-col gap-0.5">
        <h2 className="text-sm font-semibold text-neutral-900">{title}</h2>
        {subtitle ? (
          <p className="text-[11px] text-neutral-500">{subtitle}</p>
        ) : null}
      </div>
      <div>{children}</div>
    </section>
  );
}

function Stat({
  label,
  value,
  accent,
}: {
  label: string;
  value: number | string;
  accent?: boolean;
}) {
  return (
    <div className="flex flex-col rounded-md border border-neutral-200 bg-neutral-50 p-2">
      <span className="text-[10px] uppercase tracking-wide text-neutral-500">
        {label}
      </span>
      <span
        className={`text-lg font-semibold ${accent ? "text-emerald-600" : "text-neutral-900"}`}
      >
        {value}
      </span>
    </div>
  );
}

import { Plugin } from "@opencode/plugin/tui";
import { spawn } from "node:child_process";

export function setup(context) {
  const executable = process.env.RUNNER_HOOK_EXECUTABLE;
  if (!executable || !process.env.RUNNER_HOOK_GENERATION) return;
  const epoch = Number(process.hrtime.bigint() / 1000n);
  let sequence = 0;
  let key = null;
  let state;
  let idle;
  let live = false;
  let knownBaseline = false;
  let stopped = false;
  let reporter;
  let latest;
  let acknowledged;
  let retryAt = 0;
  let lastSent = 0;
  let failures = 0;
  let bridgeLost = false;
  const send = (fields) => {
    latest = { epoch, sequence: ++sequence, session_id: key ?? "", ...fields };
  };
  const flush = () => {
    const now = performance.now();
    if (stopped || reporter || !latest || now < retryAt) return;
    if (acknowledged === latest.sequence && (process.env.RUNNER_STATUS_HOOKS === "false" || now - lastSent < 1000)) return;
    const payload = latest;
    const args = ["hook", "report", "--runtime", "opencode", "--event", "snapshot", "--acknowledge"];
    if (bridgeLost) args.push("--bridge-unavailable");
    const child = spawn(executable, args, { stdio: ["pipe", "ignore", "ignore"], windowsHide: true });
    reporter = child;
    lastSent = now;
    let failed = false;
    let expired = false;
    const deadline = setTimeout(() => { expired = true; child.kill(); }, 1500);
    child.on("error", () => { failed = true; });
    child.stdin.on("error", () => { failed = true; });
    child.stdin.end(JSON.stringify(payload));
    child.on("close", (code) => {
      clearTimeout(deadline);
      reporter = undefined;
      if (stopped) return;
      if (code === 0 && !failed && !expired) {
        failures = 0;
        acknowledged = payload.sequence;
        if (bridgeLost) { stopped = true; clearInterval(timer); return; }
      } else {
        failures += 1;
        bridgeLost ||= failures >= 3;
        retryAt = performance.now() + Math.min(2000, 250 * failures);
      }
    });
  };
  const poll = () => {
    if (stopped) return;
    try {
      const route = context.ui.router.current();
      const next = route?.type === "session" ? route.sessionID : undefined;
      const info = next ? context.data.session.get(next) : undefined;
      if (next && !info) {
        if (key !== undefined) { key = undefined; state = undefined; idle = undefined; live = false; send({}); }
        flush(); return;
      }
      const root = info?.parentID ? undefined : next;
      const changed = root !== key;
      if (changed) { key = root; state = undefined; idle = info?.time?.idle; live = false; knownBaseline = false; }
      if (!root) { if (changed) send({}); flush(); return; }
      if (process.env.RUNNER_STATUS_HOOKS === "false") {
        if (changed) send({});
        flush(); return;
      }
      const current = context.data.session.status(root);
      if (!live) idle = info.time?.idle;
      if (live && !knownBaseline && current === "running" && Number.isFinite(info.time?.created)) { knownBaseline = true; idle = info.time.idle; }
      if (current === "running" && state !== "working") {
        live = true;
        // Require a complete native time record before interpreting an outcome.
        knownBaseline = Number.isFinite(info.time?.created);
        state = "working";
        send({ state });
      } else if (live && knownBaseline && current !== "running" && info.time?.idle && info.outcome && idle !== info.time.idle) {
        idle = info.time.idle;
        live = false;
        state = `idle:${idle}`;
        send({ state: "idle", outcome: info.outcome });
      } else if (live && current !== "running") {
        live = false;
        idle = info.time?.idle;
        state = "ready";
        send({ state });
      } else if (changed) send({});
    } catch {}
    flush();
  };
  const timer = setInterval(poll, 150);
  poll();
  return () => { stopped = true; clearInterval(timer); latest = undefined; reporter?.kill(); };
}

export default Plugin.define({ id: "runner.opencode.status", setup });

import { Plugin } from "@opencode/plugin/tui";
import { spawn } from "node:child_process";

export function setup(context) {
  const executable = process.env.RUNNER_HOOK_EXECUTABLE;
  if (!executable || !process.env.RUNNER_HOOK_GENERATION) return;
  const epoch = Number(process.hrtime.bigint() / 1000n);
  let sequence = 0;
  let key = null;
  let stopped = false;
  let reporter;
  let latest;
  let acknowledged;
  let retryAt = 0;
  let failures = 0;
  let bridgeLost = false;
  const send = () => {
    latest = { epoch, sequence: ++sequence, session_id: key ?? "" };
  };
  const flush = () => {
    const now = performance.now();
    if (stopped || reporter || !latest || now < retryAt) return;
    if (acknowledged === latest.sequence) return;
    const payload = latest;
    const args = ["hook", "report", "--runtime", "opencode", "--event", "snapshot", "--acknowledge"];
    if (bridgeLost) args.push("--bridge-unavailable");
    const child = spawn(executable, args, { stdio: ["pipe", "ignore", "ignore"], windowsHide: true });
    reporter = child;
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
        if (key !== undefined) { key = undefined; send(); }
        flush(); return;
      }
      const root = info?.parentID ? undefined : next;
      if (root !== key) {
        key = root;
        send();
      }
    } catch {}
    flush();
  };
  const timer = setInterval(poll, 150);
  poll();
  return () => { stopped = true; clearInterval(timer); latest = undefined; reporter?.kill(); };
}

export default Plugin.define({ id: "runner.opencode.status", setup });

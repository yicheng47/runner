"""Pre-implementation method fixture; deliberately not product acceptance evidence."""
import argparse
import copy
import json
import math
import os
from pathlib import Path
import re
import statistics
import subprocess
import sys
import threading
import time
import uuid
from multiprocessing.connection import Client, Listener

EVENTS = (
    "SessionStart", "UserPromptSubmit", "PreToolUse", "PostToolUse",
    "PreCompact", "PostCompact", "Stop", "Interrupt", "SessionEnd",
)
MCP_EVENTS = EVENTS[:-1]


def projection(event):
    fields = ["hook_event_name", "session_id", "transcript_path"]
    if event not in ("SessionStart", "SessionEnd"):
        fields.append("turn_id")
    if event == "SessionStart":
        fields.append("source")
    if event in ("PreCompact", "PostCompact"):
        fields.append("trigger")
    return {field: "${" + field + "}" for field in fields}


def expand(template, payload):
    return {key: payload[value[2:-1]] for key, value in template.items()}


def root(payload, meta):
    return bool(payload.get("session_id")) and meta.get("threadId") == payload["session_id"]


def inspect_method(sources, eligibility_sources):
    schema = (sources / "codex-0.159.3-schema.rs").read_text(encoding="utf-8")
    discovery = (eligibility_sources / "f6-codex-pinned-discovery.rs").read_text(encoding="utf-8")
    end = (eligibility_sources / "f6-codex-pinned-session-end.rs").read_text(encoding="utf-8")
    runtime = (sources / "codex-0.159.3-core-hook_runtime.rs").read_text(encoding="utf-8")
    mcp_handler = discovery.split("HookHandlerConfig::McpTool {")[1].split("HookHandlerConfig::Prompt {}")[0]
    assert "event_name == HookEventName::SessionEnd" in mcp_handler
    assert "MCP hooks are not supported" in mcp_handler
    assert "continue;" in mcp_handler
    end_dispatch = runtime.split("async fn run_session_end_hooks")[1].split("async fn run_turn_interrupt_hooks")[0]
    assert end_dispatch.index("SessionSource::SubAgent") < end_dispatch.index("codex_hooks::SessionEndRequest")
    assert "return;" in end_dispatch.split("SessionSource::SubAgent")[1].split("codex_hooks::SessionEndRequest")[0]
    assert 'SESSION_END_REASON: &str = "other"' in end
    assert "SESSION_END_DEFAULT_TIMEOUT_SEC: u64 = 1" in end
    assert "SESSION_END_MAX_TIMEOUT_SEC: u64 = 3" in end
    end_body = re.search(r"struct SessionEndCommandInput \{(.*?)\n\}", schema, re.S)[1]
    assert set(re.findall(r"pub (\w+):", end_body)) == {
        "session_id", "transcript_path", "cwd", "hook_event_name", "reason"}
    projections = {}
    checks = 0
    for event in MCP_EVENTS:
        body = re.search(r"struct " + event + r"CommandInput \{(.*?)\n\}", schema, re.S)[1]
        guaranteed = set(re.findall(r"pub (\w+): (?!Option)[^\n]+", body))
        template = projection(event)
        assert set(template) <= guaranteed, (event, set(template) - guaranteed)
        assert "agent_id" not in template
        payload = {field: "value" for field in guaranteed}
        payload.update(hook_event_name=event, session_id="root", transcript_path=None)
        projected = expand(template, payload)
        assert projected["transcript_path"] is None
        for mode in ("startup", "resume", "clear", "fork"):
            payload["source"] = mode
            projected = expand(template, payload)
            untouched = {"key": None, "status": "baseline"}
            for meta in ({}, {"threadId": ""}, {"threadId": "child"}, {"threadId": "grandchild"}):
                state = copy.deepcopy(untouched)
                if root(projected, meta):
                    state["key"] = projected["session_id"]
                assert state == untouched
                checks += 1
            assert root(projected, {"threadId": "root"})
            checks += 1
        missing = dict(payload)
        del missing["session_id"]
        try:
            expand(template, missing)
            raise AssertionError("missing field expanded")
        except KeyError:
            pass
        projections[event] = template
    recovery_checks = 0
    for mode in ("startup", "resume", "clear", "fork"):
        for established in (False, True):
            for event in MCP_EVENTS[1:]:
                state = {"key": "old" if established else None,
                         "status": "working" if established else "baseline",
                         "transcript": None, "starts": 0}
                payload = {"session_id": mode, "hook_event_name": event,
                           "transcript_path": "C:/日志/rollout.jsonl", "turn_id": "new"}
                for meta in ({}, {"threadId": "child"}):
                    assert not root(payload, meta)
                assert root(payload, {"threadId": mode})
                state.update(key=mode, transcript=payload["transcript_path"],
                             status="unavailable" if established else "baseline")
                if event == "UserPromptSubmit":
                    state["status"] = "working"
                elif event == "Interrupt":
                    state["status"] = "interrupted-unavailable"
                assert state["starts"] == 0 and state["status"] != "completed"
                assert state["key"] == mode and state["transcript"] == payload["transcript_path"]
                recovery_checks += 1
    return {"projection_and_identity_cases": checks, "projections": projections,
            "session_end_method": "root-only command at graceful teardown; MCP handler explicitly rejected",
            "handler_eligibility_checks": 9,
            "recovery_contract_cases": recovery_checks,
            "recovery_limitation": "executable contract model only; product parser/reducer tests required"}


def emit(value):
    print(json.dumps(value, ensure_ascii=False), flush=True)


def adapter(endpoint):
    # Process creation would be a fixture failure, including indirect subprocess calls.
    def audit(event, args):
        if event in ("subprocess.Popen", "os.system", "os.spawn", "os.posix_spawn"):
            raise RuntimeError("per-event process creation: " + event)
    sys.addaudithook(audit)
    for line in sys.stdin:
        request = json.loads(line)
        if request["method"] == "initialize":
            result = {"protocolVersion": "2024-11-05", "capabilities": {"tools": {}},
                      "serverInfo": {"name": "797-method-fixture", "version": "0"}}
        else:
            args = request["params"]["arguments"]
            if root(args, request["params"].get("_meta", {})):
                with Client(endpoint, family="AF_PIPE") as connection:
                    connection.send_bytes(json.dumps(args, ensure_ascii=False).encode())
                    assert connection.recv_bytes() == b"accepted"
            result = {"content": [{"type": "text", "text": "{}"}], "isError": False}
        emit({"jsonrpc": "2.0", "id": request["id"], "result": result})


def distribution(samples):
    return {"n": len(samples), "median_ms": statistics.median(samples),
            "p95_ms": sorted(samples)[math.ceil(len(samples) * .95) - 1],
            "max_ms": max(samples), "samples_ms": samples}


def spawn(mode, endpoint):
    return subprocess.Popen([sys.executable, __file__, mode, "--endpoint", endpoint],
                            stdin=subprocess.PIPE, stdout=subprocess.PIPE, stderr=subprocess.PIPE,
                            text=True, encoding="utf-8", creationflags=subprocess.CREATE_NO_WINDOW)


def rpc(child, method, arguments=None, meta=None):
    request = {"jsonrpc": "2.0", "id": 1, "method": method}
    if arguments is not None:
        request["params"] = {"arguments": arguments, "_meta": meta or {}}
    child.stdin.write(json.dumps(request, ensure_ascii=False) + "\n")
    child.stdin.flush()
    return json.loads(child.stdout.readline())


def close(child):
    child.stdin.close()
    assert child.wait(timeout=5) == 0
    assert not child.stderr.read()


def measure():
    endpoint = "\\\\.\\pipe\\runner-797-method-" + uuid.uuid4().hex
    accepted = []
    ready = threading.Event()

    def receiver():
        with Listener(endpoint, family="AF_PIPE") as listener:
            ready.set()
            for _ in range(100):
                with listener.accept() as connection:
                    accepted.append(json.loads(connection.recv_bytes()))
                    connection.send_bytes(b"accepted")

    server = threading.Thread(target=receiver, daemon=True)
    server.start()
    assert ready.wait(5)
    child = spawn("serve", endpoint)
    assert "result" in rpc(child, "initialize")
    warm = []
    pid = child.pid
    for index in range(100):
        payload = {"session_id": "root", "hook_event_name": "Interrupt",
                   "turn_id": str(index), "transcript_path": None}
        start = time.perf_counter()
        result = rpc(child, "tools/call", payload, {"threadId": "root"})
        warm.append((time.perf_counter() - start) * 1000)
        assert result["result"]["content"][0]["text"] == "{}"
        assert child.pid == pid and child.poll() is None
    close(child)
    server.join(5)
    assert not server.is_alive()
    assert [item["turn_id"] for item in accepted] == list(map(str, range(100)))
    baseline, enabled = [], []
    for _ in range(30):
        for mode, samples in (("baseline", baseline), ("serve", enabled)):
            start = time.perf_counter()
            child = spawn(mode, endpoint)
            assert "result" in rpc(child, "initialize")
            samples.append((time.perf_counter() - start) * 1000)
            close(child)
    return {"accepted": len(accepted), "warm": distribution(warm),
            "startup_baseline": distribution(baseline), "startup_adapter": distribution(enabled),
            "startup_delta": distribution([b - a for a, b in zip(baseline, enabled)]),
            "warm_adapter_processes": 1, "warm_per_event_processes": 0,
            "process_evidence": "direct Popen argv plus child Python audit denies process creation",
            "limitation": "Python AF_PIPE method proxy, not Runner frames, secured listener, reducer, or actual Codex; no product latency claim"}


if __name__ == "__main__":
    parser = argparse.ArgumentParser()
    parser.add_argument("mode", choices=("check", "serve", "baseline"))
    parser.add_argument("--endpoint")
    parser.add_argument("--source-dir", type=Path)
    parser.add_argument("--eligibility-source-dir", type=Path)
    args = parser.parse_args()
    if args.mode == "serve":
        adapter(args.endpoint)
    elif args.mode == "baseline":
        for line in sys.stdin:
            request = json.loads(line)
            emit({"jsonrpc": "2.0", "id": request["id"], "result": {}})
    else:
        if args.source_dir is None or args.eligibility_source_dir is None:
            parser.error("check requires --source-dir and --eligibility-source-dir")
        result = inspect_method(args.source_dir, args.eligibility_source_dir)
        result["timing"] = measure()
        emit(result)

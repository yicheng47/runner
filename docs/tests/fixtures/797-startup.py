"""Matched fixture startup, including the complete added native helper process."""

import argparse
import json
import os
import statistics
import subprocess
import time


def distribution(values):
    values = sorted(values)
    return {"samples": len(values), "median_ms": statistics.median(values),
            "p95_ms": values[int(len(values) * .95 + .999) - 1], "max_ms": values[-1]}


def fixture(executable, helper):
    started = time.perf_counter()
    # Both arms start the same harmless native fixture process. The enabled arm
    # additionally starts, initializes and reaps the real adapter, without a daemon.
    subprocess.run([executable, "--version"], capture_output=True, check=True)
    if helper:
        request = {"jsonrpc": "2.0", "id": 1, "method": "initialize",
                   "params": {"protocolVersion": "2024-11-05"}}
        env = {key: value for key, value in os.environ.items() if not key.startswith("RUNNER_HOOK_")}
        result = subprocess.run([executable, "hook", "serve"], input=json.dumps(request) + "\n",
                                capture_output=True, text=True, check=True, timeout=3, env=env)
        reply = json.loads(result.stdout)
        assert reply["result"]["serverInfo"]["name"] == "runner-hooks"
        assert result.stderr == ""
    return (time.perf_counter() - started) * 1000


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("executable")
    parser.add_argument("--shell", action="append", default=[])
    args = parser.parse_args()
    baseline, enabled, deltas = [], [], []
    for index in range(30):
        pair = {}
        for helper in ([False, True] if index % 2 == 0 else [True, False]):
            pair[helper] = fixture(args.executable, helper)
        baseline.append(pair[False])
        enabled.append(pair[True])
        deltas.append(pair[True] - pair[False])
    result = {"executable": os.path.abspath(args.executable),
              "baseline_processes_per_run": 1, "enabled_processes_per_run": 2,
              "baseline": distribution(baseline), "enabled": distribution(enabled),
              "delta": distribution(deltas), "raw_delta_ms": deltas, "shells": {}}
    for shell in args.shell:
        version = subprocess.run([shell, "-NoProfile", "-Command", "$PSVersionTable.PSVersion.ToString()"],
                                 capture_output=True, text=True, check=True).stdout.strip()
        samples = []
        for _ in range(30):
            started = time.perf_counter()
            subprocess.run([shell, "-NoProfile", "-NonInteractive", "-Command", "exit 0"],
                           capture_output=True, check=True, timeout=5)
            samples.append((time.perf_counter() - started) * 1000)
        result["shells"][shell] = {"version": version, **distribution(samples), "raw_ms": samples}
    print(json.dumps(result, indent=2))


if __name__ == "__main__":
    main()

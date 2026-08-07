import { describe, expect, it } from "vitest";

import {
  EFFORT_OPTIONS_BY_RUNTIME,
  inferPermissionMode,
  modelSuggestions,
  PERMISSION_MODES_BY_RUNTIME,
  RUNTIME_OPTIONS,
  runtimeClearsOnResize,
  runtimeSupportsEffort,
  runtimeSupportsPermissionMode,
  stripPermissionFlags,
} from "./runtimes";

describe("runtime metadata", () => {
  it("appends new runtimes without changing the default runtime", () => {
    expect(RUNTIME_OPTIONS.map((runtime) => runtime.value)).toEqual([
      "codex",
      "claude-code",
      "qoder",
      "trae",
    ]);
    expect(RUNTIME_OPTIONS[RUNTIME_OPTIONS.length - 1]).toMatchObject({
      value: "trae",
      defaultCommand: "traecli",
    });
  });

  it("supports only default and auto permission modes", () => {
    expect(runtimeSupportsPermissionMode("qoder")).toBe(true);
    expect(
      PERMISSION_MODES_BY_RUNTIME.qoder.map((mode) => mode.value),
    ).toEqual(["default", "auto"]);
    expect(
      inferPermissionMode("qoder", ["--permission-mode", "auto"]),
    ).toBe("auto");
    expect(
      stripPermissionFlags("qoder", [
        "--debug",
        "--permission-mode=auto",
      ]),
    ).toEqual(["--debug"]);
  });

  it("offers only the CLI model default and no effort override for qoder", () => {
    expect(runtimeSupportsEffort("qoder")).toBe(false);
    expect(EFFORT_OPTIONS_BY_RUNTIME.qoder).toBeUndefined();
    expect(modelSuggestions("qoder")).toEqual([
      expect.objectContaining({ value: "", label: "default" }),
    ]);
  });

  it("keeps the frontend resize policy aligned for full-repaint runtimes", () => {
    expect(runtimeClearsOnResize("claude-code")).toBe(true);
    expect(runtimeClearsOnResize("codex")).toBe(true);
    expect(runtimeClearsOnResize("qoder")).toBe(true);
    expect(runtimeClearsOnResize("trae")).toBe(true);
    expect(runtimeClearsOnResize("shell")).toBe(false);
  });

  it("exposes TRAE-native permission modes with only the CLI default model", () => {
    expect(runtimeSupportsPermissionMode("trae")).toBe(true);
    expect(PERMISSION_MODES_BY_RUNTIME.trae.map((mode) => mode.value)).toEqual([
      "default",
      "auto",
      "bypass",
    ]);
    expect(
      inferPermissionMode("trae", [
        "--permission-mode",
        "auto",
      ]),
    ).toBe("auto");
    expect(
      stripPermissionFlags("trae", [
        "--debug",
        "--permission-mode=bypass_permissions",
      ]),
    ).toEqual(["--debug"]);
    expect(runtimeSupportsEffort("trae")).toBe(true);
    expect(EFFORT_OPTIONS_BY_RUNTIME.trae.map((effort) => effort.value)).toEqual(
      ["", "low", "medium", "high", "xhigh"],
    );
    expect(modelSuggestions("trae")).toEqual([
      expect.objectContaining({ value: "", label: "default" }),
    ]);
  });

  it("offers the CLI default model first for every agent runtime", () => {
    for (const runtime of RUNTIME_OPTIONS) {
      expect(modelSuggestions(runtime.value)[0]).toMatchObject({
        value: "",
        label: "default",
      });
    }
  });

  it("offers the current codex model seed", () => {
    expect(modelSuggestions("codex").map((model) => model.value)).toEqual([
      "",
      "gpt-5.6-sol",
      "gpt-5.6-terra",
      "gpt-5.6-luna",
      "gpt-5.5",
      "gpt-5.4",
      "gpt-5.4-mini",
      "gpt-5.3-codex-spark",
    ]);
  });

  it("includes the fable alias for claude-code", () => {
    expect(modelSuggestions("claude-code").map((model) => model.value)).toContain(
      "fable",
    );
  });

  it("includes max and ultra effort for codex", () => {
    expect(
      EFFORT_OPTIONS_BY_RUNTIME.codex.map((effort) => effort.value),
    ).toEqual(["", "low", "medium", "high", "xhigh", "max", "ultra"]);
  });
});

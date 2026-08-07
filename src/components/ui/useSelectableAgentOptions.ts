import { useEffect, useState } from "react";

import { listen } from "@tauri-apps/api/event";

import { api } from "../../lib/api";
import {
  AGENT_ENABLED_CHANGED_EVENT,
  isAgentEnabled,
  STORAGE_DISABLED_AGENTS,
} from "../../lib/settings";
import { RUNTIME_OPTIONS, type RuntimeOption } from "./runtimes";

export function useSelectableAgentOptions(active: boolean): {
  options: RuntimeOption[];
  loaded: boolean;
  loading: boolean;
  checking: boolean;
  error: string | null;
} {
  const [options, setOptions] = useState<RuntimeOption[]>([]);
  const [loaded, setLoaded] = useState(false);
  const [loading, setLoading] = useState(false);
  const [checking, setChecking] = useState(false);
  const [error, setError] = useState<string | null>(null);

  useEffect(() => {
    if (!active) {
      setOptions([]);
      setLoaded(false);
      setLoading(false);
      setChecking(false);
      setError(null);
      return;
    }

    let disposed = false;
    let stop: (() => void) | undefined;
    const load = async () => {
      try {
        const status = await api.runtime.status();
        if (disposed) return;
        const selectable = new Set(
          status.runtimes
            .filter(
              (runtime) =>
                (runtime.effective_source === "detected" ||
                  runtime.effective_source === "override") &&
                isAgentEnabled(runtime.name),
            )
            .map((runtime) => runtime.name),
        );
        setOptions(
          RUNTIME_OPTIONS.filter((option) => selectable.has(option.value)),
        );
        setChecking(status.shell.checking);
        setError(null);
      } catch (loadError) {
        if (!disposed) {
          setChecking(false);
          setError(String(loadError));
        }
      } finally {
        if (!disposed) {
          setLoading(false);
          setLoaded(true);
        }
      }
    };
    const onStorage = (event: StorageEvent) => {
      if (event.key === STORAGE_DISABLED_AGENTS) void load();
    };

    setLoading(true);
    setLoaded(false);
    void load();
    void listen("runtime/changed", () => void load()).then((unlisten) => {
      if (disposed) unlisten();
      else stop = unlisten;
    });
    window.addEventListener(AGENT_ENABLED_CHANGED_EVENT, load);
    window.addEventListener("storage", onStorage);
    return () => {
      disposed = true;
      stop?.();
      window.removeEventListener(AGENT_ENABLED_CHANGED_EVENT, load);
      window.removeEventListener("storage", onStorage);
    };
  }, [active]);

  return { options, loaded, loading, checking, error };
}

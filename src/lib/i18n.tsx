// Lightweight i18n for the Windows+WSL fork.
//
// Strategy: the English source string IS the key. `t("New runner")`
// looks up the Chinese translation in `zh`; a missing entry falls back
// to the English source. So wrapping a string in `t()` is always safe —
// it shows English until its translation is added — which makes the
// "translate incrementally, main surfaces first" rollout painless.
//
// Interpolation: `t("{n} sessions", { n: 3 })` -> "{n} 个会话" -> "3 个会话".

import {
  createContext,
  useCallback,
  useContext,
  useMemo,
  useState,
  type ReactNode,
} from "react";

import { zh } from "./i18n.zh";

export type Lang = "zh" | "en";

const STORAGE_KEY = "app.lang";

export type TFn = (
  source: string,
  vars?: Record<string, string | number>,
) => string;

interface I18nValue {
  lang: Lang;
  setLang: (lang: Lang) => void;
  t: TFn;
}

const I18nContext = createContext<I18nValue | null>(null);

function interpolate(
  s: string,
  vars?: Record<string, string | number>,
): string {
  if (!vars) return s;
  return s.replace(/\{(\w+)\}/g, (_, key: string) =>
    key in vars ? String(vars[key]) : `{${key}}`,
  );
}

function readInitialLang(): Lang {
  // Default to Chinese — this is the 中文版. Users can flip to English in
  // Settings; the choice persists in localStorage.
  const stored =
    typeof localStorage !== "undefined"
      ? (localStorage.getItem(STORAGE_KEY) as Lang | null)
      : null;
  return stored === "en" || stored === "zh" ? stored : "zh";
}

export function LangProvider({ children }: { children: ReactNode }) {
  const [lang, setLangState] = useState<Lang>(readInitialLang);

  const setLang = useCallback((next: Lang) => {
    setLangState(next);
    try {
      localStorage.setItem(STORAGE_KEY, next);
    } catch {
      // Private-mode / no storage — keep the in-memory choice anyway.
    }
  }, []);

  const t = useCallback<TFn>(
    (source, vars) => {
      const resolved = lang === "zh" ? zh[source] ?? source : source;
      return interpolate(resolved, vars);
    },
    [lang],
  );

  const value = useMemo<I18nValue>(
    () => ({ lang, setLang, t }),
    [lang, setLang, t],
  );

  return <I18nContext.Provider value={value}>{children}</I18nContext.Provider>;
}

export function useI18n(): I18nValue {
  const ctx = useContext(I18nContext);
  if (!ctx) {
    throw new Error("useI18n must be used within a LangProvider");
  }
  return ctx;
}

/** Convenience hook when a component only needs the translate function. */
export function useT(): TFn {
  return useI18n().t;
}

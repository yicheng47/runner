import {
  createContext,
  useCallback,
  useContext,
  useEffect,
  useRef,
  useState,
  type ReactNode,
} from "react";

import { X } from "lucide-react";

type ToastTone = "info" | "success" | "error";

interface ToastState {
  message: string;
  tone: ToastTone;
}

interface ToastOptions {
  tone?: ToastTone;
  durationMs?: number | null;
}

interface ToastContextValue {
  showToast: (message: string, options?: ToastOptions) => void;
  hideToast: () => void;
}

const ToastContext = createContext<ToastContextValue | null>(null);

const toneClass: Record<ToastTone, string> = {
  info: "border-line-strong bg-panel text-fg",
  success: "border-accent/40 bg-panel text-accent",
  error: "border-danger/40 bg-panel text-danger",
};

export function ToastProvider({ children }: { children: ReactNode }) {
  const [toast, setToast] = useState<ToastState | null>(null);
  const timerRef = useRef<number | null>(null);

  const clearTimer = useCallback(() => {
    if (timerRef.current != null) {
      window.clearTimeout(timerRef.current);
      timerRef.current = null;
    }
  }, []);

  const hideToast = useCallback(() => {
    clearTimer();
    setToast(null);
  }, [clearTimer]);

  const showToast = useCallback(
    (message: string, options: ToastOptions = {}) => {
      clearTimer();
      setToast({ message, tone: options.tone ?? "info" });
      const durationMs = options.durationMs === undefined ? 6000 : options.durationMs;
      if (durationMs == null) return;
      timerRef.current = window.setTimeout(() => {
        setToast(null);
        timerRef.current = null;
      }, durationMs);
    },
    [clearTimer],
  );

  useEffect(() => clearTimer, [clearTimer]);

  return (
    <ToastContext.Provider value={{ showToast, hideToast }}>
      {children}
      {toast ? (
        <div
          role={toast.tone === "error" ? "alert" : "status"}
          className={`fixed left-1/2 top-5 z-[60] flex w-[min(420px,calc(100vw-32px))] -translate-x-1/2 items-start gap-3 rounded-lg border px-4 py-3 text-sm shadow-[0_8px_24px_rgba(0,0,0,0.5)] ${toneClass[toast.tone]}`}
        >
          <span className="min-w-0 flex-1 break-words leading-5">{toast.message}</span>
          <button
            type="button"
            onClick={hideToast}
            aria-label="Dismiss notification"
            className="mt-0.5 flex size-5 shrink-0 cursor-pointer items-center justify-center rounded text-current opacity-70 transition-opacity hover:opacity-100"
          >
            <X aria-hidden className="h-3.5 w-3.5" />
          </button>
        </div>
      ) : null}
    </ToastContext.Provider>
  );
}

// Co-located with the provider to match UpdateContext's local pattern.
// eslint-disable-next-line react-refresh/only-export-components
export function useToast() {
  const ctx = useContext(ToastContext);
  if (!ctx) throw new Error("useToast must be used within ToastProvider");
  return ctx;
}

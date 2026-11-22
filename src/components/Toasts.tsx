import { CircleCheck, Info, TriangleExclamationPoint, X } from "lucide-react";

import { t } from "../i18n";
import { useHearth } from "../lib/store";

export function Toasts() {
  const toasts = useHearth((s) => s.toasts);
  const dismiss = useHearth((s) => s.dismissToast);
  return (
    <div aria-live="polite" className="fixed bottom-5 right-5 z-50 flex flex-col gap-2 w-[min(420px,calc(100%-40px))]">
      {toasts.map((toast) => {
        const Icon = toast.kind === "error" ? TriangleExclamationPoint : toast.kind === "success" ? CircleCheck : Info;
        const tone =
          toast.kind === "error"
            ? "border-[#5a2a24] text-err"
            : toast.kind === "success"
              ? "border-[#2f4a39] text-ok-text"
              : "border-line-strong text-accent-text";
        return (
          <div
            key={toast.id}
            role={toast.kind === "error" ? "alert" : "status"}
            className={`flex items-start gap-3 p-3.5 rounded-xl bg-card border shadow-2xl shadow-black/40 ${tone}`}
          >
            <Icon size={18} className="shrink-0 mt-0.5" aria-hidden="true" />
            <span className="flex-1 text-ink leading-snug selectable">{toast.text}</span>
            <button
              type="button"
              aria-label={t.actions.close}
              onClick={() => dismiss(toast.id)}
              className="shrink-0 w-7 h-7 rounded-md flex items-center justify-center text-muted hover:text-ink hover:bg-hover"
            >
              <X size={16} aria-hidden="true" />
            </button>
          </div>
        );
      })}
    </div>
  );
}

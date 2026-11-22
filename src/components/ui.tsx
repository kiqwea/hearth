// Базовые элементы интерфейса в стиле Hearth.

import { useEffect, useId, useRef, type ButtonHTMLAttributes, type ReactNode } from "react";

import type { Tone } from "../lib/format";

type Variant = "primary" | "secondary" | "ghost" | "danger";

const VARIANTS: Record<Variant, string> = {
  primary: "bg-accent text-accent-ink font-bold hover:brightness-110 border border-transparent",
  secondary: "bg-[#221d18] text-ink font-semibold border border-line-strong hover:bg-hover",
  ghost: "bg-transparent text-ink border border-line-strong hover:bg-hover",
  danger: "bg-err-bg text-err font-semibold border border-[#5a2a24] hover:brightness-125",
};

export function Button({
  variant = "secondary",
  size = "md",
  className = "",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { variant?: Variant; size?: "md" | "sm" }) {
  const sizing = size === "md" ? "h-11 px-[18px] rounded-[10px]" : "h-9 px-3 rounded-lg text-[13px]";
  return (
    <button
      type="button"
      className={`inline-flex items-center justify-center gap-2 transition disabled:opacity-50 disabled:hover:brightness-100 ${sizing} ${VARIANTS[variant]} ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

export function IconButton({
  label,
  className = "",
  children,
  ...rest
}: ButtonHTMLAttributes<HTMLButtonElement> & { label: string }) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      className={`inline-flex items-center justify-center w-10 h-10 rounded-[9px] bg-chip text-soft hover:bg-hover hover:text-ink transition disabled:opacity-50 ${className}`}
      {...rest}
    >
      {children}
    </button>
  );
}

export function Card({ className = "", children, label }: { className?: string; children: ReactNode; label?: string }) {
  return (
    <section aria-label={label} className={`bg-card border border-line rounded-[14px] ${className}`}>
      {children}
    </section>
  );
}

const DOT: Record<Tone, string> = {
  ok: "bg-ok",
  warn: "bg-warn",
  err: "bg-err",
  off: "bg-off",
  busy: "bg-accent animate-pulse",
};

export function StatusDot({ tone, className = "" }: { tone: Tone; className?: string }) {
  return <span aria-hidden="true" className={`inline-block w-[9px] h-[9px] rounded-full shrink-0 ${DOT[tone]} ${className}`} />;
}

const PILL: Record<Tone, string> = {
  ok: "bg-ok-bg text-ok-text",
  warn: "bg-warn-bg text-warn",
  err: "bg-err-bg text-err",
  off: "bg-chip text-soft",
  busy: "bg-accent-soft text-accent-text",
};

export function StatusPill({ tone, children }: { tone: Tone; children: ReactNode }) {
  return (
    <span className={`inline-flex items-center gap-[7px] px-2.5 py-1 rounded-full font-semibold text-[13px] ${PILL[tone]}`}>
      <StatusDot tone={tone} className="!w-[7px] !h-[7px]" />
      {children}
    </span>
  );
}

export function Chip({ children }: { children: ReactNode }) {
  return <span className="px-2.5 py-1 rounded-full bg-chip text-soft text-[13px]">{children}</span>;
}

export const inputClass =
  "h-11 px-3 rounded-[10px] border border-line-strong bg-card text-ink placeholder:text-faint disabled:opacity-60 w-full";

/** Поле с подписью и подсказкой. `children` получает id для связки с label. */
export function Field({
  label,
  hint,
  children,
  className = "",
}: {
  label: ReactNode;
  hint?: ReactNode;
  children: (id: string) => ReactNode;
  className?: string;
}) {
  const id = useId();
  return (
    <div className={`flex flex-col gap-2 ${className}`}>
      <label htmlFor={id} className="font-semibold">
        {label}
      </label>
      {children(id)}
      {hint && <span className="text-[13px] text-muted">{hint}</span>}
    </div>
  );
}

export function Switch({
  checked,
  onChange,
  label,
  disabled,
}: {
  checked: boolean;
  onChange: (v: boolean) => void;
  label: string;
  disabled?: boolean;
}) {
  return (
    <button
      type="button"
      role="switch"
      aria-checked={checked}
      aria-label={label}
      disabled={disabled}
      onClick={() => onChange(!checked)}
      className={`shrink-0 w-[46px] h-7 rounded-full p-[3px] flex transition disabled:opacity-50 ${
        checked ? "bg-accent justify-end" : "bg-line-strong justify-start"
      }`}
    >
      <span className={`w-[22px] h-[22px] rounded-full ${checked ? "bg-accent-ink" : "bg-soft"}`} />
    </button>
  );
}

/** Кнопки-переключатели: «Выживание / Творческий / Приключение». */
export function Segmented<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: Array<{ value: T; label: string }>;
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div role="group" aria-label={label} className="flex flex-wrap gap-1.5">
      {options.map((o) => {
        const active = o.value === value;
        return (
          <button
            key={o.value}
            type="button"
            aria-pressed={active}
            onClick={() => onChange(o.value)}
            className={`h-10 px-4 rounded-[9px] transition ${
              active ? "bg-accent text-accent-ink font-bold" : "border border-line-strong text-ink hover:bg-hover"
            }`}
          >
            {o.label}
          </button>
        );
      })}
    </div>
  );
}

export function ProgressBar({ value, label }: { value: number | null; label: string }) {
  const pct = value === null ? null : Math.max(0, Math.min(100, value));
  return (
    <span
      role="progressbar"
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={100}
      aria-valuenow={pct ?? undefined}
      className="block h-2 rounded bg-[#2e2822] overflow-hidden"
    >
      <span
        className={`block h-full bg-accent transition-[width] duration-200 ${pct === null ? "w-1/3 animate-pulse" : ""}`}
        style={pct === null ? undefined : { width: `${pct}%` }}
      />
    </span>
  );
}

export function Spinner({ className = "" }: { className?: string }) {
  return (
    <span
      aria-hidden="true"
      className={`inline-block w-7 h-7 rounded-full border-[3px] border-[#3a2e22] border-t-accent animate-spin shrink-0 ${className}`}
    />
  );
}

/** Модальное окно на нативном <dialog>: фокус и Esc работают сами. */
export function Dialog({
  open,
  onClose,
  title,
  children,
  footer,
  wide,
}: {
  open: boolean;
  onClose: () => void;
  title: string;
  children: ReactNode;
  footer?: ReactNode;
  wide?: boolean;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
  }, [open]);
  return (
    <dialog
      ref={ref}
      onClose={onClose}
      onCancel={(e) => {
        e.preventDefault();
        onClose();
      }}
      className={`m-auto bg-card text-ink border border-line rounded-2xl p-0 w-[calc(100%-32px)] ${wide ? "max-w-[640px]" : "max-w-[480px]"}`}
    >
      {open && (
        <div className="flex flex-col gap-4 p-6">
          <h2 className="m-0 text-lg font-bold">{title}</h2>
          <div className="flex flex-col gap-3 text-soft leading-relaxed">{children}</div>
          {footer && <div className="flex flex-wrap justify-end gap-2.5 pt-2">{footer}</div>}
        </div>
      )}
    </dialog>
  );
}

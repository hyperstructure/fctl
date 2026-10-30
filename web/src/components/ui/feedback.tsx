import { CheckCircle2, Info, TriangleAlert, X } from "lucide-react";
import { type ReactNode, useEffect, useSyncExternalStore } from "react";
import { cn } from "@/lib/cn";

export function Spinner({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 24 24" className={cn("size-4 animate-spin", className)} fill="none" aria-hidden="true">
      <circle cx="12" cy="12" r="9" stroke="currentColor" strokeOpacity="0.2" strokeWidth="2.5" />
      <path d="M21 12a9 9 0 0 0-9-9" stroke="currentColor" strokeWidth="2.5" strokeLinecap="round" />
    </svg>
  );
}

export function Skeleton({ className }: { className?: string }) {
  return (
    <div
      className={cn(
        "rounded-md bg-[linear-gradient(90deg,var(--c-raised)_0%,var(--c-line-strong)_50%,var(--c-raised)_100%)] bg-[length:200%_100%] animate-shimmer",
        className,
      )}
    />
  );
}

export function Empty({
  title,
  children,
  action,
  icon,
}: {
  title: string;
  children?: ReactNode;
  action?: ReactNode;
  icon?: ReactNode;
}) {
  return (
    <div className="flex animate-rise flex-col items-center px-6 py-16 text-center">
      {icon && <div className="mb-4 text-ink-4">{icon}</div>}
      <h3 className="font-display text-xl text-ink">{title}</h3>
      {children && <div className="mt-2 max-w-md text-[14px] leading-relaxed text-ink-3">{children}</div>}
      {action && <div className="mt-6">{action}</div>}
    </div>
  );
}

export function Banner({
  tone = "info",
  children,
  className,
}: {
  tone?: "info" | "warn" | "error" | "ok";
  children: ReactNode;
  className?: string;
}) {
  const Icon = tone === "ok" ? CheckCircle2 : tone === "info" ? Info : TriangleAlert;
  const color = {
    info: "border-blue/20 bg-blue-soft text-ink-2 [&_svg]:text-blue",
    warn: "border-amber/25 bg-amber-soft text-ink-2 [&_svg]:text-amber",
    error: "border-danger/25 bg-danger-soft text-ink-2 [&_svg]:text-danger",
    ok: "border-accent/25 bg-accent-soft text-ink-2 [&_svg]:text-accent",
  }[tone];
  return (
    <div
      className={cn(
        "flex animate-rise gap-2.5 rounded-lg border px-3.5 py-2.5 text-[13.5px] leading-relaxed",
        color,
        className,
      )}
      role={tone === "error" ? "alert" : "status"}
    >
      <Icon className="mt-[3px] size-4 shrink-0" />
      {/* A long name or link in a message wraps rather than running off a phone. */}
      <div className="min-w-0 [overflow-wrap:anywhere]">{children}</div>
    </div>
  );
}

// ── toasts ──

interface Toast {
  id: number;
  tone: "ok" | "error" | "info";
  text: string;
}
let toasts: Toast[] = [];
let nextId = 0;
const listeners = new Set<() => void>();
const emit = () => {
  for (const l of listeners) l();
};

export function toast(text: string, tone: Toast["tone"] = "ok") {
  const t = { id: ++nextId, tone, text };
  toasts = [...toasts, t].slice(-4);
  emit();
  setTimeout(() => dismiss(t.id), tone === "error" ? 6000 : 3200);
}
function dismiss(id: number) {
  toasts = toasts.filter((t) => t.id !== id);
  emit();
}

export function Toaster() {
  const list = useSyncExternalStore(
    (fn) => {
      listeners.add(fn);
      return () => listeners.delete(fn);
    },
    () => toasts,
  );
  useEffect(() => () => listeners.clear(), []);
  return (
    <div
      className="pointer-events-none fixed right-[max(1rem,env(safe-area-inset-right))] bottom-[calc(max(1rem,env(safe-area-inset-bottom))-var(--viewport-shift))] left-[max(1rem,env(safe-area-inset-left))] z-50 flex flex-col items-end gap-2"
      aria-live="polite"
    >
      {list.map((t) => (
        <div
          key={t.id}
          className="pointer-events-auto flex max-w-sm animate-rise items-center gap-2.5 rounded-lg border border-line-strong bg-raised/95 py-2 pr-2 pl-3.5 text-[13.5px] text-ink shadow-[0_12px_40px_rgb(0_0_0/0.35)] backdrop-blur-xl"
        >
          <span
            className={cn(
              "size-1.5 shrink-0 rounded-full",
              t.tone === "ok" ? "bg-accent" : t.tone === "error" ? "bg-danger" : "bg-blue",
            )}
          />
          <span className="min-w-0 flex-1">{t.text}</span>
          <button
            type="button"
            aria-label="Dismiss"
            className="rounded p-1 text-ink-4 hover:text-ink"
            onClick={() => dismiss(t.id)}
          >
            <X className="size-3.5" />
          </button>
        </div>
      ))}
    </div>
  );
}

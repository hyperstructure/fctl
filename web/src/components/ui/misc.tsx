import { Check, Copy } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { hue, initials } from "@/lib/format";

export function Logo({ className, withWord = true }: { className?: string; withWord?: boolean }) {
  return (
    <span className={cn("inline-flex items-center gap-2", className)}>
      <svg viewBox="0 0 32 32" className="size-[22px]" aria-hidden="true">
        <rect width="32" height="32" rx="8" fill="#171717" />
        <rect x="0.5" y="0.5" width="31" height="31" rx="7.5" fill="none" stroke="rgb(250 250 250 / 0.12)" />
        <path d="M7 22.5 L15 9.5 h4 L11 22.5z M14 22.5 L22 9.5 h4 L18 22.5z" fill="#ffff5d" />
      </svg>
      {withWord && <span className="font-semibold tracking-[-0.02em] text-ink text-[15px]">fctl</span>}
    </span>
  );
}

export function Avatar({
  user,
  size = 24,
  ring,
}: {
  user: Pick<G.UserRef, "id" | "name" | "avatar_url">;
  size?: number;
  ring?: boolean;
}) {
  const [broken, setBroken] = useState(false);
  const h = hue(user.id);
  const style = { width: size, height: size, fontSize: Math.max(9, size * 0.4) };
  const cls = cn(
    "inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full font-semibold select-none",
    ring && "ring-2 ring-canvas",
  );
  if (user.avatar_url && !broken) {
    return (
      <img
        src={user.avatar_url}
        alt={user.name}
        title={user.name}
        className={cls}
        style={style}
        onError={() => setBroken(true)}
        referrerPolicy="no-referrer"
      />
    );
  }
  return (
    <span
      className={cls}
      title={user.name}
      style={{ ...style, background: AVATAR_FILLS[h % AVATAR_FILLS.length], color: "#171717" }}
    >
      {initials(user.name)}
    </span>
  );
}

/** Initials sit on one of the brand colors, picked by the user's id. */
const AVATAR_FILLS = ["#ffff5d", "#ff00d8", "#007eff", "#eaff81"];

export function AvatarStack({
  users,
  max = 4,
  size = 22,
}: {
  users: G.UserRef[];
  max?: number;
  size?: number;
}) {
  const shown = users.slice(0, max);
  return (
    <span className="flex -space-x-1.5">
      {shown.map((u) => (
        <Avatar key={u.id} user={u} size={size} ring />
      ))}
      {users.length > max && (
        <span
          className="inline-flex items-center justify-center rounded-full bg-raised text-[10px] text-ink-3 ring-2 ring-canvas"
          style={{ width: size, height: size }}
        >
          +{users.length - max}
        </span>
      )}
    </span>
  );
}

export function Kbd({ children }: { children: ReactNode }) {
  return (
    <kbd className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded border border-line-strong bg-raised px-1 font-sans text-[10.5px] font-medium text-ink-3">
      {children}
    </kbd>
  );
}

const STATUS: Record<G.SessionStatus, { label: string; tone: string; dot: string; live?: boolean }> = {
  queued: { label: "Queued", tone: "text-ink-3", dot: "bg-ink-3" },
  provisioning: { label: "Starting", tone: "text-ink-2", dot: "bg-ink-2", live: true },
  preparing: { label: "Preparing", tone: "text-ink-2", dot: "bg-ink-2", live: true },
  running: { label: "Working", tone: "text-accent", dot: "bg-accent", live: true },
  idle: { label: "Waiting", tone: "text-blue", dot: "bg-blue" },
  paused: { label: "Paused", tone: "text-pink", dot: "bg-pink" },
  stopping: { label: "Stopping", tone: "text-ink-3", dot: "bg-ink-3", live: true },
  stopped: { label: "Stopped", tone: "text-ink-3", dot: "bg-ink-4" },
  failed: { label: "Failed", tone: "text-danger", dot: "bg-danger" },
};

export const statusLabel = (s: G.SessionStatus) => STATUS[s].label;

export function StatusDot({ status, className }: { status: G.SessionStatus; className?: string }) {
  const s = STATUS[status];
  return (
    <span className={cn("relative inline-flex size-2 shrink-0", className)}>
      {s.live && (
        <span
          className={cn(
            "absolute inset-0 rounded-full opacity-60 animate-ping [animation-duration:2s]",
            s.dot,
          )}
        />
      )}
      <span className={cn("relative inline-flex size-2 rounded-full", s.dot)} />
    </span>
  );
}

export function StatusPill({ status, className }: { status: G.SessionStatus; className?: string }) {
  const s = STATUS[status];
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-[12.5px] font-medium", s.tone, className)}>
      <StatusDot status={status} />
      {s.label}
    </span>
  );
}

export function CopyButton({
  text,
  label = "Copy",
  className,
}: {
  text: string;
  label?: string;
  className?: string;
}) {
  const [done, setDone] = useState(false);
  const t = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(t.current), []);
  return (
    <button
      type="button"
      className={cn(
        "inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-ink-3 transition-colors hover:bg-raised hover:text-ink",
        className,
      )}
      onClick={async () => {
        await navigator.clipboard.writeText(text);
        setDone(true);
        clearTimeout(t.current);
        t.current = setTimeout(() => setDone(false), 1600);
      }}
    >
      <span className="relative size-3.5">
        <Copy
          className={cn(
            "absolute inset-0 size-3.5 transition-all duration-200",
            done && "scale-50 opacity-0",
          )}
        />
        <Check
          className={cn(
            "absolute inset-0 size-3.5 text-accent transition-all duration-200",
            !done && "scale-50 opacity-0",
          )}
        />
      </span>
      {done ? "Copied" : label}
    </button>
  );
}

/** A secret or command shown once, with a copy button. */
export function CodeBox({ value, className }: { value: string; className?: string }) {
  return (
    <div
      className={cn(
        "flex items-center gap-2 rounded-md border border-line-strong bg-sunken py-1.5 pr-1.5 pl-3",
        className,
      )}
    >
      <code className="min-w-0 flex-1 overflow-x-auto font-mono text-[12.5px] whitespace-nowrap text-ink">
        {value}
      </code>
      <CopyButton text={value} />
    </div>
  );
}

export function Section({
  title,
  description,
  children,
  actions,
}: {
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <section className="grid gap-x-10 gap-y-4 border-t border-line py-8 first:border-t-0 md:grid-cols-[260px_1fr]">
      <div>
        <h2 className="text-[14px] font-semibold text-ink">{title}</h2>
        {description && <p className="mt-1 text-[13px] leading-relaxed text-ink-3">{description}</p>}
        {actions && <div className="mt-3">{actions}</div>}
      </div>
      <div className="min-w-0">{children}</div>
    </section>
  );
}

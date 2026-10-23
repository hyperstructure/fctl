import { mark } from "@hyperstructure/design/brand";
import { Check, Copy } from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { hue, initials } from "@/lib/format";

/**
 * fctl's mark and name; `large` makes it larger on phones, in the top bar that heads every page
 * and in the sidebar it opens. From 768 px up, where the top bar is hidden, it's the usual size.
 */
export function Logo({
  className,
  withWord = true,
  large = false,
}: {
  className?: string;
  withWord?: boolean;
  large?: boolean;
}) {
  return (
    <span className={cn("inline-flex items-center", large ? "gap-2.5 md:gap-2" : "gap-2", className)}>
      <svg
        viewBox={mark.viewBox}
        className={large ? "size-[26px] md:size-[22px]" : "size-[22px]"}
        aria-hidden="true"
      >
        <rect width="256" height="256" rx={mark.radius} fill={mark.background} />
        <path d={mark.path} fill={mark.foreground} fillRule="evenodd" />
      </svg>
      {withWord && (
        <span
          className={cn(
            "font-semibold tracking-[-0.02em] text-ink",
            large ? "text-[17px] md:text-[15px]" : "text-[15px]",
          )}
        >
          fctl
        </span>
      )}
    </span>
  );
}

/**
 * Whether an avatar loads from another site than the app's: the browser sends the reader's cookies to the app's own
 * addresses, so an avatar there would act for everyone who sees it. The control plane takes no such avatars any more;
 * these are ones it kept from before.
 */
function offSite(url: string): boolean {
  try {
    return new URL(url, globalThis.location?.href).origin !== globalThis.location?.origin;
  } catch {
    return false;
  }
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
  if (user.avatar_url && !broken && offSite(user.avatar_url)) {
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

/** A session's status; `label` says it otherwise, as for a review agent's session, which waits for nobody. */
export function StatusPill({
  status,
  label,
  className,
}: {
  status: G.SessionStatus;
  label?: string;
  className?: string;
}) {
  const s = STATUS[status];
  return (
    <span className={cn("inline-flex items-center gap-1.5 text-[12.5px] font-medium", s.tone, className)}>
      <StatusDot status={status} />
      {label ?? s.label}
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
        "inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-ink-3 transition-colors hover:bg-hover hover:text-ink",
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
  id,
  title,
  description,
  children,
  actions,
}: {
  /** What a link to it names after `#`. */
  id?: string;
  title: ReactNode;
  description?: ReactNode;
  children: ReactNode;
  actions?: ReactNode;
}) {
  return (
    <section
      id={id}
      className="grid scroll-mt-6 gap-x-10 gap-y-4 border-t border-line py-8 first:border-t-0 md:grid-cols-[260px_1fr]"
    >
      <div>
        <h2 className="text-[14px] font-semibold text-ink">{title}</h2>
        {description && <p className="mt-1 text-[13px] leading-relaxed text-ink-3">{description}</p>}
        {actions && <div className="mt-3">{actions}</div>}
      </div>
      <div className="min-w-0">{children}</div>
    </section>
  );
}

/**
 * A line of facts, a dot between each. Each fact carries its dot before it, in a margin the line
 * hides: where the facts wrap, the next line starts with a fact, not a dot. One that hides on a
 * phone takes its dot along.
 */
export function Meta({
  parts,
}: {
  parts: (false | null | undefined | "" | { key: string; node: ReactNode; className?: string })[];
}) {
  const shown = parts.filter((p): p is { key: string; node: ReactNode; className?: string } => !!p);
  return (
    <div className="mt-1 overflow-hidden">
      <div className="-ml-4 flex min-w-0 flex-wrap items-center gap-y-1 text-[12.5px] text-ink-4">
        {shown.map((p) => (
          <span
            key={p.key}
            className={cn(
              "relative inline-flex items-center pl-4 before:absolute before:left-[6px] before:content-['·']",
              p.className,
            )}
          >
            {p.node}
          </span>
        ))}
      </div>
    </div>
  );
}

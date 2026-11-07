import { mark } from "@hyperstructure/design/brand";
import { Check, Copy } from "lucide-react";
import { createContext, type ReactNode, use, useEffect, useRef, useState } from "react";
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

/** Where the control plane keeps a picture someone uploaded (`fctl_domain::avatar::user_path`). */
const UPLOADED = /^\/api\/v1\/users\/[A-Za-z0-9_]+\/avatar\?v=\d+$/;

/**
 * Whether to show an avatar: one the person uploaded, or an address from before people uploaded them, on another site
 * than the app's. The browser sends the reader's cookies to the app's own addresses, so an address there would act for
 * everyone who sees it.
 */
function shown(url: string): boolean {
  if (UPLOADED.test(url)) return true;
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
  title = user.name,
}: {
  user: Pick<G.UserRef, "id" | "name" | "avatar_url">;
  size?: number;
  ring?: boolean;
  /** What it says where the pointer rests: the person's name, unless it says more. */
  title?: string;
}) {
  const [broken, setBroken] = useState<string>();
  const h = hue(user.id);
  const style = { width: size, height: size, fontSize: Math.max(9, size * 0.4) };
  const cls = cn(
    "inline-flex shrink-0 items-center justify-center overflow-hidden rounded-full font-semibold select-none",
    ring && "ring-2 ring-canvas",
  );
  if (user.avatar_url && broken !== user.avatar_url && shown(user.avatar_url)) {
    return (
      <img
        src={user.avatar_url}
        alt={user.name}
        title={title}
        className={cn(cls, "object-cover")}
        style={style}
        onError={() => setBroken(user.avatar_url)}
        referrerPolicy="no-referrer"
      />
    );
  }
  return (
    <span
      className={cls}
      title={title}
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

/** A key, as a keycap: small in hints, larger (`md`) where shortcuts are listed. */
export function Kbd({ children, size = "sm" }: { children: ReactNode; size?: "sm" | "md" }) {
  return (
    <kbd
      className={cn(
        "inline-flex items-center justify-center rounded border border-line-strong bg-raised font-sans font-medium",
        size === "sm"
          ? "h-[18px] min-w-[18px] px-1 text-[10.5px] text-ink-3"
          : "h-[22px] min-w-[22px] px-1.5 text-[11.5px] text-ink-2 shadow-[inset_0_-1px_0_var(--c-line-strong)]",
      )}
    >
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

/**
 * A secret or command shown once, with a copy button. A command of several lines (shell
 * continuations) keeps them, with the button at its top.
 */
export function CodeBox({ value, className }: { value: string; className?: string }) {
  const lines = value.includes("\n");
  return (
    <div
      className={cn(
        "flex gap-2 rounded-md border border-line-strong bg-sunken py-1.5 pr-1.5 pl-3",
        lines ? "items-start" : "items-center",
        className,
      )}
    >
      <code
        className={cn(
          "min-w-0 flex-1 overflow-x-auto font-mono text-[12.5px] text-ink",
          lines ? "py-1 leading-relaxed whitespace-pre" : "whitespace-nowrap",
        )}
      >
        {value}
      </code>
      <CopyButton text={value} />
    </div>
  );
}

/**
 * How the Sections in it lay out. `beside`, the default: the title and description in a column
 * beside what the section holds, from 768 px. Settings' pages stack them instead: `page`, a
 * section that is a page of its own, under the page's title; `part`, one of several on a page.
 */
export const SectionLayout = createContext<"beside" | "page" | "part">("beside");

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
  const layout = use(SectionLayout);
  if (layout === "beside")
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
  const page = layout === "page";
  return (
    <section id={id} className={cn("scroll-mt-6", !page && "not-first:mt-14")}>
      {/* Its actions beside the title, or under the description where they don't fit. */}
      <div className="flex flex-wrap items-start gap-x-6 gap-y-3">
        <div className="min-w-[min(100%,18rem)] flex-1">
          {page ? (
            // The page's heading: larger where it heads the screen, as on a phone (routes/settings.tsx).
            <h2 className="font-display text-[22px] leading-[1.15] font-medium tracking-[-0.018em] text-ink @max-2xl:text-[28px]">
              {title}
            </h2>
          ) : (
            <h3 className="text-[15px] font-semibold tracking-[-0.005em] text-ink">{title}</h3>
          )}
          {description && (
            <p
              className={cn(
                "max-w-[64ch] leading-relaxed text-pretty text-ink-3",
                page ? "mt-2 text-[14px]" : "mt-1 text-[13px]",
              )}
            >
              {description}
            </p>
          )}
        </div>
        {actions && <div className="shrink-0">{actions}</div>}
      </div>
      <div className={cn("min-w-0", page ? "mt-7" : "mt-5")}>{children}</div>
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

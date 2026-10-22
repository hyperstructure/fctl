// The lines of the transcript besides the agent's prose and people's
// messages: milestones, and the steps of the agent's work, which open.
//
// Their icons sit in a rail down the transcript's left edge, 24 px wide, on
// tiles for tool calls; what they say starts 10 px after it, where people's
// messages start too. What a line opens into hangs from a thread down from
// its icon. Lines inside a run or a subagent's work (`nested`) are smaller,
// with their icons bare.

import { ChevronRight, CircleAlert } from "lucide-react";
import { Component, type ReactNode } from "react";
import { cn } from "@/lib/cn";

export function Milestone({
  icon,
  children,
  tone = "muted",
}: {
  icon: ReactNode;
  children: ReactNode;
  tone?: "muted" | "warn" | "error" | "info" | "ok";
}) {
  return (
    <div
      className={cn(
        "flex items-center gap-2.5 py-1.5 text-[12.5px]",
        tone === "muted" && "text-ink-4",
        tone === "warn" && "text-amber",
        tone === "error" && "text-danger",
        tone === "info" && "text-blue",
        tone === "ok" && "text-accent",
      )}
    >
      <RailIcon>{icon}</RailIcon>
      <span className="min-w-0">{children}</span>
    </div>
  );
}

/** An icon in the rail, without a tile. */
export function RailIcon({ children, className }: { children: ReactNode; className?: string }) {
  return (
    <span className={cn("flex size-6 shrink-0 items-center justify-center [&_svg]:size-3.5", className)}>
      {children}
    </span>
  );
}

/**
 * A tool call's icon, on a tile in the rail, red when it failed; bare inside a run. A run's tile has
 * the edge of another showing under it, as a stack of them.
 */
export function Tile({
  children,
  nested,
  error,
  stacked,
}: {
  children: ReactNode;
  nested?: boolean;
  error?: boolean;
  stacked?: boolean;
}) {
  if (nested) {
    return (
      <span
        className={cn(
          "flex size-3.5 shrink-0 items-center justify-center transition-colors [&_svg]:size-3.5",
          error ? "text-danger" : "text-ink-4 group-hover/line:text-ink-3",
        )}
      >
        {children}
      </span>
    );
  }
  return (
    <span
      className={cn(
        "flex size-6 shrink-0 items-center justify-center rounded-[7px] border transition-colors [&_svg]:size-3.5",
        error
          ? "border-danger/30 bg-danger-soft text-danger"
          : "border-line bg-raised text-ink-3 group-hover/line:text-ink-2",
        stacked && "shadow-[0_3px_0_-1.5px_var(--c-raised),0_3px_0_-0.5px_var(--c-line-strong)]",
      )}
    >
      {children}
    </span>
  );
}

/** The chevron of a line that opens, which turns down as it does. */
export function Chevron({ open, className }: { open: boolean; className?: string }) {
  return (
    <ChevronRight
      aria-hidden="true"
      className={cn(
        "size-3.5 shrink-0 text-ink-4 transition duration-200 ease-out group-hover/line:text-ink-3",
        open && "rotate-90",
        className,
      )}
    />
  );
}

/** A milestone that opens (the sandbox's setup, the host's logs): its chevron shows when there is more. */
export function MilestoneButton({
  icon,
  open,
  onToggle,
  controls,
  more,
  className,
  children,
}: {
  icon: ReactNode;
  open: boolean;
  onToggle: () => void;
  controls?: string;
  more: boolean;
  className?: string;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-expanded={more ? open : undefined}
      aria-controls={more && open ? controls : undefined}
      className={cn(
        "group/line flex max-w-full items-center gap-2.5 text-[12.5px] transition-colors hover:text-ink-2",
        className ?? "text-ink-4",
      )}
    >
      <RailIcon>{icon}</RailIcon>
      <span className="min-w-0 truncate">{children}</span>
      {more && <Chevron open={open} className="-ml-1 size-3" />}
    </button>
  );
}

/**
 * A line that opens: its icon, what it says (which truncates, with the chevron after it) and, at the
 * right, what it says about itself. A line that has to be read whole (`wrap`) wraps instead, its
 * chevron after its last word. Its hover fill reaches a little past the column on each side.
 */
export function OpenLine({
  icon,
  open,
  onToggle,
  controls,
  nested,
  error,
  title,
  meta,
  wrap,
  children,
}: {
  icon: ReactNode;
  open: boolean;
  onToggle: () => void;
  /** The id of what it opens. */
  controls?: string;
  nested?: boolean;
  error?: boolean;
  /** Its whole text, for a pointer that rests on it. */
  title?: string;
  meta?: ReactNode;
  /** What it says, wrapping, in place of `children`. */
  wrap?: { text: string; className?: string };
  children?: ReactNode;
}) {
  // A line as tall as its icon: what is beside the icon lines up with it.
  const tall = nested ? "h-5" : "h-6";
  const words = wrap?.text.split(" ") ?? [];
  const last = words.pop();
  return (
    <div className="-mx-1.5">
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        aria-controls={open ? controls : undefined}
        title={title}
        className={cn(
          "group/line flex w-full items-start rounded-md px-1.5 py-1 text-left leading-5 transition-colors duration-150 hover:bg-hover active:bg-hover pointer-coarse:py-1.5",
          nested ? "gap-2 text-[13px]" : "gap-2.5 text-[13.5px]",
          error ? "text-danger" : "text-ink-2",
        )}
      >
        <span className={cn("flex shrink-0 items-center", tall)}>{icon}</span>
        {wrap ? (
          <span className={cn("min-w-0 flex-1 text-pretty", !nested && "py-0.5", wrap.className)}>
            {words.length > 0 && `${words.join(" ")} `}
            <span className="whitespace-nowrap">
              {last}
              <Chevron open={open} className="ml-1.5 inline-block align-[-3px]" />
            </span>
          </span>
        ) : (
          <span className={cn("flex min-w-0 flex-1 items-center gap-1.5", tall)}>
            <span className="flex min-w-0 items-baseline gap-1.5">{children}</span>
            <Chevron open={open} />
          </span>
        )}
        {meta && (
          <span className={cn("flex shrink-0 items-center pl-2 text-[11.5px] text-ink-4 tabular-nums", tall)}>
            {meta}
          </span>
        )}
      </button>
    </div>
  );
}

/** What a line opened, under it: a thread down from its icon, and the rest where its text starts. */
export function Thread({
  id,
  nested,
  tone,
  className,
  children,
}: {
  id?: string;
  nested?: boolean;
  tone?: "thought";
  className?: string;
  children: ReactNode;
}) {
  return (
    <div
      id={id}
      className={cn(
        "border-l",
        nested ? "ml-[6.5px] pl-[14.5px]" : "ml-[11.5px] pl-[21.5px]",
        tone === "thought" ? "border-violet/25" : "border-line",
        className,
      )}
    >
      {children}
    </div>
  );
}

/**
 * One item of the transcript, or a line in its place when it can't be shown: events come from the session's sandbox,
 * where anything can pose as the agent, and one this app can't read mustn't take the page down for everyone.
 */
export class Unreadable extends Component<{ children: ReactNode }, { failed: boolean }> {
  state = { failed: false };

  static getDerivedStateFromError() {
    return { failed: true };
  }

  render() {
    return this.state.failed ? (
      <Milestone icon={<CircleAlert />} tone="warn">
        This event couldn't be shown
      </Milestone>
    ) : (
      this.props.children
    );
  }
}

// An earlier session of an autopilot's own, at its message box. A fresh session of its own takes
// over after 12 hours (or past its handoff size, or as people ask), and from then on the earlier
// one's agent no longer acts as the autopilot: its `fctl` reaches nothing, so a message there gets
// the work done in that session alone, rather than planned and handed to sessions. Above the box a
// line says where the autopilot works now, and a message asks first, in a card over the box: send
// it there, and go with it, or here anyway. Once sent here anyway, the page asks no more.

import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowUp, X } from "lucide-react";
import { forwardRef, type KeyboardEvent, type ReactNode, useEffect, useId, useRef } from "react";
import { useAutopilot, useNow } from "@/components/project/autopilot";
import { AutopilotIcon, IconButton, Spinner, toast } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { movedOn, sessionWord } from "@/lib/autopilot";
import { cn } from "@/lib/cn";
import { agoShortWords, stamp } from "@/lib/format";
import { keys } from "@/lib/queries";
import type { SentFiles } from "./attachments";

/** Where the autopilot works now, for an earlier session of its own: what its message box needs. */
export interface MovedOn {
  /** The session it works in now. */
  session: G.AutopilotSession;
  /** The autopilot is paused: it acts again once someone resumes it. */
  paused: boolean;
  /** How that session stands, beside its dot. */
  word: ReturnType<typeof sessionWord>;
  /** Why the message can't go there, when it can't; until that session loads, the reader may try. */
  blocked?: string;
  /** Sends the message there, then goes there. */
  send: (text: string, files: SentFiles) => Promise<void>;
}

/** Where the autopilot works now, when `session` is an earlier session of its own. */
export function useMovedOn(session: G.Session | undefined): MovedOn | undefined {
  const navigate = useNavigate();
  const pilot = session?.autopilot?.role === "pilot";
  const a = useAutopilot(session?.autopilot?.project_slug ?? "", pilot).data;
  const to = pilot && session ? movedOn(a, session.id) : undefined;
  // Whether the reader may write there; the page it goes to has it at once.
  const there = useQuery({
    queryKey: keys.session(to?.id ?? ""),
    queryFn: () => api.session(to?.id ?? ""),
    enabled: !!to,
    staleTime: 30_000,
  });
  if (!a || !to) return;
  return {
    session: to,
    paused: a.state === "paused",
    word: sessionWord(a),
    blocked: blockedOf(to, there.data?.role, there.error),
    send: async (text, files) => {
      await api.sendMessage(to.id, {
        text,
        client_id: `web-${Date.now().toString(36)}-moved`,
        attachments: files.attachments.map((f) => f.id),
      });
      for (const url of Object.values(files.previews)) URL.revokeObjectURL(url);
      toast(`Sent to #${to.number}, where the autopilot works now`);
      void navigate({ to: "/s/$id", params: { id: to.id } });
    },
  };
}

/** Why a message can't go to the session the autopilot works in now, when it can't. */
export function blockedOf(
  to: Pick<G.AutopilotSession, "status">,
  role: G.SessionRole | undefined,
  error: unknown,
): string | undefined {
  if (to.status === "failed") return "It failed: fctl starts the autopilot a fresh session at its next wake.";
  if (error instanceof ApiFailure && (error.status === 403 || error.status === 404))
    return "You can't open that session, so you can't message it.";
  if (role && role !== "editor" && role !== "owner")
    return "You can follow it, but not message it: ask its owner for the editor role.";
}

/** The dot and word of how that session stands, in the status colors sessions have everywhere. */
const WORD_TONES = {
  accent: { dot: "bg-accent", word: "text-accent" },
  pink: { dot: "bg-pink", word: "text-pink" },
  ink: { dot: "bg-ink-4", word: "text-ink-3" },
  danger: { dot: "bg-danger", word: "text-danger" },
} as const;

/** The line above the message box: where the autopilot works now, a link there. */
export function MovedOnLine({ to }: { to: MovedOn }) {
  return (
    <span className="animate-fade flex min-w-0 items-center gap-1.5 text-ink-3">
      <AutopilotIcon className="size-3 shrink-0 text-amber" />
      <span className="shrink-0">The autopilot works in</span>
      <Link
        to="/s/$id"
        params={{ id: to.session.id }}
        title="Go to the session the autopilot works in now"
        className="shrink-0 rounded font-mono font-medium text-ink-2 underline decoration-ink-4/60 underline-offset-2 transition-colors hover:text-ink hover:decoration-ink"
      >
        #{to.session.number}
      </Link>
      <span className="truncate">now, not in this earlier session</span>
    </span>
  );
}

/**
 * Asks before a message goes to an earlier session of the autopilot's own: send it to the one it
 * works in now (first, and focused: Enter again sends it there), or here anyway. Escape, or ×,
 * goes back to the message, as it was.
 */
export function MovedOnCard({
  to,
  nudge,
  sending,
  resume,
  onThere,
  onHere,
  onClose,
}: {
  to: MovedOn;
  /** Changes as people send again while it asks: the focus comes back to it. */
  nudge?: number;
  /** Where the message is on its way to. */
  sending?: "here" | "there";
  /** Sending it here resumes this failed session. */
  resume?: boolean;
  onThere: () => void;
  onHere: () => void;
  onClose: () => void;
}) {
  const now = useNow(30_000);
  const title = useId();
  const text = useId();
  const first = useRef<HTMLButtonElement>(null);
  const second = useRef<HTMLButtonElement>(null);
  // As it opens, and as people send again, the way to where the autopilot is has the focus.
  // biome-ignore lint/correctness/useExhaustiveDependencies: nudge is what brings it back.
  useEffect(() => {
    (first.current?.disabled ? second.current : first.current)?.focus();
  }, [nudge]);
  const n = `#${to.session.number}`;
  const onKey = (e: KeyboardEvent<HTMLElement>) => {
    if (e.key === "Escape") {
      e.preventDefault();
      e.stopPropagation();
      onClose();
    } else if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      const ways = [first.current, second.current].filter((b): b is HTMLButtonElement => !!b && !b.disabled);
      const at = ways.indexOf(document.activeElement as HTMLButtonElement);
      const next = ways[(at + (e.key === "ArrowDown" ? 1 : ways.length - 1)) % ways.length];
      if (!next) return;
      e.preventDefault();
      next.focus();
    }
  };
  const there = to.blocked
    ? to.blocked
    : to.paused
      ? "The autopilot is paused, and acts again once someone resumes it. You go with your message."
      : "The autopilot takes it up there, and you go with your message.";
  return (
    <section
      role="alertdialog"
      aria-labelledby={title}
      aria-describedby={text}
      onKeyDown={onKey}
      className={cn(
        "animate-rise rounded-2xl border border-amber/30 p-4 shadow-[0_10px_40px_rgb(0_0_0/0.22)] backdrop-blur-xl",
        "bg-surface/95 bg-[linear-gradient(to_bottom,var(--c-amber-soft),transparent_70%)]",
      )}
    >
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full bg-amber-soft text-amber">
          <AutopilotIcon className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="text-[12px] font-medium text-amber">An earlier session of the autopilot</p>
          <h2 id={title} className="mt-1 text-[15px] leading-snug font-medium text-ink">
            The autopilot works in <span className="font-mono">{n}</span> now
          </h2>
          <p id={text} className="mt-1 text-[13px] leading-relaxed text-ink-3">
            This session's agent no longer acts as the autopilot: it can't start sessions or steer them, so it
            would take your message on alone.
          </p>
        </div>
        <IconButton label="Keep editing" onClick={onClose} className="-mt-1 -mr-1.5 size-7 shrink-0">
          <X className="size-4" />
        </IconButton>
      </div>
      <div className="mt-3 flex flex-col gap-1.5 md:pl-10">
        <Way
          ref={first}
          primary
          disabled={!!to.blocked || !!sending}
          busy={sending === "there"}
          onClick={onThere}
          title={`Send to ${n}`}
          meta={
            <span className="flex min-w-0 items-center gap-1.5 text-[12px] font-normal text-ink-4">
              <span className={cn("size-1.5 shrink-0 rounded-full", WORD_TONES[to.word.tone].dot)} />
              <span className={WORD_TONES[to.word.tone].word}>{to.word.word}</span>
              <span className="truncate" title={stamp(to.session.started_at)}>
                · started {agoShortWords(to.session.started_at, now)}
              </span>
            </span>
          }
          line={there}
        />
        <Way
          ref={second}
          disabled={!!sending}
          busy={sending === "here"}
          onClick={onHere}
          title={resume ? "Resume here anyway" : "Send here anyway"}
          line="This session's agent takes it up on its own."
        />
      </div>
      <p className="mt-2.5 text-[11.5px] text-ink-4 max-sm:hidden md:pl-10">
        ↑↓ choose · ↵ send · esc back to your message
      </p>
    </section>
  );
}

/** One way the message can go: what it does, a line of what follows, and the send circle. */
const Way = forwardRef<
  HTMLButtonElement,
  {
    primary?: boolean;
    disabled?: boolean;
    busy?: boolean;
    onClick: () => void;
    title: string;
    meta?: ReactNode;
    line: string;
  }
>(function Way({ primary, disabled, busy, onClick, title, meta, line }, ref) {
  return (
    <button
      ref={ref}
      type="button"
      disabled={disabled}
      onClick={onClick}
      className={cn(
        "group flex w-full items-center gap-3 rounded-xl border px-3.5 py-2.5 text-left",
        "transition-[background-color,border-color,transform] duration-150 active:scale-[0.99] disabled:active:scale-100",
        primary
          ? "border-line-strong bg-raised hover:border-ink-4 focus-visible:border-ink-3"
          : "border-line hover:border-line-strong hover:bg-hover/60",
        disabled && !busy && "opacity-60",
      )}
    >
      <span className="min-w-0 flex-1">
        <span className="flex min-w-0 flex-wrap items-center gap-x-2.5 gap-y-0.5">
          <span className="text-[13.5px] font-medium text-ink">{title}</span>
          {meta}
        </span>
        <span className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">{line}</span>
      </span>
      <span
        aria-hidden="true"
        className={cn(
          "flex size-7 shrink-0 items-center justify-center rounded-full transition-transform duration-200 ease-[var(--ease-spring)]",
          primary
            ? "bg-ink text-canvas group-enabled:group-hover:scale-105"
            : "border border-line-strong text-ink-3 group-enabled:group-hover:text-ink",
        )}
      >
        {busy ? <Spinner className="size-3.5" /> : <ArrowUp className="size-3.5" />}
      </span>
    </button>
  );
});

// What came from a session's ticket tracker, as the transcript shows it: a ticket someone delegated
// to fctl in Linear (or mentioned it on, or replied to its agent on), with the ticket as a card and
// what fctl told the agent a click away; and what fctl did in the tracker for the agent, as lines
// that open into what it posted.

import { ArrowUpRight, Compass } from "lucide-react";
import { memo, useId, useState } from "react";
import { Markdown, Pictures } from "@/components/markdown";
import { Avatar, GithubIcon, LinearIcon } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { stateTone } from "@/lib/tickets";
import { useRevealed } from "./find";
import { When } from "./follow-up";
import { MilestoneButton, RailIcon, Thread } from "./lines";

export function TrackerIcon({ tracker, className }: { tracker: G.Tracker; className?: string }) {
  return tracker === "github" ? <GithubIcon className={className} /> : <LinearIcon className={className} />;
}

/** The ticket, by its key, as a link to it in its tracker. */
export function TicketKey({
  ticket,
  className,
}: {
  ticket: G.SessionTicket | G.TicketLink;
  className?: string;
}) {
  const tracker = "tracker" in ticket ? ticket.tracker : "linear";
  return (
    <a
      href={ticket.url}
      target="_blank"
      rel="noreferrer"
      title={ticket.title ? `${ticket.key}: ${ticket.title}` : ticket.key}
      className={cn(
        "inline-flex items-center gap-1 font-mono text-[12px] text-ink-3 underline-offset-2 transition-colors hover:text-ink hover:underline",
        className,
      )}
    >
      <TrackerIcon tracker={tracker} className="size-3 shrink-0" />
      {ticket.key}
    </a>
  );
}

const DID: Record<G.TicketEvent, string> = {
  delegated: "delegated",
  mentioned: "mentioned fctl on",
  replied: "replied on",
};

/** A message to the agent that came from a ticket tracker. */
export const TicketMessageView = memo(function TicketMessageView({
  message,
  author,
  text,
  ts,
  findKey,
}: {
  message: G.TicketMessage;
  /** Whom fctl knows them as. */
  author?: G.UserRef;
  /** What fctl told the agent. */
  text: string;
  ts: number;
  /** The message's key in the transcript, which finding in it knows it by. */
  findKey?: string;
}) {
  const t = message.ticket;
  const name = author?.name ?? message.by?.name ?? "Someone";
  const where = t.tracker === "linear" ? "in Linear" : "on GitHub";
  return (
    <div className="group/msg animate-rise py-3">
      <div className="mb-1.5 flex items-start gap-2.5 text-[12.5px]">
        <RailIcon>
          <Avatar
            user={author ?? { id: name, name, avatar_url: message.by?.avatar_url ?? undefined }}
            size={20}
          />
        </RailIcon>
        <div className="flex min-h-6 min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
          <span className="font-medium text-ink">{name}</span>
          <span className="inline-flex flex-wrap items-center gap-1.5 text-ink-4">
            {DID[message.event]} <TicketKey ticket={t} />
            {message.event === "delegated" ? ` to fctl ${where}` : ` ${where}`}
          </span>
          <When at={ts} />
        </div>
      </div>
      <div className="ml-[34px] overflow-hidden rounded-xl rounded-tl-sm border border-line bg-raised/50 wrap-anywhere">
        {message.event === "delegated" ? (
          <TicketCard message={message} />
        ) : (
          <>
            <a
              href={t.url}
              target="_blank"
              rel="noreferrer"
              className="flex min-w-0 items-center gap-1.5 border-b border-line bg-sunken/60 px-4 py-1.5 text-[11.5px] text-ink-3 transition-colors hover:text-ink"
            >
              <TrackerIcon tracker={t.tracker} className="size-3 shrink-0" />
              <span className="shrink-0 font-mono">{t.key}</span>
              {t.title && <span className="truncate">{t.title}</span>}
            </a>
            <Body text={message.body ?? ""} />
          </>
        )}
      </div>
      <Told text={text} findKey={findKey} />
    </div>
  );
});

/** A delegated ticket: its key, state and labels, its title and its description. */
function TicketCard({ message }: { message: G.TicketMessage }) {
  const t = message.ticket;
  const [open, setOpen] = useState(false);
  const id = useId();
  const description = message.description?.trim() ?? "";
  const long = description.length > 360 || description.split("\n").length > 6;
  return (
    <div className="px-4 py-3">
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[11.5px]">
        <a
          href={t.url}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 font-mono text-ink-3 transition-colors hover:text-ink"
        >
          <TrackerIcon tracker={t.tracker} className="size-3" />
          {t.key}
          <ArrowUpRight className="size-3 text-ink-4" />
        </a>
        {message.state && (
          <span className={cn("inline-flex items-center gap-1", stateTone(message.state.kind))}>
            <StateDot kind={message.state.kind} />
            {message.state.name}
          </span>
        )}
        {message.labels?.map((l) => (
          <span key={l} className="rounded-full border border-line px-1.5 py-px leading-4 text-ink-3">
            {l}
          </span>
        ))}
        {message.explore && (
          <span className="inline-flex items-center gap-1 rounded-full border border-blue/30 bg-blue-soft px-1.5 py-px leading-4 text-blue">
            <Compass className="size-3" aria-hidden="true" />
            Exploration
          </span>
        )}
      </div>
      {t.title && <h3 className="mt-1.5 text-[15px] leading-snug font-semibold text-ink">{t.title}</h3>}
      {description ? (
        <>
          <div id={id} className={cn("mt-1 text-[13.5px] text-ink-2", long && !open && "line-clamp-6")}>
            <Pictures.Provider value={false}>
              <Markdown text={description} className="prose-note" />
            </Pictures.Provider>
          </div>
          {long && (
            <button
              type="button"
              onClick={() => setOpen(!open)}
              aria-expanded={open}
              aria-controls={id}
              className="mt-1 text-[12.5px] text-ink-3 transition-colors hover:text-ink"
            >
              {open ? "Show less" : "Show all"}
            </button>
          )}
        </>
      ) : (
        <p className="mt-1 text-[13px] text-ink-4">No description.</p>
      )}
    </div>
  );
}

/** A state's dot, filled as far as Linear fills its circles: empty to do, half under way, full done. */
export function StateDot({ kind, className }: { kind?: G.StateKind; className?: string }) {
  const fill = kind === "started" ? 0.5 : kind === "completed" || kind === "canceled" ? 1 : 0;
  return (
    <svg viewBox="0 0 12 12" className={cn("size-3 shrink-0", className)} aria-hidden="true">
      <circle cx="6" cy="6" r="4.75" fill="none" stroke="currentColor" strokeWidth="1.5" />
      {fill === 0.5 && <path d="M6 2.5a3.5 3.5 0 0 1 0 7Z" fill="currentColor" />}
      {fill === 1 && <circle cx="6" cy="6" r="3.5" fill="currentColor" />}
    </svg>
  );
}

function Body({ text }: { text: string }) {
  return (
    <div className="px-4 py-2.5 text-[14.5px] leading-[1.6] text-ink">
      {text.trim() ? (
        <Pictures.Provider value={false}>
          <Markdown text={text} className="prose-note" />
        </Pictures.Provider>
      ) : (
        <span className="text-ink-4">No text</span>
      )}
    </div>
  );
}

/** What fctl told the agent, word for word, a click away. */
function Told({ text, findKey }: { text: string; findKey?: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  // Finding in the transcript looks in it, and opens it for a match.
  useRevealed(findKey && `told:${findKey}`, () => setOpen(true));
  return (
    <div className="mt-2">
      <MilestoneButton
        icon={<LinearIcon className="size-3.5" />}
        open={open}
        onToggle={() => setOpen(!open)}
        controls={id}
        more
      >
        What fctl told the agent
      </MilestoneButton>
      {open && (
        <Thread id={id} className="mt-1.5 animate-rise">
          <div
            data-find={findKey && `told:${findKey}`}
            className="rounded-lg border border-line bg-sunken/50 px-3.5 py-2.5 font-mono text-[11.75px] leading-relaxed whitespace-pre-wrap text-ink-3 wrap-anywhere"
          >
            {text.trim()}
          </div>
        </Thread>
      )}
    </div>
  );
}

const NOTICE_VERB: Record<string, string> = {
  ticket_replied: "Replied on",
  ticket_commented: "Commented on",
  ticket_created: "Filed",
  ticket_updated: "Changed",
  ticket_moved: "Moved",
};

/**
 * What fctl did in the tracker for the agent: a line with the ticket, which opens into what it
 * posted (a reply, a comment, a new ticket's description).
 */
export const TicketNoticeView = memo(function TicketNoticeView({
  notice,
  code,
}: {
  notice: G.TicketNotice;
  code: string;
}) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const t = notice.ticket;
  const body = notice.body?.trim() ?? "";
  const where = notice.tracker === "linear" ? "in Linear" : "on GitHub";
  const state = notice.state;
  const line = (
    <>
      {NOTICE_VERB[code] ?? "Worked on"} <span className="font-mono text-[12px]">{t.key}</span>
      {code === "ticket_created" && t.title ? `: ${t.title}` : ""}
      {code === "ticket_moved" && state ? (
        <>
          {" "}
          to <span className={stateTone(state.kind)}>{state.name}</span>
        </>
      ) : (
        ""
      )}{" "}
      {where}
    </>
  );
  return (
    <div className="animate-rise py-1.5">
      {body ? (
        <MilestoneButton
          icon={<TrackerIcon tracker={notice.tracker} className="size-3.5" />}
          open={open}
          onToggle={() => setOpen(!open)}
          controls={id}
          more
          className="text-ink-3"
        >
          {line}
        </MilestoneButton>
      ) : (
        <div className="flex items-center gap-2.5 text-[12.5px] text-ink-3">
          <RailIcon>
            <TrackerIcon tracker={notice.tracker} className="size-3.5" />
          </RailIcon>
          <span className="min-w-0">{line}</span>
          {code === "ticket_updated" && state && (
            <span className={cn("inline-flex items-center gap-1 text-[12px]", stateTone(state.kind))}>
              <StateDot kind={state.kind} />
              {state.name}
            </span>
          )}
          <a
            href={notice.url ?? t.url}
            target="_blank"
            rel="noreferrer"
            aria-label={`Open ${t.key} ${where}`}
            className="text-ink-4 transition-colors hover:text-ink"
          >
            <ArrowUpRight className="size-3.5" />
          </a>
        </div>
      )}
      {open && body && (
        <Thread id={id} className="mt-1.5 animate-rise">
          <div className="rounded-lg border border-line bg-raised/50 px-3.5 py-2.5 text-[13.5px] text-ink-2">
            <Pictures.Provider value={false}>
              <Markdown text={body} className="prose-note" />
            </Pictures.Provider>
            <a
              href={notice.url ?? t.url}
              target="_blank"
              rel="noreferrer"
              className="mt-2 inline-flex items-center gap-1 text-[12px] text-ink-3 transition-colors hover:text-ink"
            >
              Open {where} <ArrowUpRight className="size-3" />
            </a>
          </div>
        </Thread>
      )}
    </div>
  );
});

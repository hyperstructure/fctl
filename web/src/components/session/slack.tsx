// What came from Slack and what went there, as a session shows it: where in Slack it started (a
// channel, a private channel or someone's direct messages with fctl), as a link to its thread; a
// message that reached its agent from there, as Slack shows who wrote what where, with what fctl told
// the agent a click away; and what fctl did in the thread for the agent, as lines that open into what
// it posted.

import { ArrowUpRight, Lock, MessagesSquare } from "lucide-react";
import { memo, useId, useState } from "react";
import { Avatar, SlackIcon } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import {
  contextWords,
  messageDid,
  noticeLine,
  placeName,
  sessionPlace,
  sessionPlaceLabel,
} from "@/lib/slack";
import { useRevealed } from "./find";
import { When } from "./follow-up";
import { MilestoneButton, RailIcon, Thread } from "./lines";

/**
 * A link into Slack, where it is one: an address on slack.com, over https. Anything else isn't shown
 * as a link: events come from the session's sandbox too, where anything can pose as fctl.
 */
export function slackHref(url: string | undefined | null): string | undefined {
  if (!url) return undefined;
  try {
    const u = new URL(url);
    const host = u.hostname.toLowerCase();
    return u.protocol === "https:" && (host === "slack.com" || host.endsWith(".slack.com"))
      ? u.href
      : undefined;
  } catch {
    return undefined;
  }
}

/** Where in Slack a session started, as a link to its thread there: Slack's mark and "#web-team". */
export function SlackPlaceLink({
  session,
  className,
}: {
  session: Pick<G.Session, "slack">;
  className?: string;
}) {
  const s = session.slack;
  if (!s) return null;
  const href = slackHref(s.url);
  const content = (
    <>
      <SlackIcon className="size-3 shrink-0" />
      {s.place === "private" && <Lock aria-hidden="true" className="size-3 shrink-0 text-ink-4" />}
      <span className="truncate">{sessionPlace(s)}</span>
    </>
  );
  const cls = cn("inline-flex min-w-0 items-center gap-1 text-[12px] text-ink-3", className);
  if (!href)
    return (
      <span title={sessionPlaceLabel(s)} className={cls}>
        {content}
      </span>
    );
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      title={`${sessionPlaceLabel(s)}: open its thread in Slack`}
      aria-label={`${sessionPlaceLabel(s)}: open its thread in Slack`}
      className={cn(cls, "transition-colors hover:text-ink")}
    >
      {content}
    </a>
  );
}

/** That a session started in Slack, in a list: where, a link over the session's own. */
export function SlackMark({ s }: { s: Pick<G.Session, "slack"> }) {
  if (!s.slack) return null;
  return <SlackPlaceLink session={s} className="relative z-[2] max-w-[9rem] shrink-0" />;
}

/** A message that reached the agent from Slack: who wrote what where, as Slack shows it. */
export const SlackMessageView = memo(function SlackMessageView({
  message: m,
  author,
  text,
  ts,
  findKey,
}: {
  message: G.SlackMessage;
  /** Whom fctl knows them as. */
  author?: G.UserRef;
  /** What fctl told the agent. */
  text: string;
  ts: number;
  /** The message's key in the transcript, which finding in it knows it by. */
  findKey?: string;
}) {
  const name = author?.name ?? m.by?.name ?? "Someone in Slack";
  const href = slackHref(m.url);
  const context = contextWords(m.context);
  const body = m.body?.trim() ?? "";
  // Their Slack name, where fctl knows them by another.
  const slackName = m.by && author && m.by.name !== author.name ? m.by.name : undefined;
  return (
    <div className="group/msg animate-rise py-3">
      <div className="mb-1.5 flex items-start gap-2.5 text-[12.5px]">
        <RailIcon>
          <Avatar
            user={author ?? { id: m.by?.user ?? name, name, avatar_url: m.by?.avatar_url ?? undefined }}
            size={20}
          />
        </RailIcon>
        <div className="flex min-h-6 min-w-0 flex-wrap items-center gap-x-1.5 gap-y-0.5">
          <span className="font-medium text-ink">{name}</span>
          {slackName && <span className="text-ink-4">({slackName} in Slack)</span>}
          <span className="text-ink-4">{messageDid(m)}</span>
          <When at={ts} />
        </div>
      </div>
      <div className="ml-[34px] overflow-hidden rounded-xl rounded-tl-sm border border-line bg-raised/50 wrap-anywhere">
        <SlackStrip m={m} href={href} />
        {m.dropped && !body ? (
          <p className="px-4 py-2.5 text-[13.5px] text-ink-4 italic">
            What it said was dropped once the session ended, as its workspace keeps no Slack messages in
            transcripts.
          </p>
        ) : (
          <div className="px-4 py-2.5 text-[14.5px] leading-[1.6] whitespace-pre-wrap text-ink">
            {body || <span className="text-ink-4">No text</span>}
          </div>
        )}
      </div>
      {(context || m.quoted) && (
        <p className="mt-1.5 ml-[34px] flex items-center gap-1.5 text-[12px] text-ink-4">
          <MessagesSquare aria-hidden="true" className="size-3.5 shrink-0" />
          <span className="min-w-0">
            {m.quoted && `With a message ${m.quoted.name} wrote`}
            {m.quoted && context && ", and "}
            {context && (m.quoted ? lowerFirst(context.replace(/^With /, "")) : context)}
          </span>
        </p>
      )}
      <Told text={text} findKey={findKey} />
    </div>
  );
});

const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

/** The strip atop a Slack message: where it was, as a link to it in Slack. */
function SlackStrip({ m, href }: { m: G.SlackMessage; href?: string }) {
  const where = m.place === "dm" ? "Direct message" : placeName(m);
  const content = (
    <>
      <SlackIcon className="size-3 shrink-0" />
      {m.place === "private" && <Lock aria-hidden="true" className="size-3 shrink-0" />}
      <span className="truncate">
        {where}
        {m.team_name ? ` · ${m.team_name}` : ""}
      </span>
      {href && <ArrowUpRight aria-hidden="true" className="ml-auto size-3 shrink-0 text-ink-4" />}
    </>
  );
  const cls =
    "flex min-w-0 items-center gap-1.5 border-b border-line bg-sunken/60 px-4 py-1.5 text-[11.5px] text-ink-3";
  return href ? (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      aria-label={`Open it in Slack, in ${where}`}
      className={cn(cls, "transition-colors hover:text-ink")}
    >
      {content}
    </a>
  ) : (
    <div className={cls}>{content}</div>
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
        icon={<SlackIcon mono className="size-3.5" />}
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

/**
 * What fctl did in the session's Slack thread for the agent: a line ("Replied in #web-team"), which
 * opens into what it posted, with the way to it in Slack. A failure says why, in its own words.
 */
export const SlackNoticeView = memo(function SlackNoticeView({
  notice: n,
  code,
  message,
}: {
  notice: G.SlackNotice;
  code: string;
  /** The notice's own words: why fctl couldn't, for a failure. */
  message: string;
}) {
  const [open, setOpen] = useState(false);
  const id = useId();
  const body = n.body?.trim() ?? "";
  const href = slackHref(n.url);
  const failed = code === "slack_failed";
  const line = noticeLine(code, n);
  if (failed)
    return (
      <div className="flex animate-rise items-start gap-2.5 py-1.5 text-[12.5px] text-danger">
        <RailIcon>
          <SlackIcon mono className="size-3.5" />
        </RailIcon>
        {/* The control plane's own sentence says what went wrong, and where. */}
        <span className="min-w-0 pt-[3px] [overflow-wrap:anywhere]">{message.trim() || line}</span>
      </div>
    );
  return (
    <div className="animate-rise py-1.5">
      {body ? (
        <MilestoneButton
          icon={<SlackIcon mono className="size-3.5" />}
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
            <SlackIcon mono className="size-3.5" />
          </RailIcon>
          <span className="min-w-0">{line}</span>
          {href && (
            <a
              href={href}
              target="_blank"
              rel="noreferrer"
              aria-label={`${line}: open it in Slack`}
              className="text-ink-4 transition-colors hover:text-ink"
            >
              <ArrowUpRight className="size-3.5" />
            </a>
          )}
        </div>
      )}
      {open && body && (
        <Thread id={id} className="mt-1.5 animate-rise">
          <div className="rounded-lg border border-line bg-raised/50 px-3.5 py-2.5 text-[13.5px] leading-relaxed whitespace-pre-wrap text-ink-2 wrap-anywhere">
            {body}
            {href && (
              <a
                href={href}
                target="_blank"
                rel="noreferrer"
                className="mt-2 flex w-fit items-center gap-1 text-[12px] whitespace-normal text-ink-3 transition-colors hover:text-ink"
              >
                Open in Slack <ArrowUpRight className="size-3" />
              </a>
            )}
          </div>
        </Thread>
      )}
    </div>
  );
});

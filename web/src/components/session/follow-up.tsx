// A message from fctl that brought the agent back to a pull request its session opened, as the
// transcript shows it: what happened on GitHub, as GitHub has it (people's comments with their
// avatars, failed checks, a merge), with what fctl told the agent a click away.

import { FileCode2 } from "lucide-react";
import { memo, useId, useState } from "react";
import { githubAvatarUrl } from "@/components/github-avatar";
import { Markdown, Pictures } from "@/components/markdown";
import { Avatar, ChecksIcon, GithubIcon, Logo, PullRequestIcon } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import {
  action,
  checksRest,
  type Happened,
  happenedText,
  lines,
  parts,
  pullRef,
  verdict,
  type Wrote,
} from "@/lib/follow-ups";
import { clock } from "@/lib/format";
import { githubLink } from "@/lib/pulls";
import { shortSha } from "@/lib/reviews";
import { MilestoneButton, RailIcon, Thread } from "./lines";

export const FollowUpMessage = memo(function FollowUpMessage({
  followUp,
  text,
  ts,
}: {
  followUp: G.FollowUp;
  /** What fctl told the agent. */
  text: string;
  ts: number;
}) {
  const { wrote, happened } = parts(followUp);
  return (
    <div className="group/msg animate-rise py-3">
      <div className="flex flex-col gap-3">
        {wrote.map((w) => (
          <Comment key={w.comment.id} f={followUp} w={w} ts={ts} />
        ))}
        {happened.map((e, i) => (
          <HappenedLine key={`${e.kind}-${i}`} f={followUp} e={e} ts={ts} />
        ))}
      </div>
      <Told text={text} />
    </div>
  );
});

/** The pull request, by its repository's name, as a link to `href` on GitHub. */
export function PullLink({ f, href }: { f: Pick<G.FollowUp, "repo" | "number" | "title">; href: string }) {
  const to = githubLink(href);
  if (!to) return <span className="font-mono text-[12px] text-ink-3">{pullRef(f)}</span>;
  return (
    <a
      href={to}
      target="_blank"
      rel="noreferrer"
      title={f.title ? `${f.repo.owner}/${f.repo.name}#${f.number}: ${f.title}` : undefined}
      className="inline-flex items-center gap-1 font-mono text-[12px] text-ink-3 underline-offset-2 transition-colors hover:text-ink hover:underline"
    >
      <GithubIcon className="size-3 shrink-0" />
      {pullRef(f)}
    </a>
  );
}

/**
 * When it happened, which shows as the pointer rests on the message: not on phones, where nothing
 * rests on it, and where it would take a line of its own.
 */
export function When({ at }: { at: number }) {
  return (
    <time
      dateTime={new Date(at).toISOString()}
      className="text-ink-4 opacity-0 transition-opacity group-hover/msg:opacity-100 max-sm:hidden"
    >
      {clock(at)}
    </time>
  );
}

/** Where on lines a comment is, as a strip atop its text, which opens it on GitHub. */
function Where({ c }: { c: G.FollowUpComment }) {
  return (
    <a
      href={githubLink(c.url)}
      target="_blank"
      rel="noreferrer"
      className="flex min-w-0 items-center gap-1.5 border-b border-line bg-sunken/60 px-4 py-1.5 font-mono text-[11.5px] text-ink-3 transition-colors hover:text-ink"
    >
      <FileCode2 className="size-3 shrink-0" />
      <span className="truncate">{lines(c)}</span>
      {c.deleted && <span className="shrink-0 font-sans text-ink-4">old version</span>}
    </a>
  );
}

/** What a person wrote, as GitHub's Markdown, its pictures as links. */
function Said({ body, className }: { body: string; className?: string }) {
  return (
    <div className={cn("px-4 py-2.5 leading-[1.6] text-ink", className)}>
      {body.trim() ? (
        <Pictures.Provider value={false}>
          <Markdown text={body} className="prose-note" />
        </Pictures.Provider>
      ) : (
        <span className="text-ink-4">No text</span>
      )}
    </div>
  );
}

/**
 * A person's comment, a review's text or a comment on lines: who wrote it and where, with what
 * they wrote; a review's comments on lines follow its text.
 */
function Comment({ f, w, ts }: { f: G.FollowUp; w: Wrote; ts: number }) {
  const c = w.comment;
  const said = verdict(c);
  return (
    <div>
      <div className="mb-1.5 flex items-start gap-2.5 text-[12.5px]">
        <RailIcon>
          <Avatar
            user={{ id: c.login, name: c.login, avatar_url: c.avatar_url ?? githubAvatarUrl(c.login, 40) }}
            size={20}
          />
        </RailIcon>
        <div className="flex min-h-6 min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5">
          <span className="font-medium text-ink">{c.login}</span>
          <span className="inline-flex items-center gap-1.5 text-ink-4">
            {action(c)} <PullLink f={f} href={c.url} />
          </span>
          {said && (
            <span
              className={cn(
                "rounded-full border px-1.5 py-px text-[11px] leading-4",
                said === "Approved"
                  ? "border-checks-passed/30 text-checks-passed"
                  : "border-line-strong text-ink-2",
              )}
            >
              {said}
            </span>
          )}
          <When at={c.at ?? ts} />
        </div>
      </div>
      <div className="ml-[34px] overflow-hidden rounded-xl rounded-tl-sm border border-line bg-raised/50 wrap-anywhere">
        {c.path && <Where c={c} />}
        <Said body={c.body} className="text-[14.5px]" />
        {w.lines.length > 0 && (
          <ul aria-label="Its comments on lines">
            {w.lines.map((l) => (
              <li key={l.id} className="border-t border-line">
                <Where c={l} />
                <Said body={l.body} className="text-[13.5px]" />
              </li>
            ))}
          </ul>
        )}
      </div>
    </div>
  );
}

/**
 * What fctl didn't pass on to the agent: failed checks it heard of three times in a row while nobody
 * messaged it. What happened shows muted, with why the agent didn't hear of it.
 */
export const HeldFollowUp = memo(function HeldFollowUp({
  followUp,
  ts,
}: {
  followUp: G.FollowUp;
  ts: number;
}) {
  const { happened } = parts(followUp);
  return (
    <div className="group/msg animate-rise py-3">
      <div className="flex flex-col gap-3">
        {happened.map((e, i) => (
          <HappenedLine key={`${e.kind}-${i}`} f={followUp} e={e} ts={ts} muted />
        ))}
      </div>
      <p className="mt-1 ml-[34px] text-[12.5px] leading-relaxed text-ink-4">
        Not passed on: the agent heard of failed checks three times in a row, with no message from a person in
        between. Message it to have it look.
      </p>
    </div>
  );
});

const PR_COLORS = { merged: "text-pr-merged", closed: "text-pr-closed" } as const;

/**
 * Failed checks, a merge or a close: a line of the transcript's own, with its pull request; `muted`
 * for what the agent didn't hear of.
 */
function HappenedLine({ f, e, ts, muted }: { f: G.FollowUp; e: Happened; ts: number; muted?: boolean }) {
  const icon =
    e.kind === "checks_failed" ? (
      <ChecksIcon state="failed" className={muted ? "text-ink-4" : "text-checks-failed"} />
    ) : (
      <PullRequestIcon state={e.kind} className={muted ? "text-ink-4" : PR_COLORS[e.kind]} />
    );
  const href = e.kind === "checks_failed" ? `${f.url}/checks` : f.url;
  return (
    <div className="flex items-start gap-2.5 text-[12.5px]">
      <RailIcon>{icon}</RailIcon>
      <div className="min-w-0 flex-1">
        <div className="flex min-h-6 flex-wrap items-center gap-x-2 gap-y-0.5">
          <span className={cn("font-medium", muted ? "text-ink-3" : "text-ink-2")}>{happenedText(f, e)}</span>
          <span className="inline-flex items-center gap-1.5 text-ink-4">
            {e.kind === "checks_failed" ? "on" : "·"} <PullLink f={f} href={href} />
            {e.kind === "checks_failed" && (
              <>
                at
                <a
                  href={githubLink(`${f.url}/commits/${e.sha}`)}
                  target="_blank"
                  rel="noreferrer"
                  className="font-mono text-[12px] text-ink-3 underline-offset-2 transition-colors hover:text-ink hover:underline"
                >
                  {shortSha(e.sha)}
                </a>
              </>
            )}
          </span>
          <When at={ts} />
        </div>
        {e.kind === "checks_failed" && (
          <div className="mt-1 flex flex-wrap items-center gap-1.5">
            {e.names.map((name) => (
              <span
                key={name}
                className={cn(
                  "max-w-full truncate rounded-md border px-1.5 py-px font-mono text-[11px]",
                  muted
                    ? "border-line bg-raised/50 text-ink-3"
                    : "border-checks-failed/25 bg-checks-failed/8 text-checks-failed",
                )}
              >
                {name}
              </span>
            ))}
            {checksRest(e) && <span className="text-[12px] text-ink-4">{checksRest(e)}</span>}
          </div>
        )}
      </div>
    </div>
  );
}

/** What fctl told the agent, word for word, a click away. */
export function Told({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <div className="mt-2">
      <MilestoneButton
        icon={<Logo withWord={false} className="[&_svg]:size-3.5" />}
        open={open}
        onToggle={() => setOpen(!open)}
        controls={id}
        more
      >
        What fctl told the agent
      </MilestoneButton>
      {open && (
        <Thread id={id} className="mt-1.5 animate-rise">
          <div className="rounded-lg border border-line bg-sunken/50 px-3.5 py-2.5 font-mono text-[11.75px] leading-relaxed whitespace-pre-wrap text-ink-3 wrap-anywhere">
            {text.trim()}
          </div>
        </Thread>
      )}
    </div>
  );
}

// A message from fctl that passed a review's findings on to the agent that opened the pull request, as
// the transcript shows it: what the review agent found, a line for each finding, which opens its
// comment on GitHub, with what fctl told the agent (each finding in full, and how to answer it) a
// click away.

import { Link } from "@tanstack/react-router";
import { memo } from "react";
import { ReviewIcon } from "@/components/ui";
import type * as G from "@/gen";
import { lines } from "@/lib/follow-ups";
import { issues, severityLabel, shortSha } from "@/lib/reviews";
import { PullLink, Told, When } from "./follow-up";
import { RailIcon } from "./lines";

export const FindingsMessage = memo(function FindingsMessage({
  review,
  text,
  ts,
  findKey,
}: {
  review: G.PassedOnReview;
  /** What fctl told the agent. */
  text: string;
  ts: number;
  /** The message's key in the transcript, which finding in it knows it by. */
  findKey?: string;
}) {
  // A finding's comment, else the review's summary, which GitHub shows above the comments.
  const where = review.summary_url ?? review.url;
  return (
    <div className="group/msg animate-rise py-3">
      <div className="flex items-start gap-2.5 text-[12.5px]">
        <RailIcon>
          <ReviewIcon kind="comment" className="text-ink-2" />
        </RailIcon>
        <div className="min-w-0 flex-1">
          <div className="flex min-h-6 flex-wrap items-center gap-x-2 gap-y-0.5">
            <span className="font-medium text-ink-2">
              <Link
                to="/s/$id"
                params={{ id: review.session }}
                className="underline-offset-2 transition-colors hover:text-ink hover:underline"
              >
                fctl's review agent
              </Link>{" "}
              found {issues(review.findings.length)}
            </span>
            <span className="inline-flex items-center gap-1.5 text-ink-4">
              in <PullLink f={review} href={where} />
              at
              <a
                href={`${review.url}/commits/${review.sha}`}
                target="_blank"
                rel="noreferrer"
                className="font-mono text-[12px] text-ink-3 underline-offset-2 transition-colors hover:text-ink hover:underline"
              >
                {shortSha(review.sha)}
              </a>
            </span>
            <When at={ts} />
          </div>
          <ul aria-label="Its findings" className="mt-1 flex flex-col gap-1 text-[13px] leading-5">
            {review.findings.map((f, i) => (
              <li key={`${i}-${f.title}`} className="flex min-w-0 flex-wrap items-baseline gap-x-2">
                <a
                  href={f.url ?? where}
                  target="_blank"
                  rel="noreferrer"
                  className="min-w-0 text-ink-2 underline-offset-2 transition-colors [overflow-wrap:anywhere] hover:text-ink hover:underline"
                >
                  {f.title}
                </a>
                <span className="min-w-0 text-[12px] text-ink-4 [overflow-wrap:anywhere]">
                  {severityLabel(f.severity)} · <span className="font-mono text-[11.5px]">{lines(f)}</span>
                  {f.deleted && " (old version)"}
                </span>
              </li>
            ))}
          </ul>
        </div>
      </div>
      <Told text={text} findKey={findKey} />
    </div>
  );
});

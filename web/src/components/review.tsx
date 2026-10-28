// A review agent's review, wherever it shows: its state as the icon GitHub's checks use, what fctl
// posted for it in the session's transcript, and on the session's page, what came of it.

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowUpRight } from "lucide-react";
import { ChecksIcon, EyeIcon, ReviewIcon } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { githubLink } from "@/lib/pulls";
import { keys } from "@/lib/queries";
import { earlierFindings, type ReviewLook, reviewLook, reviewOutcome, shortSha } from "@/lib/reviews";

/**
 * How a review stands, wherever it shows: an eye while it waits for a runner, reading as it
 * reviews, a check when it found nothing, a comment when it posted findings, skip when it ended
 * without posting, and an x when it failed.
 */
export function ReviewStateIcon({
  review,
  look = review && reviewLook(review),
  className,
}: {
  review?: Pick<G.Review, "state" | "status">;
  look?: ReviewLook;
  className?: string;
}) {
  const cls = cn("shrink-0", className);
  switch (look) {
    case "waiting":
      return <EyeIcon className={cn(cls, "text-ink-4")} />;
    case "reviewing":
      return <EyeIcon reading className={cn(cls, "text-accent")} />;
    case "passed":
      return <ChecksIcon state="passed" className={cn(cls, "text-checks-passed")} />;
    case "commented":
      return <ReviewIcon kind="comment" className={cn(cls, "text-blue")} />;
    case "failed":
      return <ChecksIcon state="failed" className={cn(cls, "text-danger")} />;
    default:
      return <ReviewIcon kind="skip" className={cn(cls, "text-ink-4")} />;
  }
}

/**
 * What fctl posted for the review agent, as the transcript shows it, with a link to it on GitHub,
 * and to the session whose agent opened the pull request when fctl sent it the findings.
 */
export function ReviewPostedNotice({ posted }: { posted: G.PostedReview }) {
  const found = posted.comments + posted.in_text;
  const resolved = posted.resolved?.length ?? 0;
  const facts = [
    resolved > 0 && `${resolved === 1 ? "1 earlier finding" : `${resolved} earlier findings`} marked fixed`,
    posted.in_text > 0 && `${posted.in_text} in the review's summary, on lines the diff doesn't show`,
    posted.still_open > 0 &&
      `${posted.still_open === 1 ? "1 earlier finding is" : `${posted.still_open} earlier findings are`} still open`,
  ].filter(Boolean);
  const url = githubLink(posted.url);
  return (
    <div className="my-2 flex animate-rise items-start gap-3 rounded-xl border border-line bg-raised/30 px-4 py-3">
      <ReviewStateIcon look={posted.state === "passed" ? "passed" : "commented"} className="mt-0.5 size-4" />
      <div className="min-w-0 flex-1 text-[13.5px]">
        <p className="text-ink">
          {found === 0
            ? "Review posted: nothing to point out"
            : `Review posted on GitHub: ${found === 1 ? "1 issue" : `${found} issues`}`}
        </p>
        {facts.length > 0 && <p className="mt-0.5 text-[12.5px] text-ink-3">{facts.join(" · ")}</p>}
        {posted.sent_to && (
          <p className="mt-0.5 text-[12.5px] text-ink-3">
            Sent to{" "}
            <Link
              to="/s/$id"
              params={{ id: posted.sent_to }}
              className="underline underline-offset-2 transition-colors hover:text-ink"
            >
              the agent that opened the pull request
            </Link>
          </p>
        )}
      </div>
      {url && (
        <a
          href={url}
          target="_blank"
          rel="noreferrer"
          className="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[12.5px] text-ink-2 transition-colors hover:bg-hover hover:text-ink"
        >
          View on GitHub <ArrowUpRight className="size-3.5" />
        </a>
      )}
    </div>
  );
}

/** What came of a review agent's session: its review, as the project's list has it. */
export function useSessionReview(session?: G.Session): G.Review | undefined {
  const id = session?.id ?? "";
  const project = session?.project_id ?? "";
  const q = useQuery({
    queryKey: keys.sessionReview(project, id),
    queryFn: () => api.reviews(project, { session: id }),
    enabled: !!session?.review,
  });
  return q.data?.reviews[0];
}

/** The review's link on GitHub, or its pull request's. */
function GithubLink({ href, children }: { href: string; children: React.ReactNode }) {
  return (
    <a
      href={href}
      target="_blank"
      rel="noreferrer"
      className="inline-flex shrink-0 items-center gap-1 rounded-md px-2 py-1 text-[12.5px] font-medium text-ink-2 transition-colors hover:bg-hover hover:text-ink"
    >
      {children} <ArrowUpRight className="size-3.5" />
    </a>
  );
}

/**
 * Where a session's composer is, for a review agent's: it takes no messages, so this says what it
 * does, and once it ended, what came of it, with its review on GitHub.
 */
export function ReviewFooter({ session, review }: { session: G.Session; review?: G.Review }) {
  const pr = session.review;
  if (!pr) return null;
  const ref = `${pr.repo.name}#${pr.number}`;
  const at = shortSha(pr.head_sha);
  const look = review ? reviewLook(review) : session.status === "queued" ? "waiting" : "reviewing";
  const text = (() => {
    switch (look) {
      case "waiting":
      case "reviewing":
        return `This review agent takes no messages: it reviews ${ref} at ${at} and posts what it finds there.`;
      case "passed":
        return `Reviewed ${ref} at ${at}: nothing to point out, and its check passed.`;
      case "commented":
        return review && review.findings > 0
          ? `Reviewed ${ref} at ${at} and posted ${review.findings === 1 ? "1 issue" : `${review.findings} issues`} there.`
          : `Reviewed ${ref} at ${at}: nothing new to point out.`;
      case "superseded":
        return review?.reason ?? "A newer commit came, and a review of it took this one's place.";
      default:
        return review?.reason ?? "This review ended without posting.";
    }
  })();
  return (
    <div className="flex items-center gap-3 rounded-2xl border border-line-strong bg-surface/80 py-2.5 pr-2.5 pl-4 backdrop-blur-sm">
      <ReviewStateIcon look={look} className="size-4" />
      <p className="min-w-0 flex-1 text-[13.5px] leading-snug text-ink-2 [overflow-wrap:anywhere]">{text}</p>
      {review?.review_url ? (
        <GithubLink href={review.review_url}>Review</GithubLink>
      ) : (
        <GithubLink href={pr.url}>{ref}</GithubLink>
      )}
    </div>
  );
}

/**
 * The review in the session's rail: how it went, the commit, the session whose agent opened the pull
 * request, and the project's other reviews.
 */
export function ReviewFacts({ session, review }: { session: G.Session; review?: G.Review }) {
  const pr = session.review;
  if (!pr) return null;
  const look = review ? reviewLook(review) : undefined;
  const earlier = review ? earlierFindings(review) : [];
  const opener = review?.opened_by;
  return (
    <div className="flex flex-col gap-2.5 text-[12.5px]">
      {review && look && (
        <div className="flex items-start gap-2">
          <ReviewStateIcon look={look} className="mt-px size-3.5" />
          <div className="min-w-0">
            <p className="font-medium text-ink">{reviewOutcome(review)}</p>
            {earlier.length > 0 && <p className="text-ink-3">{earlier.join(" · ")}</p>}
            {review.state !== "running" && review.reason && (
              <p className="mt-0.5 leading-relaxed text-ink-3 [overflow-wrap:anywhere]">{review.reason}</p>
            )}
          </div>
        </div>
      )}
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-ink-4">Commit</span>
        <a
          href={`${pr.url}/commits/${pr.head_sha}`}
          target="_blank"
          rel="noreferrer"
          className="font-mono text-[12px] text-ink-2 underline-offset-2 hover:text-ink hover:underline"
        >
          {shortSha(pr.head_sha)}
        </a>
      </div>
      {opener && (
        <div className="flex items-baseline justify-between gap-3">
          <span className="shrink-0 text-ink-4">Opened by</span>
          <Link
            to="/s/$id"
            params={{ id: opener.id }}
            title={`#${opener.number} ${opener.title}`}
            className="min-w-0 truncate text-ink-2 underline-offset-2 hover:text-ink hover:underline"
          >
            <span className="font-mono text-[12px] text-ink-3">#{opener.number}</span> {opener.title}
          </Link>
        </div>
      )}
      {review?.review_url && (
        <a
          href={review.review_url}
          target="_blank"
          rel="noreferrer"
          className="flex items-center justify-between rounded-md border border-line px-3 py-2 text-ink-2 transition-colors hover:border-line-strong hover:text-ink"
        >
          The review on GitHub <ArrowUpRight className="size-3.5" />
        </a>
      )}
      {review?.check_error && <p className="leading-relaxed text-amber">{review.check_error}</p>}
      <Link
        to="/p/$slug"
        params={{ slug: session.project_slug }}
        search={{ tab: "reviews" }}
        className="text-ink-3 transition-colors hover:text-ink"
      >
        All reviews in {session.project_slug} →
      </Link>
    </div>
  );
}

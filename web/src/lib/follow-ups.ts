// Follow-ups: what brings an agent back to a pull request its session opened, where its project
// turns them on. fctl passes on what happened there in a message from fctl: a person's comment
// that addresses the agent, failed checks, conflicts with its base branch, a merge. Here: how the
// transcript shows it, as GitHub has it, and how a project's settings say what brings its agents
// back.

import type * as G from "@/gen";

/** What else happened besides people's comments (and what a newer control plane says, which this app doesn't know). */
export type Happened = Exclude<G.FollowUpEvent, { kind: "comment" } | { kind: "other" }>;

/** Where a person wrote: on the pull request's conversation, a review's own text, or on lines of its diff. */
export type Place = "conversation" | "review" | "lines";

export function place(c: Pick<G.FollowUpComment, "review" | "path">): Place {
  if (c.review) return "review";
  if (c.path) return "lines";
  return "conversation";
}

/** A comment people wrote to the agent, and for a review, its comments on lines, which came with it. */
export interface Wrote {
  comment: G.FollowUpComment;
  lines: G.FollowUpComment[];
}

/**
 * A follow-up's parts as the transcript shows them: people's comments in the order they came, a
 * review's comments on lines under it, then what else happened.
 */
export function parts(f: Pick<G.FollowUp, "events">): { wrote: Wrote[]; happened: Happened[] } {
  const comments = f.events.flatMap((e) => (e.kind === "comment" ? [e as G.FollowUpComment] : []));
  const reviews = new Set(comments.filter((c) => c.review).map((c) => c.id));
  const under = (c: G.FollowUpComment) => !c.review && c.review_id !== undefined && reviews.has(c.review_id);
  const wrote = comments
    .filter((c) => !under(c))
    .map((comment) => ({
      comment,
      lines: comment.review ? comments.filter((c) => under(c) && c.review_id === comment.id) : [],
    }));
  const happened = f.events.filter((e): e is Happened => e.kind !== "comment" && e.kind !== "other");
  return { wrote, happened };
}

/** What a person did, after their name: "commented on", "reviewed", "replied on". */
export function action(c: Pick<G.FollowUpComment, "review" | "path" | "in_reply_to">): string {
  switch (place(c)) {
    case "review":
      return "reviewed";
    case "lines":
      return c.in_reply_to !== undefined ? "replied on" : "commented on";
    case "conversation":
      return "commented on";
  }
}

/** A review's verdict, as GitHub labels it; nothing for one that only commented. */
export function verdict(c: Pick<G.FollowUpComment, "review">): "Approved" | "Changes requested" | undefined {
  if (c.review === "approved") return "Approved";
  if (c.review === "changes_requested") return "Changes requested";
  return undefined;
}

/** Where a comment on lines is: `src/login.rs:42`, `src/login.rs:40–42`. */
export function lines(c: Pick<G.FollowUpComment, "path" | "line" | "start_line">): string {
  const path = c.path ?? "";
  if (c.line === undefined) return path;
  if (c.start_line !== undefined && c.start_line !== c.line) return `${path}:${c.start_line}–${c.line}`;
  return `${path}:${c.line}`;
}

/** `web#12`: the pull request by its repository's name. */
export const pullRef = (f: Pick<G.FollowUp, "repo" | "number">) => `${f.repo.name}#${f.number}`;

/** What happened, for a line of its own: "Merged into main by ada", "Conflicts with main". */
export function happenedText(f: Pick<G.FollowUp, "repo" | "number" | "base">, e: Happened): string {
  switch (e.kind) {
    case "checks_failed": {
      const n = e.names.length;
      return n > 1 ? `${n} checks failed` : "A check failed";
    }
    case "merged": {
      const into = e.base ?? f.base;
      return `Merged${into ? ` into ${into}` : ""}${e.by ? ` by ${e.by}` : ""}`;
    }
    case "closed":
      return `Closed without merging${e.by ? ` by ${e.by}` : ""}`;
    case "conflicts": {
      const into = e.base ?? f.base;
      return `Conflicts with ${into ?? "its base branch"}`;
    }
  }
}

/** The commit a line of what happened is about: failed checks' and conflicts'. */
export function happenedAt(e: Happened): string | undefined {
  return e.kind === "checks_failed" || e.kind === "conflicts" ? e.sha : undefined;
}

/** `a`, `a and b`, `a, b and c`. */
function listed(items: string[]): string {
  if (items.length < 2) return items.join("");
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

const capitalized = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** The checks still running and those that passed, after failed ones: "1 still running · 7 passed". */
export function checksRest(e: Extract<Happened, { kind: "checks_failed" }>): string {
  return [e.pending > 0 && `${e.pending} still running`, e.passed > 0 && `${e.passed} passed`]
    .filter(Boolean)
    .join(" · ");
}

/** What brings agents back besides comments, in words: those that are on. */
function others(f: Pick<G.FollowUps, "checks" | "conflicts" | "closed">): string[] {
  return [f.checks && "failed checks", f.conflicts && "merge conflicts", f.closed && "merges"].filter(
    (x): x is string => !!x,
  );
}

/** What brings a project's agents back to their pull requests, in a few words, for its settings. */
export function followUpsSummary(f: G.FollowUps): string {
  const on = [...(f.comments ? ["comments"] : []), ...others(f)];
  return on.length ? capitalized(listed(on)) : "Off";
}

/**
 * What a session's page says under its pull requests of what brings its agent back to them, or
 * nothing when its project brings it back for nothing. Settings from a control plane before
 * conflicts say nothing of them.
 */
export function followUpHint(f: G.FollowUps | undefined): string | undefined {
  if (!f) return undefined;
  const rest = others(f);
  if (f.comments) {
    const too = rest.length ? `; ${listed(rest)} do too` : "";
    return `Mention @fctl or @agent on GitHub to bring the agent back${too}.`;
  }
  if (rest.length === 0) return undefined;
  if (rest.length === 1 && f.closed) return "A merge brings the agent back.";
  return `${capitalized(listed(rest))} bring the agent back.`;
}

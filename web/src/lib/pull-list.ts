// The pull requests page's list: each pull request that sessions list, once, as the session that
// heard of it last has it, with every session that lists it; what each needs next; and how many
// are in each state. fctl_domain::pulls::listed is its twin for `fctl pulls`.

import type * as G from "@/gen";
import { checksMark, prFullRef, prReview, prReviewLook, prState, pullRequests } from "./pulls";

/**
 * The sessions the page and the sidebar's count look through, newest first: every one that lists a
 * pull request, archived ones and those of schedules that hide theirs too, since neither closes the
 * pull requests they opened. The page adds its workspace or project and Mine; unnarrowed, it asks
 * what the sidebar asks, once.
 */
export const PULLS_QUERY = { pulls: "any", limit: 200, archived: true, hidden: true } as const;

export interface ListedPull {
  pr: G.PullRequestRef;
  /** Newest first. */
  sessions: G.Session[];
}

/** Which tab a pull request is in. */
export type PullState = "open" | "merged" | "closed";
export const PULL_STATES: PullState[] = ["open", "merged", "closed"];

/** What an open pull request needs next, in the order the page shows them. */
export type Need = "asking" | "ready" | "failing" | "working" | "queued" | "draft";

export const NEEDS: { need: Need; title: string; hint: string }[] = [
  { need: "asking", title: "Waiting for your answer", hint: "Their agents asked something, and wait" },
  { need: "ready", title: "Ready for review", hint: "Their agents finished, and nothing failed" },
  { need: "failing", title: "Checks failing", hint: "What the agent pushed last didn't pass" },
  { need: "working", title: "Agents at work", hint: "Their sessions are still going" },
  { need: "queued", title: "In a merge queue", hint: "GitHub merges them in turn" },
  { need: "draft", title: "Drafts", hint: "Not ready for review yet" },
];

/** Each pull request `sessions` list (newest session first), once. */
export function listPulls(sessions: readonly G.Session[]): ListedPull[] {
  const byUrl = new Map<string, ListedPull>();
  for (const s of sessions)
    for (const pr of pullRequests(s)) {
      const l = byUrl.get(pr.url);
      if (!l) byUrl.set(pr.url, { pr, sessions: [s] });
      else {
        if ((pr.updated_at ?? 0) > (l.pr.updated_at ?? 0)) l.pr = pr;
        if (!l.sessions.some((x) => x.id === s.id)) l.sessions.push(s);
      }
    }
  return [...byUrl.values()].sort((a, b) => moved(b) - moved(a));
}

/** When a pull request last moved: GitHub's last change, else when it took its state. */
export const moved = (l: ListedPull) =>
  l.pr.updated_at ?? l.pr.state_changed_at ?? l.sessions[0]?.updated_at ?? 0;

/** Its tab: merged and closed, and open for the rest (drafts, merge queues, and those nobody looked up). */
export function stateOf(l: ListedPull): PullState {
  const s = prState(l.pr);
  return s === "merged" || s === "closed" ? s : "open";
}

const AT_WORK: G.SessionStatus[] = ["queued", "provisioning", "preparing", "running", "stopping"];

/** The newest of its sessions whose agent asked something (`fctl ask`) and waits for an answer. */
export const askingOf = (l: ListedPull): G.Session | undefined => l.sessions.find((s) => s.question);

/** What an open pull request needs next: an answer its agent waits for comes before the rest. */
export function needOf(l: ListedPull): Need {
  if (askingOf(l)) return "asking";
  const s = prState(l.pr);
  if (s === "queued") return "queued";
  if (s === "draft") return "draft";
  if (checksMark(l.pr) === "failed") return "failing";
  if (l.sessions.some((x) => AT_WORK.includes(x.status))) return "working";
  return "ready";
}

/** How many pull requests each tab has. */
export function countStates(list: readonly ListedPull[]): Record<PullState, number> {
  const n: Record<PullState, number> = { open: 0, merged: 0, closed: 0 };
  for (const l of list) n[stateOf(l)]++;
  return n;
}

/** The words of a filter, as `pullMatches` looks for them. */
export const filterWords = (query: string) => query.toLowerCase().split(/\s+/).filter(Boolean);

/**
 * Whether `l` has every word of `query` in its title, its reference as its row shows it
 * ("acme/shop#12", which "shop#12" and "#12" find too), its link, its sessions or what their
 * agents ask.
 */
export function pullMatches(l: ListedPull, query: string): boolean {
  const words = filterWords(query);
  if (!words.length) return true;
  const text = [
    l.pr.title ?? "",
    prFullRef(l.pr),
    l.pr.url,
    ...l.sessions.flatMap((s) => [
      s.title,
      `${s.project_slug}#${s.number}`,
      s.created_by.name,
      s.question?.text ?? "",
    ]),
  ]
    .join(" ")
    .toLowerCase();
  return words.every((w) => text.includes(w));
}

/** How much a pull request changes, for its reviewer. */
export interface PullSize {
  files: number;
  additions: number;
  deletions: number;
}

/**
 * Its size, from the newest session whose branch it is: one that lists it alone, since a session's
 * counts are those of all its repositories' changes. None where no session says.
 */
export function sizeOf(l: ListedPull): PullSize | undefined {
  const s = l.sessions.find((s) => pullRequests(s).length === 1 && s.stats.files_changed > 0);
  return s && { files: s.stats.files_changed, additions: s.stats.additions, deletions: s.stats.deletions };
}

/** Who started its sessions, each once, newest first. */
export function peopleOf(l: ListedPull): G.UserRef[] {
  const seen = new Map<string, G.UserRef>();
  for (const s of l.sessions)
    if (s.created_by && !seen.has(s.created_by.id)) seen.set(s.created_by.id, s.created_by);
  return [...seen.values()];
}

/**
 * What the page shows of where it stands, to tell when that changes as it watches: its tab, what it
 * needs, its checks and fctl's review.
 */
export function standing(l: ListedPull): string {
  const state = stateOf(l);
  const review = prReview(l.pr);
  return [
    state,
    state === "open" ? needOf(l) : "",
    checksMark(l.pr) ?? "",
    review ? prReviewLook(review) : "",
  ].join("|");
}

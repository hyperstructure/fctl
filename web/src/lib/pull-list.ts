// The pull requests page's list: each pull request that sessions list, once, as the session that
// heard of it last has it, with every session that lists it; what each needs next; and how many
// are in each state. fctl_domain::pulls::listed is its twin for `fctl pulls`.

import type * as G from "@/gen";
import { checksMark, prState, pullRequests } from "./pulls";

export interface ListedPull {
  pr: G.PullRequestRef;
  /** Newest first. */
  sessions: G.Session[];
}

/** Which tab a pull request is in. */
export type PullState = "open" | "merged" | "closed";
export const PULL_STATES: PullState[] = ["open", "merged", "closed"];

/** What an open pull request needs next, in the order the page shows them. */
export type Need = "ready" | "failing" | "working" | "queued" | "draft";

export const NEEDS: { need: Need; title: string; hint: string }[] = [
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

/** What an open pull request needs next. */
export function needOf(l: ListedPull): Need {
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

/** Whether `l` has every word of `query` in its title, repository, number or sessions. */
export function pullMatches(l: ListedPull, query: string): boolean {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return true;
  const repo = l.pr.repo ? `${l.pr.repo.owner}/${l.pr.repo.name}` : l.pr.url;
  const text = [
    l.pr.title ?? "",
    repo,
    `#${l.pr.number}`,
    ...l.sessions.flatMap((s) => [s.title, `${s.project_slug}#${s.number}`, s.created_by.name]),
  ]
    .join(" ")
    .toLowerCase();
  return words.every((w) => text.includes(w.replace(/^#/, "#")));
}

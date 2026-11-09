// What needs the person signed in, for the sidebar's "Needs you": in the sessions they started or
// follow, what each waits on them for, most pressing first. A session comes once, for the most
// pressing thing it needs.

import type * as G from "@/gen";
import { checksMark, prState, pullRequests } from "./pulls";
import { waitsForYou } from "./status";

export type NeedKind = "asking" | "failed" | "checks" | "ready" | "waiting" | "paused";

export interface Need {
  kind: NeedKind;
  s: G.Session;
  /** Since when it has needed them. */
  at?: number;
  /** The pull request it is about. */
  pr?: G.PullRequestRef;
}

const RANK: Record<NeedKind, number> = { asking: 0, failed: 1, checks: 2, ready: 3, waiting: 4, paused: 5 };
/** A failed session needs someone for a day; then it's history. */
export const FAILED_FOR = 24 * 3_600_000;

/**
 * What needs `me` in `sessions` (any lists, in any order; each session once, its newest summary):
 * a question its agent asks, a failure in the last day, a pull request whose checks failed, a pull
 * request its agent finished (it waits for a reply, or ended, and the pull request is open and
 * not a draft), a reply it waits for, a pause on a usage limit. A session whose pull requests are
 * all merged or closed, and that nobody asked for more since (an answer to its agent's question
 * doesn't), waits for no reply: its work is done, and its agent waits for a message until the
 * session stops only in case someone has more for it.
 * Nothing until it is known who `me` is: the lists hold other people's sessions too. An autopilot
 * that is on looks after the sessions it started (`piloted`: the ids of its home projects; while
 * they aren't known, every autopilot is taken to be on); those of one paused or off are anyone's.
 */
export function needsOf(
  me: string | undefined,
  sessions: readonly G.Session[],
  now: number,
  piloted?: ReadonlySet<string>,
): Need[] {
  if (!me) return [];
  const newest = new Map<string, G.Session>();
  for (const s of sessions) {
    const seen = newest.get(s.id);
    if (!seen || s.updated_at >= seen.updated_at) newest.set(s.id, s);
  }
  const out: Need[] = [];
  for (const s of newest.values()) {
    if (s.archived || s.review) continue;
    // An autopilot's sessions are its own to look after while it's on: it hears when they wait or
    // ask, and asks people itself (its tab, Needs you's own list). Those someone follows still count
    // for them, and those of an autopilot that is paused or off for whoever started them.
    if (s.autopilot && !s.following && (!piloted || piloted.has(s.autopilot.project_id))) continue;
    if (s.created_by?.id !== me && !s.following) continue;
    const need = needOf(s, now);
    if (need) out.push(need);
  }
  return out.sort((a, b) => RANK[a.kind] - RANK[b.kind] || (b.at ?? 0) - (a.at ?? 0));
}

function needOf(s: G.Session, now: number): Need | undefined {
  if (s.question) return { kind: "asking", s, at: s.question.asked_at };
  if (s.status === "failed")
    return (s.ended_at ?? 0) > now - FAILED_FOR ? { kind: "failed", s, at: s.ended_at } : undefined;
  const pulls = pullRequests(s);
  const open = pulls.filter((pr) => ["open", "unknown"].includes(prState(pr)));
  const failing = open.find((pr) => checksMark(pr) === "failed");
  if (failing) return { kind: "checks", s, at: failing.updated_at ?? s.last_activity_at, pr: failing };
  const done = s.status === "stopped" || waitsForYou(s);
  const ready = done ? open.find((pr) => prState(pr) === "open") : undefined;
  if (ready) return { kind: "ready", s, at: s.ended_at ?? s.last_activity_at, pr: ready };
  if (waitsForYou(s) && !finished(s, pulls)) return { kind: "waiting", s, at: s.last_activity_at };
  if (s.status === "paused") return { kind: "paused", s, at: s.last_activity_at };
  return undefined;
}

/**
 * Whether a session's work is done: it has pull requests, all merged or closed, and nobody has asked it for more
 * since the last of them was (`requested_at`). An answer to its agent's question doesn't ask for more: the agent
 * carries on with what it asked for (whether to re-run a failed deploy, say). One merged or closed at a time nobody
 * knows counts as before any request.
 */
function finished(s: G.Session, pulls: readonly G.PullRequestRef[]): boolean {
  if (!pulls.length || !pulls.every((pr) => ["merged", "closed"].includes(prState(pr)))) return false;
  if (s.requested_at === undefined) return true;
  const landed = pulls.map((pr) => pr.state_changed_at ?? Number.NEGATIVE_INFINITY);
  return Math.max(...landed) >= s.requested_at;
}

/** What a need says, after its session's title. */
export function needSays(n: Need): string {
  switch (n.kind) {
    case "asking":
      return n.s.question?.text ?? "Asks you";
    case "failed":
      return "Failed";
    case "checks":
      return `Checks failed on #${n.pr?.number ?? ""}`;
    case "ready":
      return `#${n.pr?.number ?? ""} is ready for you`;
    case "waiting":
      return "Waiting for your reply";
    case "paused":
      return "Paused on a usage limit";
  }
}

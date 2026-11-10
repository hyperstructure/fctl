// Moving a session to your Claude subscription. A session runs on the subscription of whoever
// started it, or last resumed it or took it over; a message keeps it there. Taking it over moves
// it to the Claude connection you chose for its workspace: one that waits for its limit to reset,
// or for a message, starts again there, and a stopped one resumes on it.

import type * as G from "@/gen";

/** Statuses a session can be taken over in: it waits or rests, rather than works. */
const MOVES = new Set<G.SessionStatus>(["paused", "idle", "stopping", "stopped"]);

export interface TakeOver {
  /** Whose subscription it runs on, when not yours. */
  whose?: string;
  /** It waits for its subscription's limit to reset. */
  waits: boolean;
}

/**
 * Whether you may move the session to your subscription now: you may message its agent, and you
 * have a Claude connection; it isn't a review agent's or an autopilot's own (the autopilot moves
 * those itself once taken over); it waits or rests; and it runs on someone else's subscription,
 * or on yours while you have more than one connection to move it to.
 */
export function takeOverOf(
  session: G.Session | undefined,
  role: G.SessionRole | undefined,
  me: Pick<G.Me, "user" | "claude"> | null | undefined,
): TakeOver | undefined {
  if (!session || !me || (role !== "editor" && role !== "owner")) return undefined;
  if (session.review || session.autopilot?.role === "pilot" || !MOVES.has(session.status)) return undefined;
  const connections = me.claude.filter((c) => c.status === "active").length;
  if (!connections) return undefined;
  const runsOn = session.claude ?? session.created_by;
  const mine = runsOn.id === me.user.id;
  if (mine && connections < 2) return undefined;
  return { whose: mine ? undefined : runsOn.name, waits: session.status === "paused" };
}

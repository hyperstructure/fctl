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
 * or on another of your connections than the one you chose for its workspace, which taking it
 * over moves it to. `mine`: the connection it runs on when it's yours (`Session.claude_connection`,
 * which only the session's own request has); `chosen`: the one you chose for its workspace
 * (`Workspace.claude`).
 */
export function takeOverOf(
  session: G.Session | undefined,
  role: G.SessionRole | undefined,
  me: Pick<G.Me, "user" | "claude"> | null | undefined,
  connections: { mine?: string; chosen?: string } = {},
): TakeOver | undefined {
  if (!session || !me || (role !== "editor" && role !== "owner")) return undefined;
  if (session.review || session.autopilot?.role === "pilot" || !MOVES.has(session.status)) return undefined;
  const active = me.claude.filter((c) => c.status === "active");
  if (!active.length) return undefined;
  const runsOn = session.claude ?? session.created_by;
  if (runsOn.id !== me.user.id) return { whose: runsOn.name, waits: session.status === "paused" };
  // Yours: only to move it to the connection its workspace's sessions use now, as the server
  // picks it, when that's another one.
  const now = workspaceConnection(active, connections.chosen);
  if (active.length < 2 || !now || !connections.mine || connections.mine === now) return undefined;
  return { waits: session.status === "paused" };
}

/**
 * The connection your sessions in a workspace run on, as the server picks it: the one you chose
 * for it, else your default one, else your newest.
 */
export function workspaceConnection(
  active: G.ClaudeCredentialInfo[],
  chosen: string | undefined,
): string | undefined {
  const pick =
    active.find((c) => c.id === chosen) ??
    active.find((c) => c.is_default) ??
    [...active].sort((a, b) => b.created_at - a.created_at)[0];
  return pick?.id;
}

// What a session's status says beyond its pill.

import type * as G from "@/gen";

type Status = Pick<G.Session, "status" | "status_reason">;

/** What fctl-agent says when an idle session waits for a message, and for
 * nothing else. */
const WAITING_FOR_A_MESSAGE = "Waiting for a message";

/** Whether a session waits for a person: it is idle, and waits for nothing
 * else, such as its agent's background work. */
export function waitsForYou(s: Status): boolean {
  return s.status === "idle" && (!s.status_reason || s.status_reason === WAITING_FOR_A_MESSAGE);
}

/** What sits beside a session's status pill: its reason, unless the pill
 * says it all (a session at work, or one waiting for a message). A session
 * that waits for its agent's background tasks says so. */
export function statusReason(s: Status): string | undefined {
  if (!s.status_reason || s.status === "running" || waitsForYou(s)) return undefined;
  return s.status_reason;
}

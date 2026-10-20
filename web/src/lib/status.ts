// What a session's status says beyond its pill.

import type * as G from "@/gen";
import { until } from "./format";
import { type Stall, stallLabel } from "./stall";

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

/** A few words beside a session's status pill; a warning shows in amber. */
export interface StatusNote {
  text: string;
  warn?: boolean;
}

/**
 * What a session's header says beside its status pill, the most pressing
 * first: the page reconnecting, the agent's work not coming through, when a
 * paused session resumes, then the status's reason. A phone has room for the
 * first.
 */
export function statusNotes(
  s: Status & Pick<G.Session, "resume_at">,
  live: { reconnecting: boolean; stall?: Stall },
  now = Date.now(),
): StatusNote[] {
  const notes: StatusNote[] = [];
  if (live.reconnecting) notes.push({ text: "Reconnecting…", warn: true });
  if (live.stall) notes.push({ text: stallLabel(live.stall, now), warn: true });
  if (s.resume_at) notes.push({ text: `resumes in ${until(s.resume_at, now)}`, warn: true });
  const reason = statusReason(s);
  if (reason) notes.push({ text: reason });
  return notes;
}

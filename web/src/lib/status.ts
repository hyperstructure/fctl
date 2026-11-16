// What a session's status says beyond its pill.

import type * as G from "@/gen";
import { until } from "./format";
import { type DiskShort, diskLabel, type Stall, stallLabel } from "./stall";

type Status = Pick<G.Session, "status" | "status_reason">;

/** What fctl-agent says when an idle session waits for a message, and for
 * nothing else: also after Claude Code was cut off twice in ten minutes
 * (`fctl_domain::autopilot::CUT_OFF_TWICE`), which the transcript tells. */
const WAITING_FOR_A_MESSAGE = new Set(["Waiting for a message", "Cut off twice: waiting for a message"]);

/** Whether an idle session's reason says it waits for a message alone, not
 * for its agent's background work. */
export function waitsForAMessage(reason?: string): boolean {
  return !reason || WAITING_FOR_A_MESSAGE.has(reason);
}

/** Whether a session waits for a person: it is idle, and waits for nothing
 * else, such as its agent's background work. */
export function waitsForYou(s: Status): boolean {
  return s.status === "idle" && waitsForAMessage(s.status_reason);
}

/** What sits beside a session's status pill: its reason, unless the pill
 * says it all (a session at work, or one waiting for a message). A session
 * that waits for its agent's background tasks says so. */
export function statusReason(s: Status): string | undefined {
  if (!s.status_reason || s.status === "running" || waitsForYou(s)) return undefined;
  return s.status_reason;
}

/**
 * A note as it reads beside a status pill that already says its first word: "Stopped after being
 * idle" beside "Stopped" is "after being idle", and "Stopping: studio is draining" beside "Stopping"
 * is "studio is draining". Without it the header read "Stopped · Stopped after being idle". A note
 * that doesn't start with the pill's word is as it is.
 */
export function besidePill(text: string, pill: string): string {
  if (!pill || text.slice(0, pill.length).toLowerCase() !== pill.toLowerCase()) return text;
  const rest = text.slice(pill.length);
  const gap = /^(?::\s+|\s+)/.exec(rest)?.[0];
  return gap && rest.length > gap.length ? rest.slice(gap.length) : text;
}

/** A few words beside a session's status pill; a warning shows in amber, danger in red. */
export interface StatusNote {
  text: string;
  warn?: boolean;
  danger?: boolean;
}

/**
 * What a session's header says beside its status pill, the most pressing
 * first: the page reconnecting, the agent's work not coming through, its
 * runner short of disk, when a paused session resumes, then the status's
 * reason. A phone has room for the first.
 */
export function statusNotes(
  s: Status & Pick<G.Session, "resume_at">,
  live: { reconnecting: boolean; stall?: Stall; disk?: DiskShort },
  now = Date.now(),
): StatusNote[] {
  const notes: StatusNote[] = [];
  if (live.reconnecting) notes.push({ text: "Reconnecting…", warn: true });
  if (live.stall) notes.push({ text: stallLabel(live.stall, now), warn: true });
  if (live.disk) notes.push({ text: diskLabel(live.disk), warn: true, danger: live.disk.room === "out" });
  if (s.resume_at) notes.push({ text: `resumes in ${until(s.resume_at, now)}`, warn: true });
  const reason = statusReason(s);
  if (reason) notes.push({ text: reason });
  return notes;
}

// Early access (docs/architecture.md#early-access). Where the instance takes
// requests, people without an account ask for it at /early-access; this
// browser remembers the address it asked with, so that the page says so when
// they come back. Admins invite them from Admin → Early access, and send the
// link from their own mail app when the instance sends no email.

import type * as G from "@/gen";

const SENT = "fctl:early-access";
/** The control plane forgets a request a year after it was made. */
const KEPT_MS = 365 * 86_400_000;

/** The request this browser sent. */
export interface Sent {
  email: string;
  /** When, in milliseconds. */
  at: number;
}

export function loadSent(now = Date.now()): Sent | undefined {
  try {
    const d: unknown = JSON.parse(localStorage.getItem(SENT) ?? "null");
    if (!d || typeof d !== "object") return undefined;
    const email = "email" in d && typeof d.email === "string" ? d.email : "";
    const at = "at" in d && typeof d.at === "number" ? d.at : 0;
    return email && at > now - KEPT_MS ? { email, at } : undefined;
  } catch {
    return undefined;
  }
}

/** Remembers the request, or forgets it (`undefined`). */
export function saveSent(s: Sent | undefined) {
  try {
    if (s) localStorage.setItem(SENT, JSON.stringify(s));
    else localStorage.removeItem(SENT);
  } catch {
    // Private browsing without storage: the page says it until it closes.
  }
}

/** Where a request stands: it waits for an invitation, or has one that works, or one that stopped working. */
export type RequestState = "waiting" | "invited" | "expired";

export function requestState(
  r: Pick<G.AccessRequest, "invited_at" | "invite_expires_at">,
  now = Date.now(),
): RequestState {
  if (r.invited_at === undefined || r.invited_at === null) return "waiting";
  return r.invite_expires_at && r.invite_expires_at > now ? "invited" : "expired";
}

/**
 * The waiting requests, those waiting longest first, and the invited ones, latest first:
 * the order the list comes in, kept as rows change state while the page is open.
 */
export function ordered(requests: readonly G.AccessRequest[]): {
  waiting: G.AccessRequest[];
  invited: G.AccessRequest[];
} {
  const waiting = requests.filter((r) => requestState(r) === "waiting");
  const invited = requests.filter((r) => requestState(r) !== "waiting");
  waiting.sort((a, b) => a.created_at - b.created_at);
  invited.sort((a, b) => (b.invited_at ?? 0) - (a.invited_at ?? 0));
  return { waiting, invited };
}

/**
 * An email with someone's invitation, for the admin's own mail app: a `mailto:` link with
 * their address, a subject and the link. RFC 6068 wants line breaks as CRLF, and the
 * address's `@` as it is.
 */
export function invitationMail({
  email,
  url,
  instance,
  expiresAt,
}: {
  email: string;
  url: string;
  instance: string;
  expiresAt: number;
}): string {
  const until = new Date(expiresAt).toLocaleDateString(undefined, { day: "numeric", month: "long" });
  const subject = `Your early access to ${instance}`;
  const body = [
    `You asked for early access to ${instance}, and it's ready. This link sets up your account:`,
    "",
    url,
    "",
    `It works once, until ${until}.`,
  ].join("\r\n");
  const to = encodeURIComponent(email).replace(/%40/g, "@");
  return `mailto:${to}?subject=${encodeURIComponent(subject)}&body=${encodeURIComponent(body)}`;
}

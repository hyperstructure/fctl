// Remote Control: following a session in the Claude apps.

import type * as G from "@/gen";

const SESSION_URL = /^https:\/\/claude\.ai\/code\/((?:session|cse)_[A-Za-z0-9_-]+)$/;

/**
 * The `claude://` link that opens a Remote Control session in Claude's
 * desktop or mobile app, from the session's claude.ai/code URL. Undefined
 * for any other URL.
 */
export function claudeAppUrl(sessionUrl: string): string | undefined {
  const id = SESSION_URL.exec(sessionUrl)?.[1];
  return id ? `claude://code/${id}` : undefined;
}

/**
 * Props for a link that opens the session in the Claude app. A `claude://`
 * link hands off to the app and leaves this page as it is; a URL without one
 * opens in a new tab.
 */
export function claudeAppLink(sessionUrl: string): { href: string; target?: string; rel?: string } {
  const app = claudeAppUrl(sessionUrl);
  return app ? { href: app } : { href: sessionUrl, target: "_blank", rel: "noreferrer" };
}

/**
 * The person on whose Claude subscription a session's Remote Control state
 * `rc` came about: who started the session, or who resumed it. Only they can
 * open it in the Claude apps. Before the session has any, and from control
 * planes before they said, it is who started the session.
 */
export function remoteControlUser(rc: G.RemoteControlState | undefined, startedBy: G.UserRef): G.UserRef {
  return rc?.user ?? startedBy;
}

/**
 * The claude.ai/code URL of a session's Remote Control state `rc` for `me`:
 * none unless it is theirs to open (see {@link remoteControlUser}).
 */
export function remoteControlUrl(
  rc: G.RemoteControlState | undefined,
  startedBy: G.UserRef,
  me: G.UserId | undefined,
): string | undefined {
  return me !== undefined && remoteControlUser(rc, startedBy).id === me ? rc?.session_url : undefined;
}

// Remote Control: following a session in the Claude apps.

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

// Paths of the app that people are sent on to, as after a sign-in (`?next=`).

/**
 * `next` when it is a path of the app, else the home page. Browsers read `\` as `/` and drop tabs and newlines,
 * so `/\evil.example` and `/<tab>/evil.example` would leave the app (as `fctl_domain::validate::is_app_path`).
 */
export function appPath(next?: string): string {
  return next && /^\/(?!\/)/.test(next) && !/[\\\s\p{Cc}]/u.test(next) ? next : "/";
}

/** Pages that sign someone in, approve a sign-in or hand an address or account over. */
const SIGN_IN_PAGES = /^\/(?:device|invite|reset|setup|two-factor|login|early-access)(?:[/?#]|$)/i;

/**
 * Where a link in text someone else wrote (an agent's answer, a note) may go: a web page, in a new tab; an email
 * address; or a page of the app, or a place on this one. Nothing for the rest, which shows as text: other schemes,
 * paths a browser reads as another site (`//evil.example`, `/\evil.example`), and the app's sign-in pages, where a
 * link could have someone approve an attacker's `fctl login` or connect an address.
 */
export function textLink(href: string): { href: string; kind: "web" | "mail" | "app" } | undefined {
  if (/^https?:/i.test(href)) return { href, kind: "web" };
  if (/^mailto:/i.test(href)) return { href, kind: "mail" };
  if (href.startsWith("#")) return { href, kind: "app" };
  if (appPath(href) === href && !SIGN_IN_PAGES.test(href)) return { href, kind: "app" };
  return undefined;
}

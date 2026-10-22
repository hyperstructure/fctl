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

/** The API's paths, which start sign-ins with Google and GitHub, except published screenshots' links. */
const API = /^\/api(?:[/?#]|$)(?!v1\/published\/)/i;

/** Who `host` is, however it is spelled: `App.Example.com.` is `app.example.com`. */
const hostOf = (host: string) => host.toLowerCase().replace(/\.$/, "");

/**
 * Where a link in text someone else wrote (an agent's answer, a note) may go: a web page, in a new tab; an email
 * address; or a page of the app, or a place on this one. Nothing for the rest, which shows as text: other schemes,
 * paths a browser reads as another site (`//evil.example`, `/\evil.example`), and the app's sign-in pages and API,
 * however a link names them (`/x/../device`, `/%64evice`, `https://<this app>/device`): a link there could have
 * someone approve an attacker's `fctl login`, connect an address, or start a GitHub sign-in. `origin` is the app's.
 */
export function textLink(
  href: string,
  origin: string = globalThis.location?.origin ?? "http://localhost",
): { href: string; kind: "web" | "mail" | "app" } | undefined {
  if (/^mailto:/i.test(href)) return { href, kind: "mail" };
  if (href.startsWith("#")) return { href, kind: "app" };
  const web = /^https?:/i.test(href);
  if (!web && appPath(href) !== href) return undefined;
  let url: URL;
  let app: URL;
  try {
    app = new URL(origin);
    url = new URL(href, app);
  } catch {
    return undefined;
  }
  // As the browser goes there: dot segments resolved, and letters as the router reads them.
  let path = url.pathname;
  try {
    path = decodeURIComponent(path);
  } catch {}
  if (hostOf(url.host) === hostOf(app.host) && (SIGN_IN_PAGES.test(path) || API.test(path))) return undefined;
  if (web) return { href, kind: "web" };
  return { href: url.pathname + url.search + url.hash, kind: "app" };
}

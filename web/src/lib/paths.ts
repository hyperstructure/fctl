// Paths of the app that people are sent on to, as after a sign-in (`?next=`).

/**
 * `next` when it is a path of the app, else the home page. Browsers read `\` as `/` and drop tabs and newlines,
 * so `/\evil.example` and `/<tab>/evil.example` would leave the app (as `fctl_domain::validate::is_app_path`).
 */
export function appPath(next?: string): string {
  return next && /^\/(?!\/)/.test(next) && !/[\\\s\p{Cc}]/u.test(next) ? next : "/";
}

// "Did you mean …?" on the not-found page: the app's page that an address
// which doesn't exist most likely meant.

/** The app's pages with fixed addresses (router.tsx), besides the floor. */
const PAGES = ["/new", "/settings", "/admin", "/device", "/login"];

/** Words people use for the floor, the list of sessions and projects. */
const FLOOR = new Set(["sessions", "projects", "floor", "home", "dashboard", "index", "index.html"]);
const LOGIN = new Set(["signin", "sign-in", "log-in", "logon", "auth"]);

/** Levenshtein distance. */
export function distance(a: string, b: string): number {
  if (a === b) return 0;
  let prev = Array.from({ length: b.length + 1 }, (_, j) => j);
  for (let i = 1; i <= a.length; i++) {
    const row = [i];
    for (let j = 1; j <= b.length; j++) {
      const cost = a[i - 1] === b[j - 1] ? 0 : 1;
      row[j] = Math.min((prev[j] ?? 0) + 1, (row[j - 1] ?? 0) + 1, (prev[j - 1] ?? 0) + cost);
    }
    prev = row;
  }
  return prev[b.length] ?? 0;
}

/** The address of the page `path` probably meant, or null. */
export function suggest(path: string): string | null {
  let decoded = path;
  try {
    decoded = decodeURIComponent(path);
  } catch {}
  // A session id anywhere in it: that session.
  const session = decoded.match(/\bses_[0-9A-Za-z]{20,}/)?.[0];
  if (session) return `/s/${session}`;
  const parts = decoded.split(/[?#]/)[0]?.split("/").filter(Boolean) ?? [];
  const [first, second] = parts;
  if (!first) return null;
  const head = first.toLowerCase();
  if ((head === "session" || head === "sessions") && second) return `/s/${second}`;
  if ((head === "project" || head === "projects") && second) return `/p/${second}`;
  // A project or session page with more after it.
  if ((head === "p" || head === "s") && second) return `/${head}/${second}`;
  if (FLOOR.has(head)) return "/";
  if (LOGIN.has(head)) return "/login";
  let best: string | null = null;
  let score = Number.POSITIVE_INFINITY;
  for (const page of PAGES) {
    const d = distance(`/${head}`, page);
    if (d < score) {
      score = d;
      best = page;
    }
  }
  return score <= 2 ? best : null;
}

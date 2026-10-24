// The GitHub Apps whose pull requests start reviews by themselves in a project
// (`ReviewAgents.bots`): how people name them, how the Reviews tab shows them, and which
// it offers to add.

import type * as G from "@/gen";
import { distance } from "@/lib/suggest";

/**
 * An app the Reviews tab knows without asking GitHub: its bot, its name, what it opens pull
 * requests for, and its id on GitHub, which its logo goes by.
 */
export interface KnownBot {
  login: string;
  name: string;
  about: string;
  app: number;
}

/** Apps that open pull requests in many repositories, the most used first. */
export const KNOWN_BOTS: readonly KnownBot[] = [
  { login: "dependabot[bot]", name: "Dependabot", about: "Dependency and security updates", app: 29110 },
  { login: "renovate[bot]", name: "Renovate", about: "Dependency updates, by Mend", app: 2740 },
  {
    login: "github-actions[bot]",
    name: "GitHub Actions",
    about: "Pull requests the repository's workflows open",
    app: 15368,
  },
  { login: "copilot-swe-agent[bot]", name: "Copilot", about: "GitHub Copilot's coding agent", app: 1143301 },
  { login: "pre-commit-ci[bot]", name: "pre-commit.ci", about: "Hook updates and fixes", app: 68672 },
  { login: "mergify[bot]", name: "Mergify", about: "Backports and merge queues", app: 10562 },
  { login: "snyk-io[bot]", name: "Snyk", about: "Fixes for vulnerable dependencies", app: 372950 },
  { login: "depfu[bot]", name: "Depfu", about: "Dependency updates", app: 715 },
  {
    login: "deepsource-autofix[bot]",
    name: "DeepSource Autofix",
    about: "Fixes for what DeepSource finds",
    app: 57168,
  },
  { login: "imgbot[bot]", name: "Imgbot", about: "Smaller images", app: 4706 },
  {
    login: "allcontributors[bot]",
    name: "All Contributors",
    about: "Credit for contributors in the README",
    app: 23186,
  },
];

/**
 * The bot of the GitHub App someone names, as pull requests name their author: "renovate",
 * "Renovate[bot]", "@renovate[bot]" and "https://github.com/apps/renovate" are all
 * "renovate[bot]". Null for what can't be an app's slug, such as a person's page. As the control
 * plane reads them (fctl_domain::reviews::bot_login).
 */
export function botLogin(input: string): string | null {
  const s = input.trim().replace(/^@+/, "").toLowerCase();
  const page = s.replace(/^https?:\/\//, "").replace(/^www\./, "");
  const name = page.startsWith("github.com/apps/")
    ? (page.slice("github.com/apps/".length).split(/[/?#]/)[0] ?? "")
    : s;
  const slug = name.endsWith("[bot]") ? name.slice(0, -"[bot]".length) : name;
  return /^[a-z0-9_][a-z0-9_-]{0,63}$/.test(slug) ? `${slug}[bot]` : null;
}

/** "renovate" for "renovate[bot]": the app's slug, as its page on GitHub has it. */
export const botSlug = (login: string) => login.replace(/\[bot\]$/i, "");

export const knownBot = (login: string): KnownBot | undefined =>
  KNOWN_BOTS.find((b) => b.login === login.toLowerCase());

/** `url` asking for an image `size` pixels square, as avatars.githubusercontent.com takes it. */
export function sized(url: string, size: number): string {
  try {
    const u = new URL(url);
    u.searchParams.set("s", String(size));
    return u.toString();
  } catch {
    return url;
  }
}

/** An app as the Reviews tab shows it. */
export interface BotView {
  login: string;
  /** Its name: as GitHub or the Reviews tab knows it, or else its slug. */
  name: string;
  /** Its logo; absent when nobody said where it is. */
  logo?: string;
  /** What it opens pull requests for, for apps the Reviews tab knows. */
  about?: string;
  /** Who makes it, as GitHub says. */
  owner?: string;
  /** Its page on GitHub. */
  url: string;
  /** What it did in the project's repositories lately. */
  seen?: G.ReviewBot;
}

/**
 * How the Reviews tab shows the app whose bot is `login`, from what it knows of it and what GitHub
 * said: the apps that opened the project's newest pull requests (`seen`), and an app looked up by
 * name (`found`).
 */
export function botView(login: string, seen?: G.ReviewBot, found?: G.ReviewBot): BotView {
  const known = knownBot(login);
  const logo = found?.avatar_url ?? seen?.avatar_url;
  return {
    login,
    name: found?.name ?? known?.name ?? botSlug(login),
    logo: logo ?? (known ? `https://avatars.githubusercontent.com/in/${known.app}` : undefined),
    about: known?.about,
    owner: found?.owner,
    url: found?.url ?? seen?.url ?? `https://github.com/apps/${botSlug(login)}`,
    seen,
  };
}

/** Whether two lists hold the same apps, in whatever order. */
export function sameBots(a: readonly string[], b: readonly string[]): boolean {
  const x = new Set(a.map((l) => l.toLowerCase()));
  const y = new Set(b.map((l) => l.toLowerCase()));
  return x.size === y.size && [...x].every((l) => y.has(l));
}

/**
 * The apps that match what someone typed, by name or bot: those with a word that starts with it
 * first, then those that contain it, each in the order given.
 */
export function matchBots<T extends Pick<BotView, "login" | "name">>(
  query: string,
  views: readonly T[],
): T[] {
  const q = query.trim().toLowerCase().replace(/^@+/, "");
  if (!q) return [...views];
  const gaps = /[\s._-]+/g;
  const wanted = botSlug(q).replace(gaps, "");
  const score = (v: T) => {
    const names = [v.name.toLowerCase(), botSlug(v.login).toLowerCase()];
    const words = names.flatMap((n) => n.split(gaps));
    const whole = names.map((n) => n.replace(gaps, ""));
    if ([...whole, ...words].some((n) => n.startsWith(wanted))) return 2;
    if (whole.some((n) => n.includes(wanted))) return 1;
    return 0;
  };
  return views
    .map((v, i) => ({ v, i, s: score(v) }))
    .filter((m) => m.s > 0)
    .sort((a, b) => b.s - a.s || a.i - b.i)
    .map((m) => m.v);
}

/** The app among `logins` someone most likely meant by `login`, which GitHub doesn't know: a slip of two letters at most. */
export function meant(login: string, logins: readonly string[]): string | undefined {
  const slug = botSlug(login);
  if (slug.length < 4) return undefined;
  let best: string | undefined;
  let score = 3;
  for (const l of logins) {
    const d = distance(slug, botSlug(l));
    if (d > 0 && d < score) {
      score = d;
      best = l;
    }
  }
  return best;
}

/** "Dependabot", "Dependabot and Renovate", "Dependabot, Renovate and Mergify", "Dependabot, Renovate and 2 other apps". */
export function botNames(names: readonly string[]): string {
  if (names.length <= 2) return names.join(" and ");
  if (names.length === 3) return `${names[0]}, ${names[1]} and ${names[2]}`;
  return `${names[0]}, ${names[1]} and ${names.length - 2} other apps`;
}

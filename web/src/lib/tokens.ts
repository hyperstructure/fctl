// Access tokens limited to what they are for (G.TokenScope): what one does in fctl (reads, runs
// sessions, or everything its person may), the projects it reaches, and what the agents of the
// sessions it starts may do on GitHub. Settings makes them and says what each does.

import type * as G from "@/gen";

export interface AccessLevel {
  value: G.TokenAccess;
  label: string;
  description: string;
}

/** What a token may do, the most common use first. */
export const TOKEN_ACCESS: readonly AccessLevel[] = [
  {
    value: "run",
    label: "Run sessions",
    description: "Starts sessions, messages their agents and keeps projects' memory. For CI and bots.",
  },
  {
    value: "read",
    label: "Read",
    description:
      "Reads projects, sessions and their transcripts; changes nothing. For dashboards and reports.",
  },
  {
    value: "full",
    label: "Everything",
    description: "All you can do in its projects, their settings and secrets too. For your own scripts.",
  },
];

/** How long a new token works. */
export const EXPIRIES: readonly { value: string; label: string; days?: number }[] = [
  { value: "7", label: "7 days", days: 7 },
  { value: "30", label: "30 days", days: 30 },
  { value: "90", label: "90 days", days: 90 },
  { value: "365", label: "A year", days: 365 },
  { value: "never", label: "Never" },
];

/** What a new token is for, as the form holds it. */
export interface TokenForm {
  name: string;
  access: G.TokenAccess;
  /** Every project you see (`undefined`), or these workspaces and projects, by id. */
  places?: { workspaces: string[]; projects: string[] };
  github: G.GithubAccess;
  /** One of `EXPIRIES`' values. */
  expires: string;
}

export const NEW_TOKEN: TokenForm = { name: "", access: "run", github: "push", expires: "90" };

/** The request that makes the token `f` describes. */
export function tokenRequest(f: TokenForm): G.CreateTokenRequest {
  const days = EXPIRIES.find((e) => e.value === f.expires)?.days;
  const req: G.CreateTokenRequest = { name: f.name.trim(), access: f.access };
  if (days) req.expires_in_days = days;
  if (f.places?.workspaces.length) req.workspaces = f.places.workspaces;
  if (f.places?.projects.length) req.projects = f.places.projects;
  // A token that reads starts no session.
  if (f.access !== "read" && f.github !== "push") req.github_access = f.github;
  return req;
}

/** Whether the form makes a token at all: it has a name, and somewhere to reach. */
export const ready = (f: TokenForm) =>
  !!f.name.trim() && (!f.places || f.places.workspaces.length + f.places.projects.length > 0);

/** "a", "a and b", "a, b and c". */
export function words(items: string[]): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}

/** The places a scope names, as a sentence says them. */
function where(places: G.TokenPlaces | undefined): string {
  if (!places) return "all your projects";
  const items = [
    ...places.projects.map((p) => p.name ?? "a project that's gone"),
    ...places.workspaces.map((w) => (w.name ? `every project of ${w.name}` : "a workspace you left")),
  ];
  return items.length ? words(items) : "no project";
}

const AGENTS: Partial<Record<G.GithubAccess, string>> = {
  comment: "their agents review and comment on GitHub, and push nothing",
  read: "their agents only read on GitHub",
};

/**
 * What a token does, in a sentence: "Runs sessions in Web; their agents review and comment on
 * GitHub, and push nothing." `undefined` is a token without limits.
 */
export function describeScope(scope: G.TokenScope | undefined): string {
  if (!scope) return "Does everything you can, as you do when signed in.";
  const places = where(scope.places);
  const agents = scope.access !== "read" && scope.github_access ? AGENTS[scope.github_access] : undefined;
  const does = { read: "Reads", run: "Runs sessions in", full: "Does everything you can in" }[scope.access];
  return `${does} ${places}${agents ? `; ${agents}` : ""}.`;
}

/** What a token reaches besides, said after what it does. */
export function reach(scope: G.TokenScope | undefined): string {
  return scope
    ? "It never reaches your account, your workspaces' people and settings, or your other tokens."
    : "That includes your account, your workspaces and your other tokens.";
}

/** The scope `f` asks for, with the names the form shows, before the token exists. */
export function formScope(
  f: TokenForm,
  names: { workspaces: Map<string, string>; projects: Map<string, string> },
): G.TokenScope | undefined {
  // With everything, everywhere and agents that push, it has no limits.
  const limited = f.access !== "full" || !!f.places || f.github !== "push";
  if (!limited) return undefined;
  const place = (names: Map<string, string>) => (id: string) => ({ id, name: names.get(id) });
  return {
    access: f.access,
    places: f.places && {
      workspaces: f.places.workspaces.map(place(names.workspaces)),
      projects: f.places.projects.map(place(names.projects)),
    },
    github_access: f.access !== "read" && f.github !== "push" ? f.github : undefined,
  };
}

/** A command that shows how to use a new token: start a session, or read the sessions. */
export function example(scope: G.TokenScope | undefined, secret: string): string {
  const env = `FCTL_TOKEN=${secret}`;
  if (scope?.access === "read") return `${env} fctl sessions --json`;
  const project = scope?.places?.projects.find((p) => p.slug)?.slug;
  const github = scope?.github_access ? ` --github ${scope.github_access}` : "";
  const task = scope?.github_access === "comment" ? "Review pull request #12" : "Fix the flaky login test";
  return `${env} fctl run${project ? ` -p ${project}` : ""}${github} -d "${task}"`;
}

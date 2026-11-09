// A project's page (routes/project.tsx): its tabs, Memory's two parts, the pages of its Settings and
// how they group, who sees which, what an address opens, and what the list of Settings' pages on a
// phone says about each.

import type * as G from "@/gen";
import { imageSource } from "./images";
import { effortLabel, modelLabel } from "./models";
import { TRACKERS, trackerOf } from "./tickets";

/** The tabs under the project's header, in their order: what its agents do, who and what it has, its settings. */
export const PROJECT_TABS = [
  "sessions",
  "autopilot",
  "schedules",
  "reviews",
  "memory",
  "repositories",
  "members",
  "settings",
] as const;
export type ProjectTab = (typeof PROJECT_TABS)[number];

/** Settings' pages, in the order its navigation lists them. */
export const SETTINGS_PAGES = [
  "general",
  "agents",
  "pull-requests",
  "tickets",
  "sandbox",
  "secrets",
] as const;
export type ProjectSettingsPage = (typeof SETTINGS_PAGES)[number];

/**
 * What the page's address takes as `?tab=`: a tab; Memory's retrospectives (`memory` is its notes);
 * or a page of Settings (`settings` is Settings itself: the list of its pages on a phone, the first
 * of them beside it elsewhere). Tickets, Retrospectives and Secrets were tabs of their own, so their
 * old addresses land where they are now.
 */
export const PROJECT_VIEWS = [...PROJECT_TABS, "retrospectives", ...SETTINGS_PAGES] as const;
export type ProjectView = (typeof PROJECT_VIEWS)[number];

export const isSettingsPage = (v: unknown): v is ProjectSettingsPage => SETTINGS_PAGES.some((p) => p === v);

/** The tab a view is under: Memory's retrospectives under Memory, Settings' pages under Settings. */
export function tabOf(view: ProjectView): ProjectTab {
  if (view === "retrospectives") return "memory";
  if (isSettingsPage(view)) return "settings";
  return view;
}

/** Owners and maintainers change a project's settings, secrets and members. */
export const manages = (role?: G.ProjectRole) => role === "owner" || role === "maintainer";

/** Pages everyone in the project reads: where its agents track their work. */
const OPEN_PAGES: readonly ProjectSettingsPage[] = ["tickets"];

/** The pages of Settings someone in the project sees, in their order. */
export function settingsPagesFor(role?: G.ProjectRole): ProjectSettingsPage[] {
  return SETTINGS_PAGES.filter((p) => manages(role) || OPEN_PAGES.includes(p));
}

export interface SettingsEntry {
  page: ProjectSettingsPage;
  label: string;
}

/** Settings' pages in their groups: the project, how its agents work, where they run. */
export const SETTINGS_GROUPS: readonly (readonly SettingsEntry[])[] = [
  [{ page: "general", label: "General" }],
  [
    { page: "agents", label: "Agents" },
    { page: "pull-requests", label: "Pull requests" },
    { page: "tickets", label: "Tickets" },
  ],
  [
    { page: "sandbox", label: "Sandbox" },
    { page: "secrets", label: "Secrets" },
  ],
];

export const settingsLabel = (page: ProjectSettingsPage) =>
  SETTINGS_GROUPS.flat().find((e) => e.page === page)?.label ?? page;

/**
 * What an address shows of a project: the view it names, where the person may see it and the
 * project has it; else Settings itself for a page of Settings they don't see, and Sessions for an
 * Autopilot tab the project hasn't (its workspace allows no autopilots).
 */
export function shownView(
  asked: ProjectView | undefined,
  project: Pick<G.Project, "role" | "autopilot">,
): ProjectView {
  const view = asked ?? "sessions";
  if (view === "autopilot" && !project.autopilot) return "sessions";
  if (isSettingsPage(view) && !settingsPagesFor(project.role).includes(view)) return "settings";
  return view;
}

/** The page of Settings a view opens: its own, or for Settings itself the first one the person sees. */
export function settingsPageOf(view: ProjectView, role?: G.ProjectRole): ProjectSettingsPage | undefined {
  return isSettingsPage(view) ? view : settingsPagesFor(role)[0];
}

const isView = (v: unknown): v is ProjectView => PROJECT_VIEWS.some((x) => x === v);

/**
 * Where an address takes someone, as a project's page shows it: the tab, Memory's part or Settings'
 * page open there. Two addresses that show the same are one place: Settings itself and the page it
 * opens beside its list, Tickets with Linear's way back and without it.
 */
export function placeOf(
  project: Pick<G.Project, "slug" | "role" | "autopilot">,
  at: { pathname: string; search: unknown },
): string {
  if (at.pathname !== `/p/${project.slug}`) return at.pathname;
  const tab = (at.search as { tab?: unknown } | undefined)?.tab;
  const view = shownView(isView(tab) ? tab : undefined, project);
  return tabOf(view) === "settings" ? `settings/${settingsPageOf(view, project.role)}` : view;
}

const count = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n} ${many}`);

/** The model and effort agents work with: "Opus · high effort", "Claude's default model". */
export function agentsGist(s: G.ProjectSettings): string {
  const model = s.model?.trim();
  const effort = s.effort?.trim();
  return [
    model ? modelLabel(model) : "Claude's default model",
    effort && `${effortLabel(effort).toLowerCase()} effort`,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** Whether agents open pull requests, and what brings them back: "Opened when done · 4 follow-ups". */
export function pullRequestsGist(s: G.ProjectSettings): string {
  const f = s.follow_ups;
  const n = f ? [f.comments, f.checks, f.conflicts, f.closed].filter(Boolean).length : 0;
  return [
    s.auto_pull_request ? "Opened when done" : "Opened when asked",
    n === 0 ? "no follow-ups" : count(n, "follow-up", "follow-ups"),
  ].join(" · ");
}

/** The tracker, with its first Linear team: "GitHub Issues", "Linear · ENG and 1 more". */
export function ticketsGist(s: G.ProjectSettings): string {
  const tracker = trackerOf(s);
  const label = TRACKERS.find((t) => t.value === tracker)?.label ?? tracker;
  const linear = s.tickets?.linear;
  if (tracker !== "linear" || !linear?.team) return label;
  const first = linear.team_key ?? linear.team_name;
  const more = linear.more_teams?.length ?? 0;
  return first ? `${label} · ${first}${more > 0 ? ` and ${more} more` : ""}` : label;
}

/** What a sandbox starts from: "Each runner's own image", "ghcr.io/acme/agent:1", "Built from Dockerfile". */
export function sandboxGist(s: G.ProjectSettings): string {
  switch (imageSource(s)) {
    case "build":
      return `Built from ${s.image_build?.file.trim() || "the repository"}`;
    case "image":
      return s.image?.trim() ?? "An image from a registry";
    default:
      return "Each runner's own image";
  }
}

/** What the list of Settings' pages says about each, under its name, from what has loaded. */
export function settingsGists(
  project: G.Project,
  loaded: { secrets?: number } = {},
): Partial<Record<ProjectSettingsPage, string>> {
  const s = project.settings;
  return {
    general: `Sessions start from ${project.default_branch}`,
    agents: agentsGist(s),
    "pull-requests": pullRequestsGist(s),
    tickets: ticketsGist(s),
    sandbox: sandboxGist(s),
    secrets:
      loaded.secrets === undefined
        ? undefined
        : loaded.secrets === 0
          ? "None yet"
          : count(loaded.secrets, "secret", "secrets"),
  };
}

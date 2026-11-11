// Routes. Signed-in pages live under the `app` layout (the shell, or the page
// that sets up two-factor sign-in when the instance requires it); the guard
// sends everyone else to /login (or /setup on a fresh instance).

import { createRootRoute, createRoute, createRouter, Outlet, redirect } from "@tanstack/react-router";
import { api, START_WITHIN } from "@/lib/api";
import { appHistory } from "@/lib/history";
import { PROJECT_VIEWS, type ProjectView } from "@/lib/project-page";
import { fetchMe, keys, queryClient, startRetries } from "@/lib/queries";
import { SETTINGS_TABS, type SettingsTab } from "@/lib/settings";
import { routerScrolls } from "@/lib/stick-to-bottom";
import { LINEAR_OUTCOMES, type LinearOutcome } from "@/lib/tickets";
import { Admin } from "@/routes/admin";
import { InvitePage, LoginPage, ResetPage, SetupPage, TwoFactorPage } from "@/routes/auth";
import { DevicePage } from "@/routes/device";
import { EarlyAccessPage } from "@/routes/early-access";
import { ErrorPage } from "@/routes/error";
import { Home } from "@/routes/home";
import { NewProject } from "@/routes/new-project";
import { NewWorkspace } from "@/routes/new-workspace";
import { NotFound } from "@/routes/not-found";
import { ProjectPage } from "@/routes/project";
import { PullsPage, type PullsSearch } from "@/routes/pulls";
import { SessionPage } from "@/routes/session";
import { SettingsPage } from "@/routes/settings";
import { Starting } from "@/routes/starting";
import { SignedInLayout } from "@/routes/two-factor";
import {
  GITHUB_OUTCOMES,
  GITHUB_REFUSALS,
  type GithubOutcome,
  type GithubRefusal,
  WorkspacePage,
  type WorkspaceTab,
} from "@/routes/workspace";

// Pages wait to know who is signed in, and what the instance offers, as the app starts. A request
// the server never answers (the app on an iPhone's Home Screen can come back after a while to one)
// kept the app blank until it was closed: each now fails after START_WITHIN and goes again once
// (other failures go again as any query's do), and then the error page says so, with its Try
// again. Meanwhile, from a second on, Starting says what the app waits for.
const me = () =>
  queryClient.ensureQueryData({
    queryKey: keys.me,
    queryFn: () => fetchMe(START_WITHIN),
    retry: startRetries,
  });
const instance = () =>
  queryClient.ensureQueryData({
    queryKey: keys.instance,
    queryFn: () => api.instance(START_WITHIN),
    retry: startRetries,
  });

// Unknown addresses get NotFound; a failed request or a crash anywhere, ErrorPage.
const root = createRootRoute({ component: Outlet, notFoundComponent: NotFound, errorComponent: ErrorPage });

const str = (v: unknown) => (typeof v === "string" && v ? v : undefined);
/** `v` when it is one of `values`: an old or mistyped link opens the default instead. */
const oneOf = <T extends string>(v: unknown, values: readonly T[]): T | undefined =>
  values.find((x) => x === v);
// Search values arrive JSON-parsed (`?deleted=1` is the number 1).
const flag = (v: unknown) => (v === undefined || v === null || v === false || v === "" ? undefined : true);

const login = createRoute({
  getParentRoute: () => root,
  path: "/login",
  component: LoginPage,
  validateSearch: (s: Record<string, unknown>) => ({
    next: str(s.next),
    error: str(s.error),
    deleted: flag(s.deleted),
  }),
  beforeLoad: async () => {
    const info = await instance();
    if (info.setup_required) throw redirect({ to: "/setup" });
    if (await me()) throw redirect({ to: "/" });
  },
});
const setup = createRoute({ getParentRoute: () => root, path: "/setup", component: SetupPage });
// Where the instance takes requests for early access (as app.fctl.sh does).
const earlyAccess = createRoute({
  getParentRoute: () => root,
  path: "/early-access",
  component: EarlyAccessPage,
  beforeLoad: async () => {
    const info = await instance();
    if (info.setup_required) throw redirect({ to: "/setup" });
    if (!info.features.early_access) {
      throw redirect({ to: "/login", search: { next: undefined, error: undefined, deleted: undefined } });
    }
    if (await me()) throw redirect({ to: "/" });
  },
});
const invite = createRoute({ getParentRoute: () => root, path: "/invite", component: InvitePage });
const reset = createRoute({ getParentRoute: () => root, path: "/reset", component: ResetPage });
// After Google, when the account's Google sign-ins ask for a code.
const twoFactor = createRoute({ getParentRoute: () => root, path: "/two-factor", component: TwoFactorPage });

const app = createRoute({
  getParentRoute: () => root,
  id: "app",
  component: SignedInLayout,
  beforeLoad: async ({ location }) => {
    const user = await me();
    if (!user) {
      const info = await instance();
      if (info.setup_required) throw redirect({ to: "/setup" });
      throw redirect({ to: "/login", search: { next: location.href, error: undefined, deleted: undefined } });
    }
  },
});

const home = createRoute({ getParentRoute: () => app, path: "/", component: Home });
const device = createRoute({
  getParentRoute: () => app,
  path: "/device",
  component: DevicePage,
  validateSearch: (s: Record<string, unknown>) => ({ code: str(s.code) }),
});
const newProject = createRoute({
  getParentRoute: () => app,
  path: "/new",
  component: NewProject,
  // Optional, so links to it need no search: the workspace to create it in.
  validateSearch: (s: Record<string, unknown>): { workspace?: string } => ({ workspace: str(s.workspace) }),
});
const newWorkspace = createRoute({ getParentRoute: () => app, path: "/w/new", component: NewWorkspace });
const workspace = createRoute({
  getParentRoute: () => app,
  path: "/w/$slug",
  component: WorkspacePage,
  validateSearch: (
    s: Record<string, unknown>,
  ): {
    tab?: WorkspaceTab;
    github?: GithubOutcome;
    account?: string;
    reason?: GithubRefusal;
    linear?: LinearOutcome;
  } => ({
    tab: oneOf(s.tab, ["projects", "people", "runners", "github", "linear", "settings", "audit"] as const),
    // Back from GitHub: installing the app, or the sign-in that connects an account (api::github).
    github: oneOf(s.github, GITHUB_OUTCOMES),
    account: str(s.account),
    reason: oneOf(s.reason, GITHUB_REFUSALS),
    // Back from Linear: installing fctl's app there (api::linear).
    linear: oneOf(s.linear, LINEAR_OUTCOMES),
  }),
});
const project = createRoute({
  getParentRoute: () => app,
  path: "/p/$slug",
  component: ProjectPage,
  validateSearch: (
    s: Record<string, unknown>,
  ): { tab?: ProjectView; schedule?: string; linear?: LinearOutcome } => ({
    // A tab, or a page of Memory or Settings (lib/project-page.ts); an unknown one opens Sessions.
    tab: oneOf(s.tab, PROJECT_VIEWS),
    // The schedule the Schedules tab opens, from a session it started.
    schedule: str(s.schedule),
    // Back from Linear: installing fctl's app, or linking someone's account (api::linear).
    linear: oneOf(s.linear, LINEAR_OUTCOMES),
  }),
});
const session = createRoute({
  getParentRoute: () => app,
  path: "/s/$id",
  component: SessionPage,
  // From a search of all sessions: what was found, and the event it was found in (lib/find.ts).
  // From a link to a turn: the event the turn starts with.
  validateSearch: (s: Record<string, unknown>): { find?: string; at?: number; turn?: number } => ({
    find: str(s.find),
    at: typeof s.at === "number" && Number.isSafeInteger(s.at) && s.at > 0 ? s.at : undefined,
    turn: typeof s.turn === "number" && Number.isSafeInteger(s.turn) && s.turn > 0 ? s.turn : undefined,
  }),
});
const pulls = createRoute({
  getParentRoute: () => app,
  path: "/pulls",
  component: PullsPage,
  validateSearch: (s: Record<string, unknown>): PullsSearch => ({
    state: oneOf(s.state, ["open", "merged", "closed"] as const),
    scope: typeof s.scope === "string" && /^[wp]:[a-z0-9-]+$/.test(s.scope) ? s.scope : undefined,
    q: str(s.q),
  }),
});
const settings = createRoute({
  getParentRoute: () => app,
  path: "/settings",
  component: SettingsPage,
  validateSearch: (
    s: Record<string, unknown>,
  ): { tab?: SettingsTab; linked?: string; error?: string; linear?: LinearOutcome } => ({
    // The page open; none: the list of pages on a phone, Profile beside it (lib/settings.ts).
    tab: oneOf(s.tab, SETTINGS_TABS),
    // Back from Google: linking it, which worked or didn't (api::google).
    linked: str(s.linked),
    error: str(s.error),
    // Back from Linear: linking your account (api::linear).
    linear: oneOf(s.linear, LINEAR_OUTCOMES),
  }),
});
const admin = createRoute({
  getParentRoute: () => app,
  path: "/admin",
  component: Admin,
  validateSearch: (s: Record<string, unknown>) => ({
    // "users" is what the People tab was called.
    tab:
      s.tab === "users"
        ? undefined
        : oneOf(s.tab, [
            "people",
            "access",
            "early-access",
            "workspaces",
            "github",
            "runners",
            "settings",
            "audit",
          ] as const),
  }),
});

const tree = root.addChildren([
  login,
  setup,
  earlyAccess,
  invite,
  reset,
  twoFactor,
  app.addChildren([
    home,
    device,
    newProject,
    newWorkspace,
    workspace,
    project,
    session,
    pulls,
    settings,
    admin,
  ]),
]);

export const router = createRouter({
  routeTree: tree,
  // On an iPhone's or iPad's Home Screen, with no history for iOS's edge swipes to go through.
  history: appHistory(),
  defaultPreload: "intent",
  // What shows while a page waits on the server for over a second, as one does when the app starts.
  defaultPendingComponent: Starting,
  defaultViewTransition: true,
  // A session's page opens at its newest work by itself (lib/stick-to-bottom.ts).
  scrollRestoration: ({ location }) => routerScrolls(location.pathname),
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

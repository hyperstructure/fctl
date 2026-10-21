// Routes. Signed-in pages live under the `app` layout (the shell, or the page
// that sets up two-factor sign-in when the instance requires it); the guard
// sends everyone else to /login (or /setup on a fresh instance).

import { createRootRoute, createRoute, createRouter, Outlet, redirect } from "@tanstack/react-router";
import { ApiFailure, api } from "@/lib/api";
import { keys, queryClient } from "@/lib/queries";
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
import { SessionPage } from "@/routes/session";
import { SettingsPage } from "@/routes/settings";
import { SignedInLayout } from "@/routes/two-factor";
import {
  GITHUB_OUTCOMES,
  GITHUB_REFUSALS,
  type GithubOutcome,
  type GithubRefusal,
  WorkspacePage,
  type WorkspaceTab,
} from "@/routes/workspace";

async function me() {
  return queryClient.ensureQueryData({
    queryKey: keys.me,
    queryFn: async () => {
      try {
        return await api.me();
      } catch (e) {
        if (e instanceof ApiFailure && (e.status === 401 || e.status === 403)) return null;
        throw e;
      }
    },
  });
}

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
    const info = await queryClient.ensureQueryData({ queryKey: keys.instance, queryFn: api.instance });
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
    const info = await queryClient.ensureQueryData({ queryKey: keys.instance, queryFn: api.instance });
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
      const info = await queryClient.ensureQueryData({ queryKey: keys.instance, queryFn: api.instance });
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
  ): { tab?: WorkspaceTab; github?: GithubOutcome; account?: string; reason?: GithubRefusal } => ({
    tab: oneOf(s.tab, ["projects", "people", "runners", "github", "settings", "audit"] as const),
    // Back from GitHub: installing the app, or the sign-in that connects an account (api::github).
    github: oneOf(s.github, GITHUB_OUTCOMES),
    account: str(s.account),
    reason: oneOf(s.reason, GITHUB_REFUSALS),
  }),
});
const project = createRoute({
  getParentRoute: () => app,
  path: "/p/$slug",
  component: ProjectPage,
  validateSearch: (s: Record<string, unknown>) => ({
    tab: str(s.tab) as
      | "sessions"
      | "memory"
      | "retrospectives"
      | "repositories"
      | "settings"
      | "members"
      | "secrets"
      | undefined,
  }),
});
const session = createRoute({ getParentRoute: () => app, path: "/s/$id", component: SessionPage });
const settings = createRoute({
  getParentRoute: () => app,
  path: "/settings",
  component: SettingsPage,
  validateSearch: (s: Record<string, unknown>) => ({ linked: str(s.linked), error: str(s.error) }),
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
  app.addChildren([home, device, newProject, newWorkspace, workspace, project, session, settings, admin]),
]);

export const router = createRouter({
  routeTree: tree,
  defaultPreload: "intent",
  defaultViewTransition: true,
  scrollRestoration: true,
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}

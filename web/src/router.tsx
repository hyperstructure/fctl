// Routes. Signed-in pages live under the `app` layout (the shell); the
// guard sends everyone else to /login (or /setup on a fresh instance).

import { createRootRoute, createRoute, createRouter, Outlet, redirect } from "@tanstack/react-router";
import { AppShell } from "@/components/shell/app-shell";
import { ApiFailure, api } from "@/lib/api";
import { keys, queryClient } from "@/lib/queries";
import { Admin } from "@/routes/admin";
import { InvitePage, LoginPage, ResetPage, SetupPage } from "@/routes/auth";
import { DevicePage } from "@/routes/device";
import { ErrorPage } from "@/routes/error";
import { Home } from "@/routes/home";
import { NewProject } from "@/routes/new-project";
import { NotFound } from "@/routes/not-found";
import { ProjectPage } from "@/routes/project";
import { SessionPage } from "@/routes/session";
import { SettingsPage } from "@/routes/settings";

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
const invite = createRoute({ getParentRoute: () => root, path: "/invite", component: InvitePage });
const reset = createRoute({ getParentRoute: () => root, path: "/reset", component: ResetPage });

const app = createRoute({
  getParentRoute: () => root,
  id: "app",
  component: AppShell,
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
const newProject = createRoute({ getParentRoute: () => app, path: "/new", component: NewProject });
const project = createRoute({
  getParentRoute: () => app,
  path: "/p/$slug",
  component: ProjectPage,
  validateSearch: (s: Record<string, unknown>) => ({
    tab: str(s.tab) as "sessions" | "repositories" | "settings" | "members" | "secrets" | undefined,
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
    tab: str(s.tab) as "access" | "users" | "runners" | "audit" | undefined,
  }),
});

const tree = root.addChildren([
  login,
  setup,
  invite,
  reset,
  app.addChildren([home, device, newProject, project, session, settings, admin]),
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

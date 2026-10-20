// The signed-in layout: a quiet sidebar, the page, and the command palette.

import { Link, Outlet, useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import {
  ChevronRight,
  ChevronsUpDown,
  Cpu,
  FolderPlus,
  Lock,
  LogOut,
  Menu as MenuIcon,
  Moon,
  Plus,
  Search,
  Settings,
  Shield,
  Sun,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import { Avatar, HomeIcon, Kbd, Logo, Menu, MenuItem, MenuSeparator, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { forgetDraft } from "@/lib/draft";
import { useFeed } from "@/lib/feed";
import { useHotkey } from "@/lib/hotkeys";
import { forgetPush, onPushOpen, syncPush } from "@/lib/push";
import { queryClient, useInstance, useMe, useProjects, useWorkspaces } from "@/lib/queries";
import { useTheme } from "@/lib/theme";
import { groupProjects, useFactorsWatch, useFolded } from "@/lib/workspaces";
import { CommandPalette } from "./command-palette";

export function AppShell() {
  const [palette, setPalette] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const openPalette = useCallback(() => setPalette(true), []);
  useHotkey("mod+k", openPalette, { inInputs: true });
  useFeed(); // live updates for every page
  usePush();
  useFactorsWatch();
  const path = useRouterState({ select: (s) => s.location.pathname });
  return (
    <div className="flex min-h-dvh bg-canvas">
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-30 flex w-[calc(244px+env(safe-area-inset-left))] flex-col border-r border-line bg-surface/80 pl-[env(safe-area-inset-left)] backdrop-blur-xl transition-transform duration-300 ease-out md:translate-x-0",
          drawer ? "translate-x-0" : "-translate-x-full",
        )}
      >
        <Sidebar onPalette={openPalette} onNavigate={() => setDrawer(false)} path={path} />
      </aside>
      {drawer && (
        <button
          type="button"
          aria-label="Close menu"
          className="fixed inset-0 z-20 bg-black/40 md:hidden"
          onClick={() => setDrawer(false)}
        />
      )}
      <div className="flex min-w-0 flex-1 flex-col pr-[env(safe-area-inset-right)] pl-[env(safe-area-inset-left)] md:pl-[calc(244px+env(safe-area-inset-left))]">
        <div className="sticky top-0 z-10 flex h-12 items-center gap-2 border-b border-line bg-canvas/80 px-3 backdrop-blur-xl md:hidden">
          <button
            type="button"
            aria-label="Open menu"
            className="rounded-md p-2 text-ink-3 hover:text-ink"
            onClick={() => setDrawer(true)}
          >
            <MenuIcon className="size-4.5" />
          </button>
          <Logo />
        </div>
        <main className="min-w-0 flex-1">
          <Outlet />
        </main>
      </div>
      <CommandPalette open={palette} onClose={() => setPalette(false)} />
    </div>
  );
}

/** Notifications in this browser: keep its subscription current, and open what a click is about. */
function usePush() {
  const { data: me } = useMe();
  const router = useRouter();
  const userId = me?.user.id;
  useEffect(() => {
    if (userId) syncPush(userId).catch((e) => console.warn("fctl: notifications", e));
  }, [userId]);
  useEffect(() => onPushOpen((url) => router.history.push(url)), [router]);
}

function NavLink({
  to,
  icon,
  children,
  active,
  onClick,
}: {
  to: string;
  icon: React.ReactNode;
  children: React.ReactNode;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <Link
      to={to}
      onClick={onClick}
      className={cn(
        "group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
        active
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "text-ink-3 hover:bg-hover/60 hover:text-ink",
      )}
    >
      <span
        className={cn(
          "transition-colors [&_svg]:size-4",
          active ? "text-accent" : "text-ink-4 group-hover:text-ink-3",
        )}
      >
        {icon}
      </span>
      {children}
    </Link>
  );
}

function Sidebar({
  onPalette,
  onNavigate,
  path,
}: {
  onPalette: () => void;
  onNavigate: () => void;
  path: string;
}) {
  const { data: me } = useMe();
  const { data: instance } = useInstance();
  const { data: projects } = useProjects();
  const { data: workspaces } = useWorkspaces();
  const [folded, toggleFold] = useFolded();
  const groups = groupProjects(workspaces?.workspaces ?? [], projects?.projects ?? []);
  const [theme, setTheme] = useTheme();
  const navigate = useNavigate();
  const isAdmin = me?.user.role === "owner" || me?.user.role === "admin";
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 items-center justify-between px-4">
        <Link to="/" onClick={onNavigate} className="rounded-md">
          <Logo />
        </Link>
        <button
          type="button"
          aria-label="Close menu"
          className="rounded-md p-1.5 text-ink-4 hover:text-ink md:hidden"
          onClick={onNavigate}
        >
          <X className="size-4" />
        </button>
      </div>
      <div className="px-3">
        <button
          type="button"
          onClick={onPalette}
          className="flex h-8.5 w-full items-center gap-2.5 rounded-md border border-line bg-sunken/60 px-2.5 text-[13px] text-ink-4 transition-colors hover:border-line-strong hover:text-ink-3"
        >
          <Search className="size-3.5" />
          <span className="flex-1 text-left">Search or jump to…</span>
          <Kbd>⌘K</Kbd>
        </button>
      </div>
      <nav className="mt-4 flex flex-col gap-0.5 px-3">
        <NavLink to="/" icon={<HomeIcon />} active={path === "/"} onClick={onNavigate}>
          Home
        </NavLink>
      </nav>
      <nav
        aria-label="Workspaces"
        className="mt-5 flex min-h-0 flex-1 flex-col gap-3 overflow-y-auto px-3 pb-3"
      >
        {workspaces?.invitations.map((i) => (
          <Link
            key={i.id}
            to="/"
            onClick={onNavigate}
            className="group flex items-center gap-2.5 rounded-md border border-accent/25 bg-accent-soft px-2.5 py-2 text-[12.5px] text-ink-2 transition-colors hover:border-accent/45"
          >
            <WorkspaceAvatar workspace={i.workspace} size={18} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-ink">{i.workspace.name}</span>
              <span className="block truncate text-[11.5px] text-ink-3">Invited · join on Home</span>
            </span>
          </Link>
        ))}
        {groups.map(({ workspace, projects: list }) => (
          <WorkspaceGroup
            key={workspace.id}
            workspace={workspace}
            projects={list}
            path={path}
            folded={folded.has(workspace.id)}
            onFold={() => toggleFold(workspace.id)}
            onNavigate={onNavigate}
          />
        ))}
        {workspaces && workspaces.workspaces.length < workspaces.limit && (
          <Link
            to="/w/new"
            onClick={onNavigate}
            className={cn(
              "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] transition-colors",
              workspaces.workspaces.length === 0
                ? "text-ink-2 hover:bg-hover hover:text-ink"
                : "text-ink-4 hover:bg-hover/60 hover:text-ink-2",
            )}
          >
            <Plus className="size-3.5" />
            {workspaces.workspaces.length === 0 ? "Create a workspace" : "New workspace"}
          </Link>
        )}
      </nav>
      <div className="flex flex-col gap-0.5 border-t border-line px-3 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        {isAdmin && (
          <NavLink to="/admin" icon={<Shield />} active={path === "/admin"} onClick={onNavigate}>
            Admin
          </NavLink>
        )}
        <NavLink to="/settings" icon={<Settings />} active={path === "/settings"} onClick={onNavigate}>
          Settings
        </NavLink>
        {me && (
          // At the bottom of the window: the menu opens upward.
          <Menu
            side="top"
            align="stretch"
            className="mt-1"
            trigger={(toggle, open) => (
              <button
                type="button"
                onClick={toggle}
                aria-haspopup="menu"
                aria-expanded={open}
                className="group flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-hover/60 aria-expanded:bg-hover/60"
              >
                <Avatar user={me.user} size={24} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] text-ink">{me.user.name}</span>
                  <span className="block truncate text-[11.5px] text-ink-4">{instance?.name ?? "fctl"}</span>
                </span>
                <ChevronsUpDown className="size-3.5 shrink-0 text-ink-4 transition-colors group-hover:text-ink-3 group-aria-expanded:text-ink-3" />
              </button>
            )}
          >
            {(close) => (
              <>
                <MenuItem
                  icon={theme === "dark" ? <Sun /> : <Moon />}
                  onClick={() => {
                    setTheme(theme === "dark" ? "light" : "dark");
                    close();
                  }}
                >
                  {theme === "dark" ? "Light theme" : "Dark theme"}
                </MenuItem>
                <MenuItem
                  icon={<Cpu />}
                  onClick={() => {
                    close();
                    onNavigate();
                    void navigate({ to: "/settings", search: { linked: undefined, error: undefined } });
                  }}
                >
                  Claude and tokens
                </MenuItem>
                <MenuSeparator />
                <MenuItem
                  icon={<LogOut />}
                  onClick={async () => {
                    close();
                    await forgetPush();
                    await forgetDraft();
                    await api.logout().catch(() => {});
                    queryClient.clear();
                    window.location.assign("/login");
                  }}
                >
                  Sign out
                </MenuItem>
              </>
            )}
          </Menu>
        )}
      </div>
    </div>
  );
}

/** A workspace in the sidebar: its name (its page) and its projects, which fold away. */
function WorkspaceGroup({
  workspace,
  projects,
  path,
  folded,
  onFold,
  onNavigate,
}: {
  workspace: G.Workspace;
  projects: G.Project[];
  path: string;
  folded: boolean;
  onFold: () => void;
  onNavigate: () => void;
}) {
  const here = path === `/w/${workspace.slug}`;
  const active = projects.reduce((n, p) => n + p.active_sessions, 0);
  return (
    <section aria-label={workspace.name}>
      <div
        className={cn(
          "group flex h-8 items-center gap-1 rounded-md pr-1 transition-colors",
          here ? "bg-raised shadow-[inset_0_0_0_1px_var(--c-line)]" : "hover:bg-hover/60",
        )}
      >
        <button
          type="button"
          onClick={onFold}
          aria-expanded={!folded}
          aria-label={folded ? `Show ${workspace.name}'s projects` : `Hide ${workspace.name}'s projects`}
          className="flex h-8 w-5 shrink-0 items-center justify-center rounded text-ink-4 transition-colors hover:text-ink-2"
        >
          <ChevronRight
            className={cn(
              "size-3 transition-transform duration-200 ease-[var(--ease-out)] motion-reduce:transition-none",
              !folded && "rotate-90",
            )}
          />
        </button>
        <Link
          to="/w/$slug"
          params={{ slug: workspace.slug }}
          search={{ tab: undefined }}
          onClick={onNavigate}
          className="flex min-w-0 flex-1 items-center gap-2 py-1"
        >
          <WorkspaceAvatar workspace={workspace} size={18} title={false} />
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[13px] font-medium",
              here ? "text-ink" : "text-ink-2 group-hover:text-ink",
            )}
          >
            {workspace.name}
          </span>
          {workspace.blocked ? (
            <Lock
              className="size-3 shrink-0 text-amber"
              aria-label="Closed until you set up two-factor sign-in"
            />
          ) : (
            folded && active > 0 && <span className="text-[11.5px] text-accent tabular-nums">{active}</span>
          )}
        </Link>
        {!workspace.blocked && (
          <Link
            to="/new"
            search={{ workspace: workspace.slug }}
            onClick={onNavigate}
            aria-label={`New project in ${workspace.name}`}
            className="rounded p-1 text-ink-4 opacity-0 transition-[opacity,color] group-hover:opacity-100 hover:text-ink focus-visible:opacity-100"
          >
            <Plus className="size-3.5" />
          </Link>
        )}
      </div>
      {!folded && (
        <div className="mt-0.5 flex flex-col gap-0.5 pl-3.5">
          {workspace.blocked ? (
            <Link
              to="/settings"
              search={{ linked: undefined, error: undefined }}
              onClick={onNavigate}
              className="flex items-center gap-2 rounded-md px-2.5 py-1.5 text-[12.5px] leading-snug text-ink-3 hover:text-ink"
            >
              Set up two-factor sign-in to open it
            </Link>
          ) : (
            <>
              {projects.map((p) => (
                <Link
                  key={p.id}
                  to="/p/$slug"
                  params={{ slug: p.slug }}
                  search={{ tab: undefined }}
                  onClick={onNavigate}
                  className={cn(
                    "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
                    path === `/p/${p.slug}`
                      ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                      : "text-ink-3 hover:bg-hover/60 hover:text-ink",
                  )}
                >
                  <span className="flex size-4 items-center justify-center">
                    {p.active_sessions > 0 ? (
                      <StatusDot status="running" />
                    ) : (
                      <span className="size-1.5 rounded-full bg-ink-4/60" />
                    )}
                  </span>
                  <span className="min-w-0 flex-1 truncate">{p.name}</span>
                  {p.active_sessions > 0 && (
                    <span className="text-[11.5px] text-ink-4 tabular-nums">{p.active_sessions}</span>
                  )}
                </Link>
              ))}
              {projects.length === 0 && (
                <Link
                  to="/new"
                  search={{ workspace: workspace.slug }}
                  onClick={onNavigate}
                  className="flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] text-ink-4 hover:text-ink"
                >
                  <FolderPlus className="size-4" /> Create a project
                </Link>
              )}
            </>
          )}
        </div>
      )}
    </section>
  );
}

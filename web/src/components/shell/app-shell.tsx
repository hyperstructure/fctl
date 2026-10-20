// The signed-in layout: a quiet sidebar, the page, and the command palette.

import { useQuery } from "@tanstack/react-query";
import { Link, Outlet, useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import {
  ChevronRight,
  ChevronsUpDown,
  Cpu,
  FolderPlus,
  Lock,
  LogOut,
  Moon,
  Plus,
  Search,
  Settings,
  Shield,
  Sun,
  X,
} from "lucide-react";
import { useCallback, useEffect, useState } from "react";
import {
  Avatar,
  HomeIcon,
  Kbd,
  Logo,
  Menu,
  MenuItem,
  MenuSeparator,
  StatusDot,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { forgetDraft } from "@/lib/draft";
import { useFeed } from "@/lib/feed";
import { useHotkey } from "@/lib/hotkeys";
import { forgetPush, onPushOpen, syncPush } from "@/lib/push";
import { keys, queryClient, useInstance, useMe, useProjects, useWorkspaces } from "@/lib/queries";
import { type Sortable, useSortable } from "@/lib/sortable";
import { useTheme } from "@/lib/theme";
import {
  groupProjects,
  reorderProjects,
  reorderWorkspaces,
  useFactorsWatch,
  useFolded,
} from "@/lib/workspaces";
import { CommandPalette } from "./command-palette";
import { MenuButton, ShellContext, useHeaderButtons } from "./menu-button";

export function AppShell() {
  const [palette, setPalette] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const openPalette = useCallback(() => setPalette(true), []);
  const openDrawer = useCallback(() => setDrawer(true), []);
  // A page whose header holds the menu button (the session's) takes the top bar's place.
  const [headerButton, shell] = useHeaderButtons(openDrawer);
  useHotkey("mod+k", openPalette, { inInputs: true });
  useFeed(); // live updates for every page
  usePush();
  useFactorsWatch();
  const path = useRouterState({ select: (s) => s.location.pathname });
  return (
    <div className="flex min-h-dvh bg-canvas">
      {/* On phones the sidebar is a drawer over the page: opaque, and out of the page once
          it has slid away (display: none), so that it is drawn anew each time it opens.
          Kept off screen with a blur, it sometimes stopped showing properly in Safari on iOS
          until a reload. */}
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-30 flex w-[calc(244px+env(safe-area-inset-left))] flex-col border-r border-line bg-surface pl-[env(safe-area-inset-left)] transition-[translate,display] transition-discrete duration-300 ease-out max-md:starting:-translate-x-full md:translate-x-0 md:bg-surface/80 md:backdrop-blur-xl",
          drawer ? "translate-x-0" : "-translate-x-full max-md:hidden",
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
      {/* The page is never wider than the window: what is wider scrolls in its own row. The
          clip, unlike hidden, keeps the sticky bars working. */}
      <div className="flex min-w-0 flex-1 flex-col overflow-x-clip pr-[env(safe-area-inset-right)] pl-[env(safe-area-inset-left)] md:pl-[calc(244px+env(safe-area-inset-left))]">
        {/* On phones: the menu button, its icon in line with the page's text. */}
        {!headerButton && (
          <div className="sticky top-0 z-10 flex h-14 items-center gap-1.5 border-b border-line bg-canvas/80 px-2.5 backdrop-blur-xl md:hidden">
            <MenuButton onClick={openDrawer} />
            <Logo large />
          </div>
        )}
        <main className="min-w-0 flex-1">
          <ShellContext value={shell}>
            <Outlet />
          </ShellContext>
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
  badge,
}: {
  to: string;
  icon: React.ReactNode;
  children: React.ReactNode;
  active: boolean;
  onClick: () => void;
  /** A count of what waits there, and what it counts. */
  badge?: { n: number; label: string };
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
      {badge && badge.n > 0 && (
        <span
          title={badge.label}
          className="ml-auto inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-highlight px-1 text-[10.5px] font-semibold text-highlight-ink tabular-nums"
        >
          {badge.n}
          <span className="sr-only">: {badge.label}</span>
        </span>
      )}
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
  const sort = useSortable({
    ids: groups.map((g) => g.workspace.id),
    onReorder: (ids) => reorderWorkspaces(ids, notSaved),
    label: (id) => groups.find((g) => g.workspace.id === id)?.workspace.name ?? "",
    describedBy: ORDER_HELP,
  });
  const [theme, setTheme] = useTheme();
  const navigate = useNavigate();
  const isAdmin = me?.user.role === "owner" || me?.user.role === "admin";
  // Who waits for an invitation, where the instance takes requests for early access.
  const asking =
    useQuery({
      queryKey: keys.accessRequests,
      queryFn: api.accessRequests,
      enabled: isAdmin && !!instance?.features.early_access,
      staleTime: 60_000,
    }).data?.waiting ?? 0;
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 items-center justify-between px-4">
        {/* On phones, as large as the top bar's logo. The link is a flex box, as tall as the
            logo, to keep it in the middle: as a line of text it would sit 3 px higher. */}
        <Link to="/" onClick={onNavigate} className="flex rounded-md">
          <Logo large />
        </Link>
        <button
          type="button"
          aria-label="Close menu"
          className="-mr-2 flex size-11 items-center justify-center rounded-lg text-ink-4 hover:text-ink md:hidden"
          onClick={onNavigate}
        >
          <X className="size-5" />
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
            sort={sort}
          />
        ))}
        <p id={ORDER_HELP} hidden>
          Drag to move it, or press Alt or Option with the up or down arrow.
        </p>
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
          <NavLink
            to="/admin"
            icon={<Shield />}
            active={path === "/admin"}
            onClick={onNavigate}
            badge={{
              n: asking,
              label:
                asking === 1 ? "1 person waits for early access" : `${asking} people wait for early access`,
            }}
          >
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

// The text that says how to move workspaces and projects, for screen readers.
const ORDER_HELP = "sidebar-order-help";

const notSaved = (e: unknown) =>
  toast(`The new order wasn't saved: ${e instanceof Error ? e.message : "try again"}`, "error");

// Rows people drag: a held finger lifts one instead of opening the link's menu.
const DRAGGABLE = "touch-manipulation select-none [-webkit-touch-callout:none]";
// A row or workspace lifted to be dragged, and settling into its place: a
// little larger, over the others.
const LIFTED =
  "relative z-10 scale-[1.02] bg-raised shadow-[inset_0_0_0_1px_var(--c-line-strong),0_16px_32px_-8px_rgb(0_0_0/0.55),0_3px_8px_-2px_rgb(0_0_0/0.22)]";

/** A workspace in the sidebar: its name (its page) and its projects, which fold away. */
function WorkspaceGroup({
  workspace,
  projects,
  path,
  folded,
  onFold,
  onNavigate,
  sort,
}: {
  workspace: G.Workspace;
  projects: G.Project[];
  path: string;
  folded: boolean;
  onFold: () => void;
  onNavigate: () => void;
  /** The sidebar's workspaces, which people drag by their headers. */
  sort: Sortable;
}) {
  const here = path === `/w/${workspace.slug}`;
  const active = projects.reduce((n, p) => n + p.active_sessions, 0);
  const sortProjects = useSortable({
    ids: projects.map((p) => p.id),
    onReorder: (ids) => reorderProjects(workspace.id, ids, notSaved),
    label: (id) => projects.find((p) => p.id === id)?.name ?? "",
    describedBy: ORDER_HELP,
  });
  const lifted = sort.lifted === workspace.id;
  // Its link, which gets the focus, says how to move it.
  const { "aria-describedby": describedBy, ...handle } = sort.handleProps(workspace.id);
  return (
    <section
      ref={sort.itemRef(workspace.id)}
      aria-label={workspace.name}
      className={cn(
        "rounded-md transition-[background-color,box-shadow,scale] duration-150 ease-[var(--ease-out)]",
        lifted && LIFTED,
      )}
    >
      <div
        {...handle}
        className={cn(
          "group flex h-8 items-center gap-1 rounded-md pr-1 transition-colors",
          DRAGGABLE,
          here ? "bg-raised shadow-[inset_0_0_0_1px_var(--c-line)]" : !lifted && "hover:bg-hover/60",
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
          aria-describedby={describedBy}
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
                  ref={sortProjects.itemRef(p.id)}
                  {...sortProjects.handleProps(p.id)}
                  to="/p/$slug"
                  params={{ slug: p.slug }}
                  search={{ tab: undefined }}
                  onClick={onNavigate}
                  className={cn(
                    "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-[color,background-color,box-shadow,scale] duration-150 ease-[var(--ease-out)]",
                    DRAGGABLE,
                    sortProjects.lifted === p.id
                      ? `${LIFTED} text-ink`
                      : path === `/p/${p.slug}`
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

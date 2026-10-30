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
  Kbd,
  Logo,
  Menu,
  MenuItem,
  MenuSeparator,
  StatusDot,
  statusLabel,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { forgetDraft } from "@/lib/draft";
import { useFeed } from "@/lib/feed";
import { forgetPush, onPushOpen, syncPush } from "@/lib/push";
import {
  keys,
  queryClient,
  useInstance,
  useLiveSessions,
  useMe,
  useProjects,
  useSessionSummary,
  useWorkspaces,
} from "@/lib/queries";
import { forgetSidebar, type SidebarProject, type SidebarWorkspace, useSidebarLists } from "@/lib/sidebar";
import { type Sortable, useSortable } from "@/lib/sortable";
import { useTheme } from "@/lib/theme";
import {
  createsProjects,
  groupProjects,
  reorderProjects,
  reorderWorkspaces,
  roomForAnother,
  useFactorsWatch,
  useFolded,
} from "@/lib/workspaces";
import { CommandPalette, usePaletteKeys } from "./command-palette";
import { FloorLink } from "./floor-link";
import { MenuButton, ShellContext, useHeaderButtons } from "./menu-button";

/**
 * The sidebar on phones: open, closed with its button or the shade (it slides away), or
 * gone, closed by one of its links or by an entry of the palette opened from it (at once).
 */
type Drawer = "open" | "closed" | "gone";

export function AppShell() {
  const [palette, setPalette] = useState(false);
  const [drawer, setDrawer] = useState<Drawer>("closed");
  const openPalette = useCallback(() => setPalette(true), []);
  const openDrawer = useCallback(() => setDrawer("open"), []);
  const closeDrawer = () => setDrawer("closed");
  const dropDrawer = () => setDrawer("gone");
  // A page whose header holds the menu button (the session's) takes the top bar's place.
  const [headerButton, shell] = useHeaderButtons(openDrawer);
  usePaletteKeys(openPalette);
  useFeed(); // live updates for every page
  usePush();
  useFactorsWatch();
  const path = useRouterState({ select: (s) => s.location.pathname });
  return (
    <div className="flex min-h-dvh bg-canvas">
      {/* On phones the sidebar is a drawer over the page: opaque, and out of the page once
          it has slid away (display: none), so that it is drawn anew each time it opens.
          Kept off screen with a blur, it sometimes stopped showing properly in Safari on iOS
          until a reload. A link in it, or an entry of the palette opened from it, takes it
          out at once, without the slide: the router's view transition snapshots the page as
          the link is followed, and a drawer that had only started to slide away showed there
          open, fading out over the page the link opened. */}
      <aside
        className={cn(
          "fixed top-(--viewport-shift) -bottom-(--viewport-shift) left-0 z-30 flex w-[calc(244px+env(safe-area-inset-left))] flex-col border-r border-line bg-surface pl-[env(safe-area-inset-left)] max-md:starting:-translate-x-full md:translate-x-0 md:bg-surface/80 md:backdrop-blur-xl",
          drawer !== "gone" && "transition-[translate,display] transition-discrete duration-300 ease-out",
          drawer === "open" ? "translate-x-0" : "-translate-x-full max-md:hidden",
        )}
      >
        <Sidebar onPalette={openPalette} onNavigate={dropDrawer} onClose={closeDrawer} path={path} />
      </aside>
      {drawer === "open" && (
        <button
          type="button"
          aria-label="Close menu"
          className="fixed inset-x-0 top-(--viewport-shift) -bottom-(--viewport-shift) z-20 bg-black/40 md:hidden"
          onClick={closeDrawer}
        />
      )}
      {/* The page is never wider than the window: what is wider scrolls in its own row. The
          clip, unlike hidden, keeps the sticky bars working. */}
      <div className="flex min-w-0 flex-1 flex-col overflow-x-clip pr-[env(safe-area-inset-right)] pl-[env(safe-area-inset-left)] md:pl-[calc(244px+env(safe-area-inset-left))]">
        {/* On phones: the menu button, its icon in line with the page's text. */}
        {!headerButton && (
          <div className="sticky top-(--viewport-shift) z-10 flex h-14 items-center gap-1.5 border-b border-line bg-canvas/80 px-2.5 backdrop-blur-xl md:hidden">
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
      <CommandPalette open={palette} onClose={() => setPalette(false)} onNavigate={dropDrawer} />
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

export function Sidebar({
  onPalette,
  onNavigate,
  onClose,
  path,
}: {
  onPalette: () => void;
  /** A link or menu item in it opens a page. */
  onNavigate: () => void;
  /** Its close button, which phones show, was pressed. */
  onClose: () => void;
  path: string;
}) {
  const { data: me } = useMe();
  const { data: instance } = useInstance();
  const { data: projectList } = useProjects();
  const { data: workspaceList } = useWorkspaces();
  // The lists this browser kept show at once, until the control plane's come.
  const { workspaces, projects, kept } = useSidebarLists(me?.user.id, workspaceList, projectList);
  const [folded, toggleFold] = useFolded();
  const groups = groupProjects(workspaces?.workspaces ?? [], projects ?? []);
  // On a project's page, and on its sessions' pages, the project's sessions that haven't ended
  // show under it, to switch between.
  const viewed = useSessionSummary(SESSION_PATH.exec(path)?.[1]).data;
  const opened = PROJECT_PATH.exec(path)?.[1];
  const liveProject = projects?.find((p) =>
    viewed ? p.id === viewed.project_id : p.slug === opened || p.id === opened,
  )?.id;
  const live = useLiveSessions(liveProject).data;
  // Each session at work lights a thread on the Agent Floor's row.
  const working = (projects ?? []).reduce((n, p) => n + p.active_sessions, 0);
  const sort = useSortable({
    // Kept lists don't move: a new order names the workspaces the control plane sent.
    ids: kept ? [] : groups.map((g) => g.workspace.id),
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
          onClick={onClose}
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
        <FloorLink active={path === "/"} working={working} onClick={onNavigate} />
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
              <span className="block truncate text-[11.5px] text-ink-3">Invited · join on Agent Floor</span>
            </span>
          </Link>
        ))}
        {groups.map(({ workspace, projects: list }) => (
          <WorkspaceGroup
            key={workspace.id}
            workspace={workspace}
            projects={list}
            path={path}
            loading={!projects}
            kept={kept}
            folded={folded.has(workspace.id)}
            onFold={() => toggleFold(workspace.id)}
            onNavigate={onNavigate}
            sort={sort}
            live={liveProject && live ? { project: liveProject, sessions: live } : undefined}
          />
        ))}
        <p id={ORDER_HELP} hidden>
          Drag to move it, or press Alt or Option with the up or down arrow.
        </p>
        {workspaces && roomForAnother(workspaces) && (
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
                    forgetSidebar(); // last, so that no list that comes is kept again
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

// A session's page, and the session's id; a project's page, and the project's slug (or id).
const SESSION_PATH = /^\/s\/([^/]+)$/;
const PROJECT_PATH = /^\/p\/([^/]+)$/;

/** The sessions the sidebar shows under a project. */
interface LiveSessions {
  project: string;
  sessions: G.Session[];
}

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
  loading,
  kept,
  folded,
  onFold,
  onNavigate,
  sort,
  live,
}: {
  workspace: SidebarWorkspace;
  projects: SidebarProject[];
  path: string;
  /** Its projects haven't come yet. */
  loading: boolean;
  /** The workspace and its projects are as this browser kept them. */
  kept: boolean;
  folded: boolean;
  onFold: () => void;
  onNavigate: () => void;
  /** The sidebar's workspaces, which people drag by their headers. */
  sort: Sortable;
  /** The sessions to show under a project, which may be one of these. */
  live?: LiveSessions;
}) {
  const here = path === `/w/${workspace.slug}`;
  const active = projects.reduce((n, p) => n + p.active_sessions, 0);
  const sortProjects = useSortable({
    ids: kept ? [] : projects.map((p) => p.id),
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
        {createsProjects(workspace) && (
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
                <ProjectRow
                  key={p.id}
                  project={p}
                  path={path}
                  sort={sortProjects}
                  sessions={live?.project === p.id ? live.sessions : undefined}
                  onNavigate={onNavigate}
                />
              ))}
              {projects.length === 0 && !loading && createsProjects(workspace) && (
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

/**
 * A project in the sidebar, which people drag by its row (the sessions under it move with it): a
 * dot lit while its sessions are at work, and how many are.
 */
function ProjectRow({
  project: p,
  path,
  sort,
  sessions,
  onNavigate,
}: {
  project: SidebarProject;
  path: string;
  /** Its workspace's projects. */
  sort: Sortable;
  /** Its sessions to show under it. */
  sessions?: G.Session[];
  onNavigate: () => void;
}) {
  const lifted = sort.lifted === p.id;
  return (
    <div
      ref={sort.itemRef(p.id)}
      className={cn(
        "rounded-md transition-[background-color,box-shadow,scale] duration-150 ease-[var(--ease-out)]",
        lifted && LIFTED,
      )}
    >
      <Link
        {...sort.handleProps(p.id)}
        to="/p/$slug"
        params={{ slug: p.slug }}
        search={{ tab: undefined }}
        onClick={onNavigate}
        className={cn(
          "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-[color,background-color,box-shadow] duration-150 ease-[var(--ease-out)]",
          DRAGGABLE,
          path === `/p/${p.slug}`
            ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
            : lifted
              ? "text-ink"
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
      {sessions && sessions.length > 0 && (
        <ProjectSessions
          project={p}
          sessions={sessions}
          path={path}
          lifted={lifted}
          onNavigate={onNavigate}
        />
      )}
    </div>
  );
}

/**
 * A project's sessions under it in the sidebar, the one whose page shows marked: a line runs down
 * beside them from the project's dot.
 */
function ProjectSessions({
  project,
  sessions,
  path,
  lifted,
  onNavigate,
}: {
  project: SidebarProject;
  sessions: G.Session[];
  path: string;
  /** The project is being dragged, with them. */
  lifted: boolean;
  onNavigate: () => void;
}) {
  return (
    <ul
      aria-label={`${project.name}'s active sessions`}
      className="relative mt-0.5 flex flex-col gap-0.5 pl-[26px] before:absolute before:-top-2.5 before:bottom-1 before:left-[17.5px] before:w-px before:bg-line"
    >
      {sessions.map((s) => {
        const here = path === `/s/${s.id}`;
        return (
          <li key={s.id}>
            <Link
              to="/s/$id"
              params={{ id: s.id }}
              onClick={onNavigate}
              title={`${s.title} · ${statusLabel(s.status)}`}
              className={cn(
                "flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors duration-150",
                here ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]" : "text-ink-3",
                !here && !lifted && "hover:bg-hover/60 hover:text-ink",
              )}
            >
              <StatusDot status={s.status} className="scale-75" />
              <span className="min-w-0 flex-1 truncate">{s.title}</span>
              <span className="sr-only">: {statusLabel(s.status)}</span>
            </Link>
          </li>
        );
      })}
    </ul>
  );
}

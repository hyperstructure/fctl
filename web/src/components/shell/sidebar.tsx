// The sidebar, as a tree: the Agent Floor, what needs you and the pull requests ready for review
// at the top; then each workspace with its projects, each with its icon, its sessions at work and
// those that ended in the last hour; for the project a page is about, its own pull requests too.
// People drag workspaces and projects into their order, drag the sidebar wider, and fold it into a
// rail of icons (⌘\).

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  ChevronRight,
  CircleAlert,
  CirclePause,
  FolderPlus,
  Inbox,
  Lock,
  MessageCircleQuestion,
  MessageSquareReply,
  Palette,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Search,
  Settings,
  Shield,
  X,
} from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import { IconPicker } from "@/components/project/icon-picker";
import { ProjectIcon, ProjectMark } from "@/components/project/project-icon";
import { AgentFloorIcon, Logo, PullRequestIcon, StatusDot, toast } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { useAttention } from "@/lib/attention";
import { cn } from "@/lib/cn";
import { agoShort } from "@/lib/format";
import { apple } from "@/lib/hotkeys";
import { type Need, needSays, needsOf } from "@/lib/needs";
import { iconOf, useSaveIcon } from "@/lib/project-icons";
import { listPulls, PULLS_QUERY, needOf as pullNeed, stateOf } from "@/lib/pull-list";
import { usePullsMine } from "@/lib/pulls-view";
import {
  keys,
  queryClient,
  recentSessions,
  useInstance,
  useMe,
  useProjects,
  useSessions,
  useSidebarSessions,
  useWorkspaces,
  useYourSessions,
} from "@/lib/queries";
import { type SidebarProject, type SidebarWorkspace, useSidebarLists } from "@/lib/sidebar";
import { foldKeys } from "@/lib/sidebar-layout";
import { type Sortable, useSortable } from "@/lib/sortable";
import {
  createsProjects,
  groupProjects,
  reorderProjects,
  reorderWorkspaces,
  roomForAnother,
  useFolded,
} from "@/lib/workspaces";
import { FloorLink } from "./floor-link";
import {
  Count,
  SearchButton,
  SessionLine,
  SettingsRows,
  UserMenu,
  useMinute,
  waitingForAccess,
} from "./sidebar-parts";

export interface SidebarProps {
  onPalette: () => void;
  /** A link or menu item in it opens a page. */
  onNavigate: () => void;
  /** Its close button, which phones show, was pressed. */
  onClose: () => void;
  path: string;
  /** The page's tab, on a project's page: its own links light up. */
  tab?: string;
  /** On the pull requests page, the workspace or project it lists (`w:slug`, `p:slug`). */
  scope?: string;
  /** Folded into its rail (wide screens only), and how to fold or unfold it. */
  folded?: boolean;
  onFold?: (folded: boolean) => void;
}

export function Sidebar(props: SidebarProps) {
  const data = useSidebarData(props.path, props.scope);
  useAttention(data.needs.length);
  return props.folded ? <Rail {...props} data={data} /> : <Tree {...props} data={data} />;
}

type Data = ReturnType<typeof useSidebarData>;

function useSidebarData(path: string, scope?: string) {
  const { data: me } = useMe();
  const { data: instance } = useInstance();
  const { data: projectList } = useProjects();
  const { data: workspaceList } = useWorkspaces();
  // The lists this browser kept show at once, until the control plane's come.
  const { workspaces, projects, kept } = useSidebarLists(me?.user.id, workspaceList, projectList);
  const groups = groupProjects(workspaces?.workspaces ?? [], projects ?? []);
  const now = useMinute();
  // The Agent Floor's own lists (the same requests, when it is open too): what needs you comes from
  // them, with the sessions under the projects, the ones at work and those that ended in the hour.
  const active = useSessions({ status: "active", limit: 50 }).data?.sessions;
  const ended = useSessions({ status: "ended", limit: 12 }).data?.sessions;
  const listed = useSidebarSessions().data?.sessions;
  // The pull requests page's own list, unnarrowed, which what needs you looks through too.
  const open = useSessions(PULLS_QUERY).data?.sessions;
  // The counts are the page's, from its own request: with Mine on there, the one for the sessions you
  // started, which the server narrows before its limit.
  const [mine] = usePullsMine();
  const yourPulls = useSessions({ ...PULLS_QUERY, mine: true }, { enabled: mine }).data?.sessions;
  // Every session they started or follow from the last day, which the others may not reach.
  const yours = useYourSessions(!!me).data?.sessions;
  const recent = useMemo(() => byProject(recentSessions(listed ?? [], now)), [listed, now]);
  const needs = useMemo(
    () =>
      needsOf(
        me?.user.id,
        [...(yours ?? []), ...(listed ?? []), ...(active ?? []), ...(ended ?? []), ...(open ?? [])],
        now,
      ),
    [me?.user.id, yours, listed, active, ended, open, now],
  );
  // The open pull requests, each once: those ready for review, and each project's.
  const pulls = useMemo(
    () => listPulls((mine ? yourPulls : open) ?? []).filter((l) => stateOf(l) === "open"),
    [open, yourPulls, mine],
  );
  const full = useMemo(() => new Map((projectList?.projects ?? []).map((p) => [p.id, p])), [projectList]);
  const isAdmin = me?.user.role === "owner" || me?.user.role === "admin";
  // Who waits for an invitation, where the instance takes requests for early access.
  const asking =
    useQuery({
      queryKey: keys.accessRequests,
      queryFn: api.accessRequests,
      enabled: isAdmin && !!instance?.features.early_access,
      staleTime: 60_000,
    }).data?.waiting ?? 0;
  // Each session at work lights a thread on the Agent Floor's row.
  const working = (projects ?? []).reduce((n, p) => n + p.active_sessions, 0);
  const session = path.startsWith("/s/") ? path.slice(3) : undefined;
  // The session page's own, which the sidebar only reads, for one no list here has.
  const opened = useQuery({
    queryKey: keys.session(session ?? ""),
    queryFn: () => api.session(session ?? ""),
    enabled: false,
  }).data;
  const current =
    [...(listed ?? []), ...(active ?? []), ...(ended ?? []), ...(yours ?? [])].find(
      (s) => s.id === session,
    ) ?? (opened?.id === session ? opened : undefined);
  const scoped = scope?.startsWith("p:") ? scope.slice(2) : undefined;
  const here = path.startsWith("/p/")
    ? projects?.find((p) => p.slug === path.slice(3))
    : scoped
      ? projects?.find((p) => p.slug === scoped)
      : current && projects?.find((p) => p.id === current.project_id);
  return {
    me: me ?? undefined,
    instance,
    workspaces,
    projects,
    kept,
    groups,
    full,
    recent,
    needs,
    pulls,
    ready: pulls.filter((l) => pullNeed(l) === "ready"),
    /** Whether `pulls` are only those of sessions you started (Mine, on the pull requests page). */
    mine,
    isAdmin,
    asking,
    working,
    /** The project the page is about: its own, or its session's. */
    here,
    session,
    now,
  };
}

/** Sessions by their project's id, in the order they come. */
function byProject(sessions: G.Session[]): Map<string, G.Session[]> {
  const by = new Map<string, G.Session[]>();
  for (const s of sessions) {
    const list = by.get(s.project_id);
    if (list) list.push(s);
    else by.set(s.project_id, [s]);
  }
  return by;
}

const notSaved = (e: unknown) =>
  toast(`The new order wasn't saved: ${e instanceof Error ? e.message : "try again"}`, "error");

// Rows people drag: a held finger lifts one instead of opening the link's menu.
const DRAGGABLE = "touch-manipulation select-none [-webkit-touch-callout:none]";
// A row or workspace lifted to be dragged, and settling into its place: a little larger, over the
// others.
const LIFTED =
  "relative z-10 scale-[1.02] bg-raised shadow-[inset_0_0_0_1px_var(--c-line-strong),0_16px_32px_-8px_rgb(0_0_0/0.55),0_3px_8px_-2px_rgb(0_0_0/0.22)]";
// What the page is about.
const HERE = "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]";
// The text that says how to move workspaces and projects, for screen readers.
const ORDER_HELP = "sidebar-order-help";

function Tree({ onPalette, onNavigate, onClose, onFold, path, tab, data }: SidebarProps & { data: Data }) {
  const [folded, toggleFold] = useFolded();
  const [needsOpen, setNeedsOpen] = useNeedsOpen();
  // The page lists every pull request, not one project's (whose own link lights up then).
  const allPulls = path === "/pulls" && !data.here;
  const sort = useSortable({
    // Kept lists don't move: a new order names the workspaces the control plane sent.
    ids: data.kept ? [] : data.groups.map((g) => g.workspace.id),
    onReorder: (ids) => reorderWorkspaces(ids, notSaved),
    label: (id) => data.groups.find((g) => g.workspace.id === id)?.workspace.name ?? "",
    describedBy: ORDER_HELP,
  });
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 shrink-0 items-center justify-between pr-2 pl-4">
        {/* On phones, as large as the top bar's logo. */}
        <Link to="/" onClick={onNavigate} className="flex rounded-md">
          <Logo large />
        </Link>
        {onFold && (
          <button
            type="button"
            aria-label="Fold the sidebar"
            title={`Fold the sidebar (${foldKeys()})`}
            onClick={() => onFold(true)}
            className="flex size-8 items-center justify-center rounded-md text-ink-4 transition-colors hover:bg-hover hover:text-ink max-md:hidden"
          >
            <PanelLeftClose className="size-4" />
          </button>
        )}
        <button
          type="button"
          aria-label="Close menu"
          className="-mr-1 flex size-11 items-center justify-center rounded-lg text-ink-4 hover:text-ink md:hidden"
          onClick={onClose}
        >
          <X className="size-5" />
        </button>
      </div>
      <div className="px-3">
        <SearchButton onPalette={onPalette} />
      </div>
      <nav aria-label="Views" className="mt-3 flex flex-col gap-px px-3">
        <FloorLink active={path === "/"} working={data.working} onClick={onNavigate} />
        <NeedsYou
          needs={data.needs}
          open={needsOpen}
          onToggle={() => setNeedsOpen(!needsOpen)}
          session={data.session}
          now={data.now}
          projects={data.projects}
          onNavigate={onNavigate}
        />
        <Link
          to="/pulls"
          search={{}}
          onClick={onNavigate}
          aria-current={allPulls ? "page" : undefined}
          title={`${
            data.ready.length
              ? `${data.ready.length} ready for review, of ${data.pulls.length} open`
              : `${data.pulls.length} open`
          }${data.mine ? " from your sessions" : ""}`}
          className={cn(
            "group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
            allPulls ? HERE : "text-ink-3 hover:bg-hover hover:text-ink",
          )}
        >
          <PullRequestIcon
            state="open"
            className={cn("size-4", allPulls ? "text-accent" : "text-ink-4 group-hover:text-ink-3")}
          />
          <span className="flex-1">Pull requests</span>
          {data.ready.length > 0 ? (
            <span className="text-[11.5px] text-pr-open tabular-nums">{data.ready.length}</span>
          ) : (
            data.pulls.length > 0 && (
              <span className="text-[11.5px] text-ink-4 tabular-nums">{data.pulls.length}</span>
            )
          )}
        </Link>
      </nav>
      <nav
        aria-label="Workspaces"
        className="mt-4 flex min-h-0 flex-1 flex-col gap-4 overflow-y-auto px-3 pb-4"
      >
        {data.workspaces?.invitations.map((i) => (
          <Link
            key={i.id}
            to="/"
            onClick={onNavigate}
            className="group flex items-center gap-2.5 rounded-lg border border-accent/25 bg-accent-soft px-2.5 py-2 text-[12.5px] text-ink-2 transition-colors hover:border-accent/45"
          >
            <WorkspaceAvatar workspace={i.workspace} size={20} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-ink">{i.workspace.name}</span>
              <span className="block truncate text-[11.5px] text-ink-3">Invited · join on Agent Floor</span>
            </span>
          </Link>
        ))}
        {data.groups.map(({ workspace, projects }) => (
          <WorkspaceGroup
            key={workspace.id}
            workspace={workspace}
            projects={projects}
            data={data}
            path={path}
            tab={tab}
            folded={folded.has(workspace.id)}
            onFold={() => toggleFold(workspace.id)}
            onNavigate={onNavigate}
            sort={sort}
          />
        ))}
        <p id={ORDER_HELP} hidden>
          Drag to move it, or press Alt or Option with the up or down arrow.
        </p>
        {data.workspaces && roomForAnother(data.workspaces) && (
          <Link
            to="/w/new"
            onClick={onNavigate}
            className={cn(
              "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] transition-colors",
              data.workspaces.workspaces.length === 0
                ? "text-ink-2 hover:bg-hover hover:text-ink"
                : "text-ink-4 hover:bg-hover hover:text-ink-2",
            )}
          >
            <Plus className="size-3.5" />
            {data.workspaces.workspaces.length === 0 ? "Create a workspace" : "New workspace"}
          </Link>
        )}
      </nav>
      <div className="flex flex-col gap-0.5 border-t border-line px-3 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <SettingsRows isAdmin={data.isAdmin} asking={data.asking} path={path} onNavigate={onNavigate} />
        <div className="mt-1">
          <UserMenu me={data.me} instance={data.instance} onNavigate={onNavigate} />
        </div>
      </div>
    </div>
  );
}

const NEEDS_OPEN = "fctl:needs-open";

/** Whether "Needs you" is open, kept in this browser: open until someone closes it. */
function useNeedsOpen(): [boolean, (open: boolean) => void] {
  const [open, setOpen] = useState(() => {
    try {
      return localStorage.getItem(NEEDS_OPEN) !== "0";
    } catch {
      return true;
    }
  });
  return [
    open,
    (o: boolean) => {
      setOpen(o);
      try {
        localStorage.setItem(NEEDS_OPEN, o ? "1" : "0");
      } catch {}
    },
  ];
}

/** What needs you, which opens into its list in place: each with what it needs, where, and since when. */
function NeedsYou({
  needs,
  open,
  onToggle,
  session,
  now,
  projects,
  onNavigate,
}: {
  needs: Need[];
  open: boolean;
  onToggle: () => void;
  session?: string;
  now: number;
  projects?: SidebarProject[];
  onNavigate: () => void;
}) {
  const n = needs.length;
  const asks = needs.filter((x) => x.kind === "asking").length;
  return (
    <div>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open && n > 0}
        disabled={n === 0}
        className={cn(
          "group flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[13.5px] transition-colors duration-150",
          n === 0 ? "text-ink-4" : "text-ink-3 hover:bg-hover hover:text-ink",
        )}
      >
        <Inbox className={cn("size-4", n > 0 ? "text-blue" : "text-ink-4")} />
        <span className="flex-1">Needs you</span>
        {n > 0 && (
          <span
            title={asks ? `${asks} ${asks === 1 ? "asks" : "ask"} you something` : undefined}
            className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-blue px-1.5 text-[10.5px] font-semibold text-white tabular-nums"
          >
            {n}
          </span>
        )}
        {n > 0 && (
          <ChevronRight
            className={cn(
              "size-3 text-ink-4 transition-transform duration-200 ease-[var(--ease-out)] motion-reduce:transition-none",
              open && "rotate-90",
            )}
          />
        )}
      </button>
      {/* Closed, its links are out of the way of Tab and screen readers too. */}
      <div
        inert={!(open && n > 0)}
        className={cn(
          "grid transition-[grid-template-rows] duration-200 ease-[var(--ease-out)] motion-reduce:transition-none",
          open && n > 0 ? "grid-rows-[1fr]" : "grid-rows-[0fr]",
        )}
      >
        <div className="min-h-0 overflow-hidden">
          {n > 0 && (
            <ul
              aria-label="What needs you"
              className="mt-0.5 mb-1 flex flex-col gap-px rounded-lg bg-sunken/50 p-1 shadow-[inset_0_0_0_1px_var(--c-line)]"
            >
              {needs.map((x) => (
                <li key={x.s.id}>
                  <NeedLink
                    need={x}
                    here={session === x.s.id}
                    project={projects?.find((p) => p.id === x.s.project_id)}
                    now={now}
                    onNavigate={onNavigate}
                  />
                </li>
              ))}
            </ul>
          )}
        </div>
      </div>
    </div>
  );
}

const NEED_LOOKS: Record<Need["kind"], { text: string; icon: ReactNode }> = {
  asking: { text: "text-blue", icon: <MessageCircleQuestion className="size-3" /> },
  failed: { text: "text-danger", icon: <CircleAlert className="size-3" /> },
  checks: { text: "text-danger", icon: <CircleAlert className="size-3" /> },
  ready: { text: "text-pr-open", icon: <PullRequestIcon state="open" className="size-3" /> },
  waiting: { text: "text-blue", icon: <MessageSquareReply className="size-3" /> },
  paused: { text: "text-pink", icon: <CirclePause className="size-3" /> },
};

/**
 * Something that needs you: its session and since when, then what it needs. A question gets two
 * lines, since it is what you answer; the rest one.
 */
function NeedLink({
  need,
  here,
  project,
  now,
  onNavigate,
}: {
  need: Need;
  here: boolean;
  project?: SidebarProject;
  now: number;
  onNavigate: () => void;
}) {
  const look = NEED_LOOKS[need.kind];
  const says = needSays(need);
  const s = need.s;
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      onClick={onNavigate}
      title={`${s.title}\n${says}`}
      className={cn(
        "group flex items-start gap-2.5 rounded-md px-2 py-1.5 transition-colors duration-150",
        here ? HERE : "hover:bg-hover",
      )}
    >
      <ProjectMark
        project={project ?? { id: s.project_id, name: s.project_slug }}
        tile={26}
        className="mt-[5px]"
      />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-2">
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[13px] leading-5",
              here ? "text-ink" : "text-ink-2 group-hover:text-ink",
            )}
          >
            {s.title}
          </span>
          {need.at !== undefined && (
            <span className="shrink-0 text-[11px] text-ink-4 tabular-nums">{agoShort(need.at, now)}</span>
          )}
        </span>
        <span className={cn("flex min-w-0 items-start gap-1 text-[11.5px] leading-4", look.text)}>
          <span className="mt-0.5 shrink-0">{look.icon}</span>
          <span className={need.kind === "asking" ? "line-clamp-2 break-words" : "truncate"}>{says}</span>
        </span>
      </span>
    </Link>
  );
}

/** A workspace: its avatar and name (its page), and its projects, which fold away. */
function WorkspaceGroup({
  workspace,
  projects,
  data,
  path,
  tab,
  folded,
  onFold,
  onNavigate,
  sort,
}: {
  workspace: SidebarWorkspace;
  projects: SidebarProject[];
  data: Data;
  path: string;
  tab?: string;
  folded: boolean;
  onFold: () => void;
  onNavigate: () => void;
  /** The sidebar's workspaces, which people drag by their headers. */
  sort: Sortable;
}) {
  const here = path === `/w/${workspace.slug}`;
  const atWork = projects.reduce((n, p) => n + p.active_sessions, 0);
  const sortProjects = useSortable({
    ids: data.kept ? [] : projects.map((p) => p.id),
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
          here ? HERE : !lifted && "hover:bg-hover",
        )}
      >
        <Link
          to="/w/$slug"
          params={{ slug: workspace.slug }}
          search={{ tab: undefined }}
          onClick={onNavigate}
          aria-describedby={describedBy}
          className="flex min-w-0 flex-1 items-center gap-2.5 py-1 pl-2"
        >
          <WorkspaceAvatar workspace={workspace} size={20} title={false} />
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[13px] font-semibold",
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
            folded && (
              <Count n={atWork} label={atWork === 1 ? "1 session at work" : `${atWork} sessions at work`} />
            )
          )}
        </Link>
        {createsProjects(workspace) && (
          <Link
            to="/new"
            search={{ workspace: workspace.slug }}
            onClick={onNavigate}
            aria-label={`New project in ${workspace.name}`}
            title="New project"
            className="flex size-6 items-center justify-center rounded text-ink-4 opacity-0 transition-[opacity,color] group-hover:opacity-100 hover:text-ink focus-visible:opacity-100"
          >
            <Plus className="size-3.5" />
          </Link>
        )}
        <button
          type="button"
          onClick={onFold}
          aria-expanded={!folded}
          aria-label={folded ? `Show ${workspace.name}'s projects` : `Hide ${workspace.name}'s projects`}
          className={cn(
            "flex size-6 items-center justify-center rounded text-ink-4 transition-[opacity,color] hover:text-ink-2",
            folded ? "opacity-100" : "opacity-0 group-hover:opacity-100 focus-visible:opacity-100",
          )}
        >
          <ChevronRight
            className={cn(
              "size-3.5 transition-transform duration-200 ease-[var(--ease-out)] motion-reduce:transition-none",
              !folded && "rotate-90",
            )}
          />
        </button>
      </div>
      {!folded && (
        <div className="mt-0.5 flex flex-col gap-px">
          {workspace.blocked ? (
            <Link
              to="/settings"
              search={{ linked: undefined, error: undefined }}
              onClick={onNavigate}
              className="rounded-md px-2.5 py-1.5 pl-[38px] text-[12.5px] leading-snug text-ink-3 hover:text-ink"
            >
              Set up two-factor sign-in to open it
            </Link>
          ) : (
            <>
              {projects.map((p) => (
                <ProjectRow
                  key={p.id}
                  project={p}
                  data={data}
                  path={path}
                  tab={tab}
                  sort={sortProjects}
                  onNavigate={onNavigate}
                />
              ))}
              {projects.length === 0 && !!data.projects && createsProjects(workspace) && (
                <Link
                  to="/new"
                  search={{ workspace: workspace.slug }}
                  onClick={onNavigate}
                  className="flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] text-ink-4 hover:bg-hover hover:text-ink"
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
 * A project, which people drag by its row (its sessions move with it): its icon, its name and its
 * sessions at work. The one the page is about opens into its own links, its pull requests, above its
 * sessions. Its maintainers change its icon from the row.
 */
function ProjectRow({
  project: p,
  data,
  path,
  tab,
  sort,
  onNavigate,
}: {
  project: SidebarProject;
  data: Data;
  path: string;
  tab?: string;
  /** Its workspace's projects. */
  sort: Sortable;
  onNavigate: () => void;
}) {
  const [picking, setPicking] = useState(false);
  const full = data.full.get(p.id);
  const manage = full?.role === "owner" || full?.role === "maintainer";
  const sessions = data.recent.get(p.id) ?? [];
  const open = data.here?.id === p.id;
  const page = path === `/p/${p.slug}`;
  const pulls = data.pulls.filter((l) => l.sessions.some((s) => s.project_id === p.id)).length;
  const lifted = sort.lifted === p.id;
  return (
    <div
      ref={sort.itemRef(p.id)}
      className={cn(
        "rounded-md transition-[background-color,box-shadow,scale] duration-150 ease-[var(--ease-out)]",
        lifted && LIFTED,
      )}
    >
      <div className="group relative flex items-center">
        <Link
          {...sort.handleProps(p.id)}
          to="/p/$slug"
          params={{ slug: p.slug }}
          search={{ tab: undefined }}
          onClick={onNavigate}
          className={cn(
            "flex h-8 min-w-0 flex-1 items-center gap-2.5 rounded-md pl-[11px] text-[13.5px] transition-[color,background-color,box-shadow] duration-150 ease-[var(--ease-out)]",
            DRAGGABLE,
            page && !tab
              ? HERE
              : lifted
                ? "text-ink"
                : open
                  ? "text-ink hover:bg-hover"
                  : "text-ink-3 hover:bg-hover hover:text-ink",
          )}
        >
          <ProjectMark project={full ?? p} size={16} />
          <span className="min-w-0 flex-1 truncate">{p.name}</span>
          {p.active_sessions > 0 && (
            <span
              title={p.active_sessions === 1 ? "1 session at work" : `${p.active_sessions} sessions at work`}
              className={cn(
                "flex items-center gap-1.5 pr-2.5 text-[11.5px] text-accent tabular-nums",
                manage && "transition-opacity group-hover:opacity-0",
              )}
            >
              <StatusDot status="running" className="scale-75" />
              {p.active_sessions}
            </span>
          )}
        </Link>
        {manage && full && (
          <button
            type="button"
            onClick={() => setPicking(true)}
            aria-label={`Change ${p.name}'s icon`}
            title="Change its icon"
            className="absolute right-1 flex size-6 items-center justify-center rounded text-ink-4 opacity-0 transition-[opacity,color,background-color] group-hover:opacity-100 hover:bg-hover hover:text-ink focus-visible:opacity-100"
          >
            <Palette className="size-3.5" />
          </button>
        )}
      </div>
      {(open || sessions.length > 0) && (
        <div className="relative mt-px mb-1 pl-[30px] before:absolute before:top-0 before:bottom-1.5 before:left-[18.5px] before:w-px before:bg-line">
          {open && (
            <div className="flex flex-col gap-px">
              <SubLink
                to="/pulls"
                search={{ scope: `p:${p.slug}` }}
                active={path === "/pulls"}
                icon={<PullRequestIcon state="open" className="size-3.5" />}
                label="Pull requests"
                n={pulls}
                onNavigate={onNavigate}
              />
              {sessions.length > 0 && <div className="mx-2 my-1 h-px bg-line" />}
            </div>
          )}
          {sessions.length > 0 && (
            <ul aria-label={`${p.name}'s recent sessions`} className="flex flex-col gap-px">
              {sessions.map((s) => (
                <li key={s.id}>
                  <SessionLine s={s} here={path === `/s/${s.id}`} lifted={lifted} onNavigate={onNavigate} />
                </li>
              ))}
            </ul>
          )}
        </div>
      )}
      {full && <ProjectIconPicker project={full} open={picking} onClose={() => setPicking(false)} />}
    </div>
  );
}

/** The picker for a project's icon, which saves it. */
function ProjectIconPicker({
  project,
  open,
  onClose,
}: {
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const save = useSaveIcon(project);
  if (!open) return null;
  return (
    <IconPicker
      open
      onClose={onClose}
      project={project}
      saving={save.isPending}
      onSave={(icon) =>
        save.mutate(icon, {
          onSuccess: () => {
            onClose();
            void queryClient.invalidateQueries({ queryKey: keys.projects });
          },
          onError: (e) => toast(`The icon wasn't saved: ${e.message}`, "error"),
        })
      }
    />
  );
}

/** One of the open project's own links: its pull requests, or its tasks. */
function SubLink({
  to,
  search,
  active,
  icon,
  label,
  n,
  onNavigate,
}: {
  to: string;
  search: Record<string, string>;
  active: boolean;
  icon: ReactNode;
  label: string;
  n?: number;
  onNavigate: () => void;
}) {
  return (
    <Link
      to={to}
      search={search}
      onClick={onNavigate}
      aria-current={active ? "page" : undefined}
      className={cn(
        "flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors",
        active ? HERE : "text-ink-3 hover:bg-hover hover:text-ink",
      )}
    >
      <span className="flex w-3.5 justify-center text-ink-4">{icon}</span>
      <span className="flex-1">{label}</span>
      {n !== undefined && n > 0 && <span className="text-[11.5px] text-ink-4 tabular-nums">{n}</span>}
    </Link>
  );
}

/** Folded: a rail of the workspaces' avatars and their projects' icons, each lit by its work. */
function Rail({ onPalette, onNavigate, onFold, path, data }: SidebarProps & { data: Data }) {
  return (
    <div className="flex h-full flex-col items-center">
      <div className="flex h-14 shrink-0 items-center">
        <Link to="/" onClick={onNavigate} className="flex rounded-md" aria-label="fctl">
          <Logo withWord={false} />
        </Link>
      </div>
      <RailButton label={`Search or jump to… (${apple() ? "⌘" : "Ctrl+"}K)`} onClick={onPalette}>
        <Search className="size-4" />
      </RailButton>
      <RailLink
        to="/"
        label="Agent Floor"
        active={path === "/"}
        lit={data.working > 0}
        onNavigate={onNavigate}
      >
        <AgentFloorIcon className="size-4" live />
      </RailLink>
      <RailLink
        to="/pulls"
        label={data.ready.length ? `Pull requests · ${data.ready.length} ready for review` : "Pull requests"}
        active={path === "/pulls"}
        onNavigate={onNavigate}
        badge={data.ready.length}
      >
        <PullRequestIcon state="open" className="size-4" />
      </RailLink>
      {data.needs.length > 0 && (
        <span
          className="mt-1 mb-1 flex h-5 min-w-5 items-center justify-center rounded-full bg-blue px-1.5 text-[10.5px] font-semibold text-white tabular-nums"
          title={`${data.needs.length} ${data.needs.length === 1 ? "thing needs" : "things need"} you`}
        >
          {data.needs.length}
        </span>
      )}
      <div className="mt-2 flex min-h-0 w-full flex-1 flex-col items-center gap-1 overflow-y-auto pb-3">
        {data.groups.map(({ workspace, projects }) => (
          <div
            key={workspace.id}
            className="flex w-full flex-col items-center gap-1 border-t border-line pt-3 pb-1 first:border-t-0"
          >
            <Link
              to="/w/$slug"
              params={{ slug: workspace.slug }}
              search={{ tab: undefined }}
              onClick={onNavigate}
              title={workspace.name}
              aria-label={workspace.name}
              className="mb-1 rounded-md"
            >
              <WorkspaceAvatar workspace={workspace} size={22} title={false} />
            </Link>
            {projects.map((p) => {
              const asks = data.needs.some((x) => x.kind === "asking" && x.s.project_id === p.id);
              return (
                <Link
                  key={p.id}
                  to="/p/$slug"
                  params={{ slug: p.slug }}
                  search={{ tab: undefined }}
                  onClick={onNavigate}
                  title={p.active_sessions ? `${p.name} · ${p.active_sessions} at work` : p.name}
                  aria-label={p.name}
                  className={cn(
                    "relative flex size-9 items-center justify-center rounded-lg transition-colors",
                    data.here?.id === p.id
                      ? "bg-raised shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                      : "hover:bg-hover",
                  )}
                >
                  <ProjectIcon icon={iconOf(data.full.get(p.id) ?? p)} tile={26} />
                  {(asks || p.active_sessions > 0) && (
                    <span className="absolute top-0.5 right-0.5 flex size-2.5 items-center justify-center rounded-full bg-surface">
                      {asks ? (
                        <span className="size-1.5 rounded-full bg-blue" />
                      ) : (
                        <StatusDot status="running" className="scale-[0.7]" />
                      )}
                    </span>
                  )}
                </Link>
              );
            })}
          </div>
        ))}
      </div>
      <div className="flex w-full flex-col items-center gap-1 border-t border-line pt-2 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        {data.isAdmin && (
          <RailLink
            to="/admin"
            label={data.asking ? `Admin · ${waitingForAccess(data.asking)}` : "Admin"}
            active={path === "/admin"}
            badge={data.asking}
            tone="highlight"
            onNavigate={onNavigate}
          >
            <Shield className="size-4" />
          </RailLink>
        )}
        <RailLink to="/settings" label="Settings" active={path === "/settings"} onNavigate={onNavigate}>
          <Settings className="size-4" />
        </RailLink>
        {onFold && (
          <RailButton label={`Unfold the sidebar (${foldKeys()})`} onClick={() => onFold(false)}>
            <PanelLeftOpen className="size-4" />
          </RailButton>
        )}
        <UserMenu me={data.me} instance={data.instance} onNavigate={onNavigate} compact />
      </div>
    </div>
  );
}

function RailButton({
  label,
  onClick,
  children,
}: {
  label: string;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={onClick}
      className="flex size-9 items-center justify-center rounded-lg text-ink-4 transition-colors hover:bg-hover hover:text-ink"
    >
      {children}
    </button>
  );
}

function RailLink({
  to,
  label,
  active,
  lit,
  badge,
  tone = "pr",
  onNavigate,
  children,
}: {
  to: "/" | "/pulls" | "/admin" | "/settings";
  label: string;
  active: boolean;
  lit?: boolean;
  badge?: number;
  /** The badge's color: pull requests' green, or the highlight of people waiting. */
  tone?: "pr" | "highlight";
  onNavigate: () => void;
  children: ReactNode;
}) {
  return (
    <Link
      to={to}
      search={{}}
      onClick={onNavigate}
      aria-label={label}
      aria-current={active ? "page" : undefined}
      title={label}
      className={cn(
        "relative flex size-9 items-center justify-center rounded-lg transition-colors",
        active
          ? "bg-raised text-accent shadow-[inset_0_0_0_1px_var(--c-line)]"
          : lit
            ? "text-accent hover:bg-hover"
            : "text-ink-4 hover:bg-hover hover:text-ink",
      )}
    >
      {children}
      {!!badge && (
        <span
          className={cn(
            "absolute -top-0.5 -right-0.5 flex h-3.5 min-w-3.5 items-center justify-center rounded-full px-1 text-[9px] font-semibold tabular-nums",
            tone === "pr" ? "bg-pr-open text-white" : "bg-highlight text-highlight-ink",
          )}
        >
          {badge}
        </span>
      )}
    </Link>
  );
}

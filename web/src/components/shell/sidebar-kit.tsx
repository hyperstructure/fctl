// What the proposed sidebars share (a design prototype, lib/sidebar-design.ts): their data, and
// the parts they all show: the search, a session's line, counts, the person's menu and the
// handle that makes a sidebar wider or narrower.

import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  ChevronsUpDown,
  CircleAlert,
  CirclePause,
  Cpu,
  LogOut,
  MessageSquareReply,
  Moon,
  Search,
  Settings,
  Shield,
  Sun,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { PullRequestChecksIcon, PullRequestStateIcon } from "@/components/session/pull-request";
import {
  Avatar,
  Kbd,
  Menu,
  MenuItem,
  MenuSeparator,
  PullRequestIcon,
  StatusDot,
  statusLabel,
} from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { forgetDraft } from "@/lib/draft";
import { agoShort } from "@/lib/format";
import { checksMark } from "@/lib/pulls";
import { forgetPush } from "@/lib/push";
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
} from "@/lib/queries";
import { ACTIVE } from "@/lib/sessions";
import {
  forgetSidebar,
  type SidebarProject,
  type SidebarWorkspace,
  type SidebarWorkspaces,
  useSidebarLists,
} from "@/lib/sidebar";
import { waitsForYou } from "@/lib/status";
import { useTheme } from "@/lib/theme";
import { groupProjects, type WorkspaceGroup } from "@/lib/workspaces";

/** What a page's address says about where it is: a workspace's, a project's or a session's page. */
export interface Place {
  path: string;
  /** The page's tab, on a workspace's or a project's page. */
  tab?: string;
  workspace?: string;
  project?: string;
  session?: string;
}

export function placeOf(path: string, tab?: string): Place {
  const [, kind, id] = path.split("/");
  if (kind === "w" && id && id !== "new") return { path, tab, workspace: id };
  if (kind === "p" && id) return { path, tab, project: id };
  if (kind === "s" && id) return { path, session: id };
  return { path };
}

/** A project of the lists by its id, for a session's: its name as people gave it. */
export const projectOf = (data: NavData, id: string) => data.projects?.find((p) => p.id === id);

/** A pull request a session found, with the session. */
export interface Pull {
  pr: G.PullRequestRef;
  session: G.Session;
}

export interface NavData {
  me?: G.Me;
  instance?: G.InstanceInfo;
  workspaces?: SidebarWorkspaces;
  projects?: SidebarProject[];
  /** The full projects, once they came: descriptions, repositories, counts. */
  full: Map<string, G.Project>;
  kept: boolean;
  groups: WorkspaceGroup<SidebarWorkspace, SidebarProject>[];
  /** The sessions to show under each project, by its id: at work, or ended in the last hour. */
  recent: Map<string, G.Session[]>;
  /** Sessions that haven't ended, everyone's you can see. */
  active: G.Session[];
  /** Of those, the ones waiting for a person's reply. */
  waiting: G.Session[];
  /** Of those, the ones at work. */
  working: G.Session[];
  /** Open and draft pull requests the sessions above and those that ended lately found, newest session first. */
  pulls: Pull[];
  /** What needs the person signed in, most pressing first. */
  needs: Need[];
  isAdmin: boolean;
  /** People waiting for early access, for instance admins. */
  asking: number;
  /** The session whose page is open, when the lists have it. */
  current?: G.Session;
}

/** The time, again every minute: a session leaves the sidebar an hour after it ended. */
export function useMinute(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(t);
  }, []);
  return now;
}

const OPEN = new Set(["open", "draft", "queued"]);

export function useNavData(place: Place): NavData {
  const { data: me } = useMe();
  const { data: instance } = useInstance();
  const { data: projectList } = useProjects();
  const { data: workspaceList } = useWorkspaces();
  const { workspaces, projects, kept } = useSidebarLists(me?.user.id, workspaceList, projectList);
  const groups = groupProjects(workspaces?.workspaces ?? [], projects ?? []);
  const now = useMinute();
  const listed = useSidebarSessions().data?.sessions;
  // The Agent Floor's own lists, of sessions at work and of those that ended lately: the same
  // requests, when it is open too.
  const activeList = useSessions({ status: "active", limit: 50 }).data?.sessions;
  const endedList = useSessions({ status: "ended", limit: 12 }).data?.sessions;
  const recent = useMemo(() => byProject(recentSessions(listed ?? [], now)), [listed, now]);
  const active = useMemo(() => (activeList ?? []).filter((s) => ACTIVE.includes(s.status)), [activeList]);
  const full = useMemo(() => new Map((projectList?.projects ?? []).map((p) => [p.id, p])), [projectList]);
  const isAdmin = me?.user.role === "owner" || me?.user.role === "admin";
  const asking =
    useQuery({
      queryKey: keys.accessRequests,
      queryFn: api.accessRequests,
      enabled: isAdmin && !!instance?.features.early_access,
      staleTime: 60_000,
    }).data?.waiting ?? 0;
  const pulls = useMemo(() => {
    const seen = new Set<string>();
    const out: Pull[] = [];
    const all = [...(listed ?? []), ...active, ...(endedList ?? [])].sort(
      (a, b) => b.created_at - a.created_at,
    );
    for (const s of all)
      for (const pr of s.pull_requests ?? (s.pull_request ? [s.pull_request] : [])) {
        if (seen.has(pr.url) || !OPEN.has(pr.state ?? "open")) continue;
        seen.add(pr.url);
        out.push({ pr, session: s });
      }
    return out;
  }, [listed, active, endedList]);
  const current = place.session
    ? [...(listed ?? []), ...active].find((s) => s.id === place.session)
    : undefined;
  const waiting = active.filter(waitsForYou);
  const needs = useMemo(
    () => needsOf(me?.user.id, waiting, active, [...(listed ?? []), ...(endedList ?? [])], pulls, now),
    [me?.user.id, waiting, active, listed, endedList, pulls, now],
  );
  return {
    me: me ?? undefined,
    instance,
    workspaces,
    projects,
    full,
    kept,
    groups,
    recent,
    active,
    waiting,
    working: active.filter((s) => s.status === "running" || (s.status === "idle" && !waitsForYou(s))),
    pulls,
    needs,
    isAdmin,
    asking,
    current,
  };
}

/** Something that needs the person, and why. */
export interface Need {
  s: G.Session;
  why: string;
  tone: "waiting" | "danger" | "paused" | "ready";
  /** When it came to need them. */
  at?: number;
  pr?: G.PullRequestRef;
}

const RANK: Record<Need["tone"], number> = { danger: 0, waiting: 1, ready: 2, paused: 3 };

/**
 * What needs `me`, in sessions they started or follow: one that failed in the last day, one whose
 * pull request's checks failed, one that waits for their reply, a pull request its agent finished
 * (the session ended, the pull request is open and not a draft), and one paused on a limit.
 */
function needsOf(
  me: string | undefined,
  waiting: G.Session[],
  active: G.Session[],
  ended: G.Session[],
  pulls: Pull[],
  now: number,
): Need[] {
  const theirs = (s: G.Session) => !me || s.created_by.id === me || !!s.following;
  const out: Need[] = [];
  const seen = new Set<string>();
  const add = (n: Need) => {
    if (seen.has(n.s.id) || !theirs(n.s)) return;
    seen.add(n.s.id);
    out.push(n);
  };
  for (const s of ended)
    if (s.status === "failed" && (s.ended_at ?? 0) > now - 86_400_000)
      add({ s, why: "Failed", tone: "danger", at: s.ended_at });
  for (const p of pulls)
    if (checksMark(p.pr) === "failed")
      add({
        s: p.session,
        why: `Checks failed on #${p.pr.number}`,
        tone: "danger",
        at: p.pr.updated_at,
        pr: p.pr,
      });
  for (const s of waiting) add({ s, why: "Waiting for your reply", tone: "waiting", at: s.last_activity_at });
  for (const p of pulls)
    if (p.session.status === "stopped" && (p.pr.state ?? "open") === "open")
      add({
        s: p.session,
        why: `#${p.pr.number} is ready for you`,
        tone: "ready",
        at: p.session.ended_at,
        pr: p.pr,
      });
  for (const s of active)
    if (s.status === "paused") add({ s, why: "Paused", tone: "paused", at: s.last_activity_at });
  return out.sort((a, b) => RANK[a.tone] - RANK[b.tone] || (b.at ?? 0) - (a.at ?? 0));
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

/** The project a page is about: its own, or its session's. */
export function projectHere(place: Place, data: NavData): SidebarProject | undefined {
  if (place.project) return data.projects?.find((p) => p.slug === place.project);
  if (data.current) return data.projects?.find((p) => p.id === data.current?.project_id);
  return undefined;
}

/** The workspace a page is about: its own, its project's or its session's. */
export function workspaceHere(place: Place, data: NavData): SidebarWorkspace | undefined {
  const all = data.workspaces?.workspaces ?? [];
  if (place.workspace) return all.find((w) => w.slug === place.workspace);
  const p = projectHere(place, data);
  return p?.workspace ? all.find((w) => w.id === p.workspace?.id) : undefined;
}

/** The search box that opens the command palette. */
export function SearchButton({ onPalette, className }: { onPalette: () => void; className?: string }) {
  return (
    <button
      type="button"
      onClick={onPalette}
      className={cn(
        "flex h-8.5 w-full items-center gap-2.5 rounded-lg border border-line bg-sunken/60 px-2.5 text-[13px] text-ink-4 transition-colors hover:border-line-strong hover:text-ink-3",
        className,
      )}
    >
      <Search className="size-3.5" />
      <span className="flex-1 text-left">Search or jump to…</span>
      <Kbd>⌘K</Kbd>
    </button>
  );
}

/** A count beside a row: yellow with a live dot for work going on, blue for what waits for you. */
export function Count({
  n,
  tone = "plain",
  label,
}: {
  n: number;
  tone?: "plain" | "live" | "waiting" | "pull";
  label?: string;
}) {
  if (n <= 0) return null;
  return (
    <span
      title={label}
      className={cn(
        "ml-auto inline-flex shrink-0 items-center gap-1.5 text-[11.5px] tabular-nums",
        tone === "live" ? "text-accent" : tone === "waiting" ? "text-blue" : "text-ink-4",
      )}
    >
      {tone === "live" && (
        <span className="relative inline-flex size-1.5">
          <span className="absolute inset-0 animate-ping rounded-full bg-accent opacity-60 [animation-duration:2s]" />
          <span className="relative inline-flex size-1.5 rounded-full bg-accent" />
        </span>
      )}
      {n}
      {label && <span className="sr-only">: {label}</span>}
    </span>
  );
}

/** A pill for a count that wants attention: blue for replies, yellow for work. */
export function Badge({ n, tone, label }: { n: number; tone: "waiting" | "live" | "plain"; label?: string }) {
  if (n <= 0) return null;
  return (
    <span
      title={label}
      className={cn(
        "ml-auto inline-flex h-[18px] min-w-[18px] shrink-0 items-center justify-center rounded-full px-1.5 text-[10.5px] font-semibold tabular-nums",
        tone === "waiting"
          ? "bg-blue text-white"
          : tone === "live"
            ? "bg-highlight text-highlight-ink"
            : "bg-hover text-ink-3",
      )}
    >
      {n}
      {label && <span className="sr-only">: {label}</span>}
    </span>
  );
}

/** A session's pull request, as its state and checks. */
export function PullMark({ s }: { s: G.Session }) {
  const pr = s.pull_request;
  if (!pr) return null;
  return (
    <span
      className="flex shrink-0 items-center gap-0.5"
      title={`#${pr.number}${pr.title ? ` · ${pr.title}` : ""}`}
    >
      <PullRequestStateIcon pr={pr} className="size-3" />
      <PullRequestChecksIcon pr={pr} className="size-3" />
    </span>
  );
}

/** A session in a sidebar: its status, its title and its pull request. */
export function SessionLink({
  s,
  here,
  onNavigate,
  className,
  lead,
  trail,
}: {
  s: G.Session;
  here: boolean;
  onNavigate: () => void;
  className?: string;
  /** In place of the status dot. */
  lead?: ReactNode;
  /** After the title, in place of the pull request's marks. */
  trail?: ReactNode;
}) {
  const ended = s.status === "stopped" || s.status === "failed";
  const waits = waitsForYou(s);
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      onClick={onNavigate}
      title={`${s.title} · ${statusLabel(s.status)}`}
      className={cn(
        "group/s flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors duration-150",
        here
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : ended
            ? "text-ink-4 hover:bg-hover hover:text-ink-2"
            : waits
              ? "text-ink-2 hover:bg-hover hover:text-ink"
              : "text-ink-3 hover:bg-hover hover:text-ink",
        className,
      )}
    >
      {lead ?? <StatusDot status={s.status} className="scale-75" />}
      <span className="min-w-0 flex-1 truncate">{s.title}</span>
      <span className="sr-only">: {statusLabel(s.status)}</span>
      {trail ?? <PullMark s={s} />}
    </Link>
  );
}

/** Admin (for instance admins) and Settings, as rows. */
export function SettingsRows({
  data,
  path,
  onNavigate,
}: {
  data: NavData;
  path: string;
  onNavigate: () => void;
}) {
  return (
    <>
      {data.isAdmin && (
        <BottomRow to="/admin" icon={<Shield />} active={path === "/admin"} onClick={onNavigate}>
          Admin
          <Count
            n={data.asking}
            label={
              data.asking === 1
                ? "1 person waits for early access"
                : `${data.asking} people wait for early access`
            }
          />
        </BottomRow>
      )}
      <BottomRow to="/settings" icon={<Settings />} active={path === "/settings"} onClick={onNavigate}>
        Settings
      </BottomRow>
    </>
  );
}

function BottomRow({
  to,
  icon,
  active,
  onClick,
  children,
}: {
  to: string;
  icon: ReactNode;
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <Link
      to={to}
      onClick={onClick}
      className={cn(
        "group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
        active
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "text-ink-3 hover:bg-hover hover:text-ink",
      )}
    >
      <span className={cn("[&_svg]:size-4", active ? "text-accent" : "text-ink-4 group-hover:text-ink-3")}>
        {icon}
      </span>
      {children}
    </Link>
  );
}

/** The person signed in, and their menu: the theme, Claude and tokens, signing out. */
export function UserMenu({
  data,
  onNavigate,
  compact,
}: {
  data: NavData;
  onNavigate: () => void;
  /** Their picture alone, as a rail shows it. */
  compact?: boolean;
}) {
  const [theme, setTheme] = useTheme();
  const navigate = useNavigate();
  const me = data.me;
  if (!me) return null;
  return (
    <Menu
      side="top"
      align={compact ? "start" : "stretch"}
      trigger={(toggle, open) => (
        <button
          type="button"
          onClick={toggle}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={compact ? me.user.name : undefined}
          title={compact ? me.user.name : undefined}
          className={cn(
            "group flex items-center gap-2.5 rounded-md text-left transition-colors hover:bg-hover aria-expanded:bg-hover",
            compact ? "size-10 justify-center rounded-xl" : "w-full px-2 py-1.5",
          )}
        >
          <Avatar user={me.user} size={compact ? 28 : 24} />
          {!compact && (
            <>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] text-ink">{me.user.name}</span>
                <span className="block truncate text-[11.5px] text-ink-4">
                  {data.instance?.name ?? "fctl"}
                </span>
              </span>
              <ChevronsUpDown className="size-3.5 shrink-0 text-ink-4 transition-colors group-hover:text-ink-3" />
            </>
          )}
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
              forgetSidebar();
              window.location.assign("/login");
            }}
          >
            Sign out
          </MenuItem>
        </>
      )}
    </Menu>
  );
}

/**
 * The handle on a sidebar's right edge: drag it, or use the arrow keys, to make the sidebar wider
 * or narrower; a double click puts it back.
 */
export function ResizeHandle({
  width,
  min,
  max,
  onResize,
  onDragging,
}: {
  width: number;
  min: number;
  max: number;
  /** A new width, or none for the initial one. */
  onResize: (w: number | null) => void;
  onDragging: (on: boolean) => void;
}) {
  const start = useRef<{ x: number; w: number } | null>(null);
  if (min === max) return null;
  return (
    // biome-ignore lint/a11y/useSemanticElements: a separator that moves is a focusable widget, not an <hr>.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Sidebar width"
      aria-valuenow={width}
      aria-valuemin={min}
      aria-valuemax={max}
      tabIndex={0}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { x: e.clientX, w: width };
        onDragging(true);
      }}
      onPointerMove={(e) => {
        if (start.current) onResize(start.current.w + e.clientX - start.current.x);
      }}
      onPointerUp={() => {
        start.current = null;
        onDragging(false);
      }}
      onDoubleClick={() => onResize(null)}
      onKeyDown={(e) => {
        if (e.key === "ArrowLeft") onResize(width - 16);
        else if (e.key === "ArrowRight") onResize(width + 16);
        else return;
        e.preventDefault();
      }}
      className="group/handle absolute inset-y-0 -right-1.5 z-10 hidden w-3 cursor-col-resize touch-none md:block"
    >
      <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 bg-transparent transition-colors duration-150 group-hover/handle:bg-line-strong group-focus-visible/handle:bg-accent group-active/handle:bg-accent" />
    </div>
  );
}

const TONES: Record<Need["tone"], { text: string; icon: ReactNode }> = {
  waiting: { text: "text-blue", icon: <MessageSquareReply className="size-3" /> },
  danger: { text: "text-danger", icon: <CircleAlert className="size-3" /> },
  paused: { text: "text-pink", icon: <CirclePause className="size-3" /> },
  ready: { text: "text-pr-open", icon: <PullRequestIcon state="open" className="size-3" /> },
};

/** Something that needs the person, on two lines: its session's title, and why, where and since when. */
export function NeedLink({
  need: { s, why, tone, at },
  here,
  icon,
  project,
  now,
  onNavigate,
}: {
  need: Need;
  here: boolean;
  icon: ReactNode;
  project: string;
  now: number;
  onNavigate: () => void;
}) {
  const t = TONES[tone];
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      onClick={onNavigate}
      title={`${s.title} · ${why}`}
      className={cn(
        "group flex items-center gap-2.5 rounded-lg px-2 py-1.5 transition-colors duration-150",
        here ? "bg-raised shadow-[inset_0_0_0_1px_var(--c-line)]" : "hover:bg-hover",
      )}
    >
      {icon}
      <span className="min-w-0 flex-1">
        <span
          className={cn(
            "block truncate text-[13px] leading-5",
            here ? "text-ink" : "text-ink-2 group-hover:text-ink",
          )}
        >
          {s.title}
        </span>
        <span className="flex min-w-0 items-center gap-1 text-[11.5px] leading-4">
          <span className={cn("flex shrink-0 items-center gap-1", t.text)}>
            {t.icon}
            {why}
          </span>
          <span className="truncate text-ink-4">
            · {project}
            {at ? ` · ${agoShort(at, now)}` : ""}
          </span>
        </span>
      </span>
    </Link>
  );
}

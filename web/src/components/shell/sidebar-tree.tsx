// The tree (a design proposal, lib/sidebar-design.ts): today's sidebar grown up. Wider, every
// project with its icon, what waits for you and the open pull requests at the top (each opens
// into its list), and the project you are in opened into its own links: its pull requests and
// its tasks, then its sessions. Folded, it is a rail of workspaces and project icons.

import { Link } from "@tanstack/react-router";
import {
  ChevronRight,
  Ellipsis,
  FolderPlus,
  Inbox,
  ListTodo,
  Lock,
  PanelLeftClose,
  PanelLeftOpen,
  Plus,
  Search,
  X,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { IconPicker } from "@/components/project/icon-picker";
import { ProjectIcon, useProjectIcons } from "@/components/project/project-icon";
import { PullRequestChecksIcon, PullRequestStateIcon } from "@/components/session/pull-request";
import { AgentFloorIcon, Logo, PullRequestIcon, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { apple } from "@/lib/hotkeys";
import type { SidebarProject, SidebarWorkspace } from "@/lib/sidebar";
import { createsProjects, roomForAnother, useFolded } from "@/lib/workspaces";
import { FloorLink } from "./floor-link";
import {
  Count,
  type NavData,
  NeedLink,
  type Place,
  type Pull,
  placeOf,
  projectHere,
  projectOf,
  SearchButton,
  SessionLink,
  SettingsRows,
  UserMenu,
  useMinute,
  useNavData,
} from "./sidebar-kit";

export interface SidebarProps {
  onPalette: () => void;
  onNavigate: () => void;
  onClose: () => void;
  path: string;
  /** The page's tab, on a workspace's or a project's page. */
  tab?: string;
  /** Folded into its rail (wide screens), and how to fold or unfold it. */
  folded: boolean;
  onFold: (folded: boolean) => void;
}

export function TreeSidebar(props: SidebarProps) {
  const place = placeOf(props.path, props.tab);
  const data = useNavData(place);
  if (props.folded) return <TreeRail {...props} data={data} place={place} />;
  return <Tree {...props} data={data} place={place} />;
}

function Tree({
  onPalette,
  onNavigate,
  onClose,
  onFold,
  data,
  place,
}: SidebarProps & { data: NavData; place: Place }) {
  const [folded, toggleFold] = useFolded();
  const iconFor = useProjectIcons();
  const [open, setOpen] = useState<"waiting" | "pulls" | null>(null);
  const now = useMinute();
  const here = projectHere(place, data);
  const [picking, setPicking] = useState<SidebarProject>();
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 shrink-0 items-center justify-between pr-2 pl-4">
        <Link to="/" onClick={onNavigate} className="flex rounded-md">
          <Logo large />
        </Link>
        <button
          type="button"
          aria-label="Fold the sidebar"
          title={`Fold the sidebar (${apple() ? "⌘" : "Ctrl+"}\\)`}
          onClick={() => onFold(true)}
          className="flex size-8 items-center justify-center rounded-md text-ink-4 transition-colors hover:bg-hover hover:text-ink max-md:hidden"
        >
          <PanelLeftClose className="size-4" />
        </button>
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
        <FloorLink active={place.path === "/"} working={data.working.length} onClick={onNavigate} />
        <Expander
          icon={<Inbox />}
          label="Needs you"
          n={data.needs.length}
          tone="waiting"
          open={open === "waiting"}
          onToggle={() => setOpen(open === "waiting" ? null : "waiting")}
        >
          {data.needs.map((n) => (
            <NeedLink
              key={n.s.id}
              need={n}
              here={place.session === n.s.id}
              icon={
                <ProjectIcon
                  icon={iconFor(
                    projectOf(data, n.s.project_id) ?? { id: n.s.project_id, name: n.s.project_slug },
                  )}
                  tile={26}
                />
              }
              project={projectName(data, n.s)}
              now={now}
              onNavigate={onNavigate}
            />
          ))}
        </Expander>
        <Expander
          icon={<PullRequestIcon state="open" className="size-4" />}
          label="Pull requests"
          n={data.pulls.length}
          open={open === "pulls"}
          onToggle={() => setOpen(open === "pulls" ? null : "pulls")}
        >
          {data.pulls.map((p) => (
            <PullLink key={p.pr.url} pull={p} here={place.session === p.session.id} onNavigate={onNavigate} />
          ))}
        </Expander>
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
            className="flex items-center gap-2.5 rounded-lg border border-accent/25 bg-accent-soft px-2.5 py-2 text-[12.5px] text-ink-2 transition-colors hover:border-accent/45"
          >
            <WorkspaceAvatar workspace={i.workspace} size={20} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-ink">{i.workspace.name}</span>
              <span className="block truncate text-[11.5px] text-ink-3">Invited · join on Agent Floor</span>
            </span>
          </Link>
        ))}
        {data.groups.map(({ workspace, projects }) => (
          <TreeWorkspace
            key={workspace.id}
            workspace={workspace}
            projects={projects}
            data={data}
            place={place}
            here={here}
            folded={folded.has(workspace.id)}
            onFold={() => toggleFold(workspace.id)}
            onNavigate={onNavigate}
            onPick={setPicking}
          />
        ))}
        {data.workspaces && roomForAnother(data.workspaces) && (
          <Link
            to="/w/new"
            onClick={onNavigate}
            className="flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] text-ink-4 transition-colors hover:bg-hover hover:text-ink-2"
          >
            <Plus className="size-3.5" />
            New workspace
          </Link>
        )}
      </nav>
      <div className="flex flex-col gap-0.5 border-t border-line px-3 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <SettingsRows data={data} path={place.path} onNavigate={onNavigate} />
        <div className="mt-1">
          <UserMenu data={data} onNavigate={onNavigate} />
        </div>
      </div>
      {picking && (
        <IconPicker
          open
          project={{ ...picking, description: data.full.get(picking.id)?.description }}
          onClose={() => setPicking(undefined)}
        />
      )}
    </div>
  );
}

const projectName = (data: NavData, s: G.Session) => projectOf(data, s.project_id)?.name ?? s.project_slug;

/** A view at the top that opens into its list in place. */
export function Expander({
  icon,
  label,
  n,
  tone = "plain",
  open,
  onToggle,
  children,
}: {
  icon: ReactNode;
  label: string;
  n: number;
  tone?: "plain" | "waiting";
  open: boolean;
  onToggle: () => void;
  children: ReactNode;
}) {
  return (
    <div>
      <button
        type="button"
        onClick={onToggle}
        aria-expanded={open}
        disabled={n === 0}
        className={cn(
          "group flex h-8 w-full items-center gap-2.5 rounded-md px-2.5 text-left text-[13.5px] transition-colors duration-150",
          n === 0 ? "text-ink-4" : "text-ink-3 hover:bg-hover hover:text-ink",
          open && "text-ink",
        )}
      >
        <span
          className={cn(
            "[&_svg]:size-4",
            n > 0 && tone === "waiting" ? "text-blue" : "text-ink-4 group-hover:text-ink-3",
          )}
        >
          {icon}
        </span>
        <span className="flex-1">{label}</span>
        {n > 0 &&
          (tone === "waiting" ? (
            <span className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-blue px-1.5 text-[10.5px] font-semibold text-white tabular-nums">
              {n}
            </span>
          ) : (
            <span className="text-[11.5px] text-ink-4 tabular-nums">{n}</span>
          ))}
        {n > 0 && (
          <ChevronRight
            className={cn(
              "size-3 text-ink-4 transition-transform duration-200 ease-[var(--ease-out)] motion-reduce:transition-none",
              open && "rotate-90",
            )}
          />
        )}
      </button>
      <div
        className={cn(
          "grid transition-[grid-template-rows] duration-200 ease-[var(--ease-out)] motion-reduce:transition-none",
          open ? "grid-rows-[1fr]" : "grid-rows-[0fr]",
        )}
      >
        <div className="min-h-0 overflow-hidden">
          <div className="mt-0.5 mb-1 flex flex-col gap-px rounded-lg bg-sunken/50 p-1 shadow-[inset_0_0_0_1px_var(--c-line)]">
            {children}
          </div>
        </div>
      </div>
    </div>
  );
}

/** An open pull request, as its state, its number and title, and its checks. */
export function PullLink({
  pull: { pr, session },
  here,
  onNavigate,
}: {
  pull: Pull;
  here: boolean;
  onNavigate: () => void;
}) {
  return (
    <Link
      to="/s/$id"
      params={{ id: session.id }}
      onClick={onNavigate}
      title={pr.title ?? session.title}
      className={cn(
        "flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors",
        here ? "bg-raised text-ink" : "text-ink-3 hover:bg-hover hover:text-ink",
      )}
    >
      <PullRequestStateIcon pr={pr} className="size-3.5" />
      <span className="min-w-0 flex-1 truncate">{pr.title ?? session.title}</span>
      <span className="font-mono text-[11px] text-ink-4">#{pr.number}</span>
      <PullRequestChecksIcon pr={pr} className="size-3" />
    </Link>
  );
}

function TreeWorkspace({
  workspace,
  projects,
  data,
  place,
  here,
  folded,
  onFold,
  onNavigate,
  onPick,
}: {
  workspace: SidebarWorkspace;
  projects: SidebarProject[];
  data: NavData;
  place: Place;
  /** The project the page is about. */
  here?: SidebarProject;
  folded: boolean;
  onFold: () => void;
  onNavigate: () => void;
  onPick: (p: SidebarProject) => void;
}) {
  const atWork = projects.reduce((n, p) => n + p.active_sessions, 0);
  const current = place.workspace === workspace.slug;
  return (
    <section aria-label={workspace.name}>
      <div
        className={cn(
          "group flex h-8 items-center gap-1 rounded-md pr-1 transition-colors",
          current ? "bg-raised shadow-[inset_0_0_0_1px_var(--c-line)]" : "hover:bg-hover",
        )}
      >
        <Link
          to="/w/$slug"
          params={{ slug: workspace.slug }}
          search={{ tab: undefined }}
          onClick={onNavigate}
          className="flex min-w-0 flex-1 items-center gap-2.5 py-1 pl-2"
        >
          <WorkspaceAvatar workspace={workspace} size={20} title={false} />
          <span
            className={cn(
              "min-w-0 flex-1 truncate text-[13px] font-semibold",
              current ? "text-ink" : "text-ink-2 group-hover:text-ink",
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
            folded && <Count n={atWork} tone="live" label="sessions at work" />
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
                <TreeProject
                  key={p.id}
                  project={p}
                  data={data}
                  place={place}
                  open={here?.id === p.id}
                  onNavigate={onNavigate}
                  onPick={() => onPick(p)}
                />
              ))}
              {projects.length === 0 && createsProjects(workspace) && (
                <Link
                  to="/new"
                  search={{ workspace: workspace.slug }}
                  onClick={onNavigate}
                  className="flex h-8 items-center gap-2.5 rounded-md px-2.5 pl-[38px] text-[13px] text-ink-4 hover:text-ink"
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
 * A project: its icon, its name and its sessions at work. The one the page is about opens into its
 * own links, its pull requests and its tasks, above its sessions.
 */
export function TreeProject({
  project: p,
  data,
  place,
  open,
  onNavigate,
  onPick,
}: {
  project: SidebarProject;
  data: NavData;
  place: Place;
  open: boolean;
  onNavigate: () => void;
  onPick: () => void;
}) {
  const iconFor = useProjectIcons();
  const icon = iconFor(p);
  const sessions = data.recent.get(p.id) ?? [];
  const pulls = data.pulls.filter((x) => x.session.project_id === p.id);
  const page = place.project === p.slug;
  const tab = page ? place.tab : undefined;
  return (
    <div>
      <div
        className={cn(
          "group relative flex h-8 items-center rounded-md transition-colors duration-150",
          page && !tab ? "bg-raised shadow-[inset_0_0_0_1px_var(--c-line)]" : "hover:bg-hover",
        )}
      >
        <Link
          to="/p/$slug"
          params={{ slug: p.slug }}
          search={{ tab: undefined }}
          onClick={onNavigate}
          onContextMenu={(e) => {
            e.preventDefault();
            onPick();
          }}
          className={cn(
            "flex h-8 min-w-0 flex-1 items-center gap-2.5 pl-[11px] text-[13.5px]",
            page || open ? "text-ink" : "text-ink-3 group-hover:text-ink",
          )}
        >
          <ProjectIcon icon={icon} size={16} />
          <span className="min-w-0 flex-1 truncate">{p.name}</span>
          <span className="pr-2.5 transition-opacity group-hover:opacity-0">
            <Count n={p.active_sessions} tone="live" label="sessions at work" />
          </span>
        </Link>
        <button
          type="button"
          onClick={onPick}
          aria-label={`Change ${p.name}'s icon`}
          title="Change its icon"
          className="absolute right-1 flex size-6 items-center justify-center rounded text-ink-4 opacity-0 transition-[opacity,color] group-hover:opacity-100 hover:text-ink focus-visible:opacity-100"
        >
          <Ellipsis className="size-3.5" />
        </button>
      </div>
      {(open || sessions.length > 0) && (
        <div className="relative mt-px mb-1 flex flex-col gap-px pl-[30px] before:absolute before:top-0 before:bottom-1.5 before:left-[18.5px] before:w-px before:bg-line">
          {open && (
            <>
              <SubLink
                to="/p/$slug"
                slug={p.slug}
                tab="reviews"
                active={page && tab === "reviews"}
                icon={<PullRequestIcon state="open" className="size-3.5" />}
                label="Pull requests"
                n={pulls.length}
                onNavigate={onNavigate}
              />
              <SubLink
                to="/p/$slug"
                slug={p.slug}
                tab="tickets"
                active={page && tab === "tickets"}
                icon={<ListTodo className="size-3.5" />}
                label="Tasks"
                onNavigate={onNavigate}
              />
              {sessions.length > 0 && <div className="mx-2 my-1 h-px bg-line" />}
            </>
          )}
          {sessions.map((s) => (
            <SessionLink key={s.id} s={s} here={place.session === s.id} onNavigate={onNavigate} />
          ))}
        </div>
      )}
    </div>
  );
}

function SubLink({
  slug,
  tab,
  active,
  icon,
  label,
  n,
  onNavigate,
}: {
  to: "/p/$slug";
  slug: string;
  tab: "reviews" | "tickets";
  active: boolean;
  icon: ReactNode;
  label: string;
  n?: number;
  onNavigate: () => void;
}) {
  return (
    <Link
      to="/p/$slug"
      params={{ slug }}
      search={{ tab }}
      onClick={onNavigate}
      className={cn(
        "flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors",
        active
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "text-ink-3 hover:bg-hover hover:text-ink",
      )}
    >
      <span className="flex w-3.5 justify-center text-ink-4">{icon}</span>
      <span className="flex-1">{label}</span>
      {n !== undefined && n > 0 && <span className="text-[11.5px] text-ink-4 tabular-nums">{n}</span>}
    </Link>
  );
}

/** Folded: a rail of the workspaces' avatars and their projects' icons, each with its work lit. */
export function TreeRail({
  onPalette,
  onNavigate,
  onFold,
  data,
  place,
}: SidebarProps & { data: NavData; place: Place }) {
  const iconFor = useProjectIcons();
  const here = projectHere(place, data);
  return (
    <div className="flex h-full flex-col items-center">
      <div className="flex h-14 shrink-0 items-center">
        <Link to="/" onClick={onNavigate} className="flex rounded-md" aria-label="fctl">
          <Logo withWord={false} />
        </Link>
      </div>
      <RailButton label="Search or jump to… (⌘K)" onClick={onPalette}>
        <Search className="size-4" />
      </RailButton>
      <RailLink
        to="/"
        label="Agent Floor"
        active={place.path === "/"}
        onNavigate={onNavigate}
        lit={data.working.length > 0}
      >
        <AgentFloorIcon className="size-4" live />
      </RailLink>
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
            {projects.map((p) => (
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
                  here?.id === p.id
                    ? "bg-raised shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                    : "hover:bg-hover",
                )}
              >
                <ProjectIcon icon={iconFor(p)} tile={26} />
                {p.active_sessions > 0 && (
                  <span className="absolute top-0.5 right-0.5 flex size-2.5 items-center justify-center rounded-full bg-surface">
                    <StatusDot status="running" className="scale-[0.7]" />
                  </span>
                )}
              </Link>
            ))}
          </div>
        ))}
      </div>
      <div className="flex w-full flex-col items-center gap-1 border-t border-line pt-2 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        <RailButton label="Unfold the sidebar" onClick={() => onFold(false)}>
          <PanelLeftOpen className="size-4" />
        </RailButton>
        <UserMenu data={data} onNavigate={onNavigate} compact />
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
  onNavigate,
  children,
}: {
  to: "/";
  label: string;
  active: boolean;
  lit?: boolean;
  onNavigate: () => void;
  children: ReactNode;
}) {
  return (
    <Link
      to={to}
      onClick={onNavigate}
      aria-label={label}
      title={label}
      className={cn(
        "flex size-9 items-center justify-center rounded-lg transition-colors",
        active
          ? "bg-raised text-accent shadow-[inset_0_0_0_1px_var(--c-line)]"
          : lit
            ? "text-accent hover:bg-hover"
            : "text-ink-4 hover:bg-hover hover:text-ink",
      )}
    >
      {children}
    </Link>
  );
}

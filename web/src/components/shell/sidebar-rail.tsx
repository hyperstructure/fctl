// The workspace rail (a design proposal, lib/sidebar-design.ts): your workspaces as a rail of
// avatars, each lit by its work, and the one you are in filling a panel beside it: its own views
// (sessions at work, what waits for you, pull requests, tasks, runners), then its projects. A
// workspace's pull requests and tasks get a place of their own as they grow, and someone in many
// workspaces sees one at a time. Rail and panel together are about as wide as a session's details
// on the other side.

import { mark } from "@hyperstructure/design/brand";
import { Link, useNavigate } from "@tanstack/react-router";
import { ChevronDown, Cog, Inbox, ListTodo, Plus, Server, Settings, Shield, Users, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { IconPicker } from "@/components/project/icon-picker";
import { ProjectIcon, useProjectIcons } from "@/components/project/project-icon";
import { AgentFloorIcon, Menu, MenuItem, MenuSeparator, PullRequestIcon, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import type { SidebarProject, SidebarWorkspace } from "@/lib/sidebar";
import { createsProjects, roomForAnother } from "@/lib/workspaces";
import {
  type NavData,
  NeedLink,
  type Place,
  placeOf,
  projectHere,
  projectOf,
  SearchButton,
  SessionLink,
  UserMenu,
  useMinute,
  useNavData,
  workspaceHere,
} from "./sidebar-kit";
import { Expander, PullLink, type SidebarProps, TreeProject } from "./sidebar-tree";

const LAST = "fctl:rail-workspace";

export function RailSidebar(props: SidebarProps) {
  const place = placeOf(props.path, props.tab);
  const data = useNavData(place);
  const all = data.workspaces?.workspaces ?? [];
  // The workspace the page is in, or the one you looked at last.
  const [chosen, setChosen] = useState<string | null>(() => {
    try {
      return localStorage.getItem(LAST);
    } catch {
      return null;
    }
  });
  const pageWorkspace = workspaceHere(place, data);
  useEffect(() => {
    if (!pageWorkspace) return;
    setChosen(pageWorkspace.id);
    try {
      localStorage.setItem(LAST, pageWorkspace.id);
    } catch {}
  }, [pageWorkspace]);
  const shown = all.find((w) => w.id === chosen) ?? pageWorkspace ?? all[0];
  const choose = (w: SidebarWorkspace) => {
    setChosen(w.id);
    try {
      localStorage.setItem(LAST, w.id);
    } catch {}
  };
  return (
    <div className="flex h-full">
      <Rail {...props} data={data} place={place} shown={shown} onChoose={choose} />
      <div className="flex min-w-0 flex-1 flex-col">
        {shown ? (
          <Panel {...props} data={data} place={place} workspace={shown} />
        ) : (
          <div className="flex flex-1 flex-col p-4">
            <Link to="/w/new" onClick={props.onNavigate} className="text-[13px] text-ink-2 hover:text-ink">
              Create a workspace
            </Link>
          </div>
        )}
      </div>
    </div>
  );
}

function Rail({
  data,
  place,
  shown,
  onChoose,
  onNavigate,
}: SidebarProps & {
  data: NavData;
  place: Place;
  shown?: SidebarWorkspace;
  onChoose: (w: SidebarWorkspace) => void;
}) {
  const working = data.working.length;
  return (
    <div className="flex w-16 shrink-0 flex-col items-center border-r border-line bg-sunken/70 [:root[data-theme=light]_&]:bg-sunken/80">
      {/* The mark is the way home, to the Agent Floor, as Discord's is: lit by the agents at work. */}
      <div className="flex h-16 shrink-0 items-center">
        <Link
          to="/"
          onClick={onNavigate}
          aria-label={working ? `Agent Floor, ${working} at work` : "Agent Floor"}
          title={working ? `Agent Floor · ${working} at work` : "Agent Floor"}
          className={cn(
            "relative flex rounded-[11px] transition-[box-shadow,transform] duration-200 ease-[var(--ease-out)] active:scale-95",
            place.path === "/" && "shadow-[0_0_0_2px_var(--c-sunken),0_0_0_3.5px_var(--c-ink-3)]",
          )}
        >
          <svg viewBox={mark.viewBox} className="size-9" aria-hidden="true">
            <rect width="256" height="256" rx={mark.radius * 1.15} fill={mark.background} />
            <path d={mark.path} fill={mark.foreground} fillRule="evenodd" />
          </svg>
          {working > 0 && (
            <span className="absolute -right-1 -bottom-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-ink px-1 text-[10px] font-semibold text-canvas tabular-nums ring-2 ring-sunken">
              {working}
            </span>
          )}
        </Link>
      </div>
      <div className="mb-3 h-px w-8 bg-line-strong/70" />
      <nav
        aria-label="Workspaces"
        className="flex min-h-0 w-full flex-1 flex-col items-center gap-2 overflow-y-auto pt-0.5 pb-3"
      >
        {(data.workspaces?.workspaces ?? []).map((w) => {
          const ids = new Set((data.projects ?? []).filter((p) => p.workspace?.id === w.id).map((p) => p.id));
          const waiting = data.needs.filter((n) => ids.has(n.s.project_id)).length;
          const atWork = data.working.filter((s) => ids.has(s.project_id)).length;
          const on = shown?.id === w.id;
          return (
            <div key={w.id} className="group relative flex w-full justify-center">
              {/* Discord's pill: the workspace you're in, and the one under the pointer. */}
              <span
                aria-hidden="true"
                className={cn(
                  "absolute top-1/2 left-0 w-1 -translate-y-1/2 rounded-r-full bg-ink transition-[height,opacity] duration-200 ease-[var(--ease-out)]",
                  on ? "h-6 opacity-100" : "h-2 opacity-0 group-hover:opacity-60",
                )}
              />
              <button
                type="button"
                onClick={() => onChoose(w)}
                aria-pressed={on}
                aria-label={`${w.name}${atWork ? `, ${atWork} at work` : ""}${waiting ? `, ${waiting} need you` : ""}`}
                title={w.name}
                className={cn(
                  "relative rounded-[11px] transition-[transform,box-shadow] duration-200 ease-[var(--ease-out)] active:scale-95",
                  on
                    ? "shadow-[0_0_0_2px_var(--c-sunken),0_0_0_3.5px_var(--c-ink-3)]"
                    : "opacity-80 hover:opacity-100",
                )}
              >
                <WorkspaceAvatar workspace={w} size={36} title={false} />
                {waiting > 0 ? (
                  <span className="absolute -top-1 -right-1 flex h-4 min-w-4 items-center justify-center rounded-full bg-blue px-1 text-[10px] font-semibold text-white tabular-nums ring-2 ring-sunken">
                    {waiting}
                  </span>
                ) : (
                  atWork > 0 && (
                    <span className="absolute -right-0.5 -bottom-0.5 flex size-3 items-center justify-center rounded-full bg-sunken">
                      <StatusDot status="running" className="scale-75" />
                    </span>
                  )
                )}
              </button>
            </div>
          );
        })}
        {data.workspaces && roomForAnother(data.workspaces) && (
          <Link
            to="/w/new"
            onClick={onNavigate}
            aria-label="New workspace"
            title="New workspace"
            className="flex size-9 items-center justify-center rounded-[11px] border border-dashed border-line-strong text-ink-4 transition-colors hover:border-ink-4 hover:text-ink"
          >
            <Plus className="size-4" />
          </Link>
        )}
      </nav>
      <div className="flex flex-col items-center gap-1 pt-2 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        {data.isAdmin && (
          <RailIcon to="/admin" label="Admin" active={place.path === "/admin"} onNavigate={onNavigate}>
            <Shield className="size-4" />
          </RailIcon>
        )}
        <RailIcon to="/settings" label="Settings" active={place.path === "/settings"} onNavigate={onNavigate}>
          <Settings className="size-4" />
        </RailIcon>
        <UserMenu data={data} onNavigate={onNavigate} compact />
      </div>
    </div>
  );
}

function RailIcon({
  to,
  label,
  active,
  onNavigate,
  children,
}: {
  to: string;
  label: string;
  active: boolean;
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
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "text-ink-4 hover:bg-hover hover:text-ink",
      )}
    >
      {children}
    </Link>
  );
}

function Panel({
  workspace,
  data,
  place,
  onPalette,
  onNavigate,
  onClose,
}: SidebarProps & { workspace: SidebarWorkspace; data: NavData; place: Place }) {
  const navigate = useNavigate();
  const iconFor = useProjectIcons();
  const [open, setOpen] = useState<"work" | "waiting" | "pulls" | null>(null);
  const [picking, setPicking] = useState<SidebarProject>();
  const full = workspace as SidebarWorkspace & Partial<G.Workspace>;
  const projects = (data.projects ?? []).filter((p) => p.workspace?.id === workspace.id);
  const ids = new Set(projects.map((p) => p.id));
  const mine = (list: G.Session[]) => list.filter((s) => ids.has(s.project_id));
  const working = mine(data.working);
  const needs = data.needs.filter((n) => ids.has(n.s.project_id));
  const now = useMinute();
  const pulls = data.pulls.filter((p) => ids.has(p.session.project_id));
  const here = projectHere(place, data);
  const on = (tab: string) => place.workspace === workspace.slug && place.tab === tab;
  const go = (tab: "settings" | "people" | "runners") =>
    void navigate({ to: "/w/$slug", params: { slug: workspace.slug }, search: { tab } });
  const lead = (s: G.Session) => (
    <ProjectIcon
      icon={iconFor(projectOf(data, s.project_id) ?? { id: s.project_id, name: s.project_slug })}
      size={14}
    />
  );
  return (
    <>
      <div className="flex h-14 shrink-0 items-center gap-1 pr-2 pl-3">
        <Menu
          align="start"
          className="min-w-0 flex-1"
          trigger={(toggle, isOpen) => (
            <button
              type="button"
              onClick={toggle}
              aria-haspopup="menu"
              aria-expanded={isOpen}
              className="group flex max-w-full min-w-0 items-center gap-1.5 rounded-md px-1.5 py-1 transition-colors hover:bg-hover aria-expanded:bg-hover"
            >
              <span className="truncate text-[15px] font-semibold tracking-[-0.01em] text-ink">
                {workspace.name}
              </span>
              <ChevronDown className="size-3.5 shrink-0 text-ink-4 transition-colors group-hover:text-ink-2" />
            </button>
          )}
        >
          {(close) => (
            <>
              <MenuItem
                icon={<Users />}
                onClick={() => {
                  close();
                  onNavigate();
                  go("people");
                }}
              >
                People
              </MenuItem>
              <MenuItem
                icon={<Server />}
                onClick={() => {
                  close();
                  onNavigate();
                  go("runners");
                }}
              >
                Runners
              </MenuItem>
              <MenuItem
                icon={<Cog />}
                onClick={() => {
                  close();
                  onNavigate();
                  go("settings");
                }}
              >
                Workspace settings
              </MenuItem>
              {createsProjects(workspace) && (
                <>
                  <MenuSeparator />
                  <MenuItem
                    icon={<Plus />}
                    onClick={() => {
                      close();
                      onNavigate();
                      void navigate({ to: "/new", search: { workspace: workspace.slug } });
                    }}
                  >
                    New project
                  </MenuItem>
                </>
              )}
            </>
          )}
        </Menu>
        <button
          type="button"
          aria-label="Close menu"
          className="flex size-11 items-center justify-center rounded-lg text-ink-4 hover:text-ink md:hidden"
          onClick={onClose}
        >
          <X className="size-5" />
        </button>
      </div>
      {full.members !== undefined && (
        <p className="-mt-3 mb-3 flex items-center gap-1.5 px-4.5 text-[12px] text-ink-4">
          <span>{full.members === 1 ? "Just you" : `${full.members} people`}</span>
          <span aria-hidden="true">·</span>
          <span className="inline-flex items-center gap-1.5">
            <span
              className={cn(
                "size-1.5 rounded-full",
                (full.runners_online ?? 0) > 0 ? "bg-checks-passed" : "bg-ink-4",
              )}
            />
            {full.runners_online} of {full.runners} {full.runners === 1 ? "runner" : "runners"} online
          </span>
        </p>
      )}
      <div className="px-3">
        <SearchButton onPalette={onPalette} />
      </div>
      <nav aria-label={`${workspace.name}'s views`} className="mt-3 flex flex-col gap-px px-3">
        <Expander
          icon={<AgentFloorIcon className="size-4" />}
          label="At work"
          n={working.length}
          open={open === "work"}
          onToggle={() => setOpen(open === "work" ? null : "work")}
        >
          {working.map((s) => (
            <SessionLink
              key={s.id}
              s={s}
              here={place.session === s.id}
              onNavigate={onNavigate}
              lead={lead(s)}
            />
          ))}
        </Expander>
        <Expander
          icon={<Inbox />}
          label="Needs you"
          n={needs.length}
          tone="waiting"
          open={open === "waiting"}
          onToggle={() => setOpen(open === "waiting" ? null : "waiting")}
        >
          {needs.map((n) => (
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
              project={projectOf(data, n.s.project_id)?.name ?? n.s.project_slug}
              now={now}
              onNavigate={onNavigate}
            />
          ))}
        </Expander>
        <Expander
          icon={<PullRequestIcon state="open" className="size-4" />}
          label="Pull requests"
          n={pulls.length}
          open={open === "pulls"}
          onToggle={() => setOpen(open === "pulls" ? null : "pulls")}
        >
          {pulls.map((p) => (
            <PullLink key={p.pr.url} pull={p} here={place.session === p.session.id} onNavigate={onNavigate} />
          ))}
        </Expander>
        <ViewLink
          to="linear"
          slug={workspace.slug}
          active={on("linear")}
          icon={<ListTodo />}
          label="Tasks"
          onNavigate={onNavigate}
        />
        <ViewLink
          to="runners"
          slug={workspace.slug}
          active={on("runners")}
          icon={<Server />}
          label="Runners"
          trail={full.runners !== undefined ? `${full.runners_online}/${full.runners}` : undefined}
          onNavigate={onNavigate}
        />
      </nav>
      <div className="mt-4 flex items-center justify-between px-5 pb-1">
        <Link
          to="/w/$slug"
          params={{ slug: workspace.slug }}
          search={{ tab: undefined }}
          onClick={onNavigate}
          className="text-[12px] font-medium text-ink-4 transition-colors hover:text-ink-2"
        >
          Projects
        </Link>
        {createsProjects(workspace) && (
          <Link
            to="/new"
            search={{ workspace: workspace.slug }}
            onClick={onNavigate}
            aria-label={`New project in ${workspace.name}`}
            title="New project"
            className="-mr-1.5 flex size-6 items-center justify-center rounded text-ink-4 transition-colors hover:bg-hover hover:text-ink"
          >
            <Plus className="size-3.5" />
          </Link>
        )}
      </div>
      <nav
        aria-label={`${workspace.name}'s projects`}
        className="flex min-h-0 flex-1 flex-col gap-px overflow-y-auto px-3 pb-4"
      >
        {projects.map((p) => (
          <TreeProject
            key={p.id}
            project={p}
            data={data}
            place={place}
            open={here?.id === p.id}
            onNavigate={onNavigate}
            onPick={() => setPicking(p)}
          />
        ))}
      </nav>
      {picking && (
        <IconPicker
          open
          project={{ ...picking, description: data.full.get(picking.id)?.description }}
          onClose={() => setPicking(undefined)}
        />
      )}
    </>
  );
}

function ViewLink({
  to,
  slug,
  active,
  icon,
  label,
  trail,
  onNavigate,
}: {
  to: "linear" | "runners";
  slug: string;
  active: boolean;
  icon: ReactNode;
  label: string;
  trail?: string;
  onNavigate: () => void;
}) {
  return (
    <Link
      to="/w/$slug"
      params={{ slug }}
      search={{ tab: to }}
      onClick={onNavigate}
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
      <span className="flex-1">{label}</span>
      {trail && <span className="text-[11.5px] text-ink-4 tabular-nums">{trail}</span>}
    </Link>
  );
}

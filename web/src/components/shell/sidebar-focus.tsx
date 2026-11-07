// Focus (a design proposal, lib/sidebar-design.ts): the sidebar as the place to see what to do
// next. What needs you comes first (a reply, a failed run or check, a pause), each saying why;
// then every agent at work and for how long; then all projects, workspace by workspace, as icons
// with their names, to jump to. Folded, it is the tree's rail.

import { Link } from "@tanstack/react-router";
import { PanelLeftClose, Plus, X } from "lucide-react";
import { type ReactNode, useState } from "react";
import { IconPicker } from "@/components/project/icon-picker";
import { ProjectIcon, useProjectIcons } from "@/components/project/project-icon";
import { Logo, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { agoShort } from "@/lib/format";
import type { SidebarProject } from "@/lib/sidebar";
import { roomForAnother } from "@/lib/workspaces";
import { FloorLink } from "./floor-link";
import {
  type NavData,
  NeedLink,
  type Place,
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
import { type SidebarProps, TreeRail } from "./sidebar-tree";

export function FocusSidebar(props: SidebarProps) {
  const place = placeOf(props.path, props.tab);
  const data = useNavData(place);
  if (props.folded) return <TreeRail {...props} data={data} place={place} />;
  return <Focus {...props} data={data} place={place} />;
}

function Focus({
  onPalette,
  onNavigate,
  onClose,
  onFold,
  data,
  place,
}: SidebarProps & { data: NavData; place: Place }) {
  const now = useMinute();
  const iconFor = useProjectIcons();
  const need = data.needs;
  const here = projectHere(place, data);
  const [picking, setPicking] = useState<SidebarProject>();
  const iconOfSession = (s: G.Session) =>
    iconFor(projectOf(data, s.project_id) ?? { id: s.project_id, name: s.project_slug });
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 shrink-0 items-center justify-between pr-2 pl-4">
        <Link to="/" onClick={onNavigate} className="flex rounded-md">
          <Logo large />
        </Link>
        <button
          type="button"
          aria-label="Fold the sidebar"
          title="Fold the sidebar"
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
      <div className="mt-3 px-3">
        <FloorLink active={place.path === "/"} working={data.working.length} onClick={onNavigate} />
      </div>
      <div className="mt-2 flex min-h-0 flex-1 flex-col overflow-y-auto px-3 pb-4">
        <Section title="Needs you" n={need.length} tone="waiting">
          {need.length === 0 ? (
            <p className="px-2.5 py-1.5 text-[12.5px] text-ink-4">Nothing for now: your agents are on it.</p>
          ) : (
            need
              .slice(0, 5)
              .map((n) => (
                <NeedLink
                  key={n.s.id}
                  need={n}
                  here={place.session === n.s.id}
                  icon={<ProjectIcon icon={iconOfSession(n.s)} tile={26} />}
                  project={projectOf(data, n.s.project_id)?.name ?? n.s.project_slug}
                  now={now}
                  onNavigate={onNavigate}
                />
              ))
          )}
        </Section>
        <Section title="At work" n={data.working.length}>
          {data.working.length === 0 ? (
            <p className="px-2.5 py-1.5 text-[12.5px] text-ink-4">No agent is working.</p>
          ) : (
            data.working.map((s) => (
              <SessionLink
                key={s.id}
                s={s}
                here={place.session === s.id}
                onNavigate={onNavigate}
                className="h-8"
                lead={<ProjectIcon icon={iconOfSession(s)} size={15} />}
                trail={
                  <span className="flex shrink-0 items-center gap-1.5 text-[11px] text-ink-4 tabular-nums">
                    <StatusDot status={s.status} className="scale-75" />
                    {agoShort(s.started_at ?? s.created_at, now)}
                  </span>
                }
              />
            ))
          )}
        </Section>
        <Section title="Projects">
          <div className="flex flex-col gap-3 pt-0.5">
            {data.groups.map(({ workspace, projects }) => (
              <div key={workspace.id}>
                <Link
                  to="/w/$slug"
                  params={{ slug: workspace.slug }}
                  search={{ tab: undefined }}
                  onClick={onNavigate}
                  className={cn(
                    "mb-1.5 flex items-center gap-2 rounded-md px-1 py-0.5 text-[12px] font-medium transition-colors",
                    place.workspace === workspace.slug ? "text-ink" : "text-ink-3 hover:text-ink",
                  )}
                >
                  <WorkspaceAvatar workspace={workspace} size={16} title={false} />
                  <span className="truncate">{workspace.name}</span>
                </Link>
                <div className="flex flex-wrap gap-1">
                  {projects.map((p) => {
                    const on = here?.id === p.id;
                    return (
                      <Link
                        key={p.id}
                        to="/p/$slug"
                        params={{ slug: p.slug }}
                        search={{ tab: undefined }}
                        onClick={onNavigate}
                        onContextMenu={(e) => {
                          e.preventDefault();
                          setPicking(p);
                        }}
                        title={p.active_sessions ? `${p.name} · ${p.active_sessions} at work` : p.name}
                        className={cn(
                          "inline-flex h-7 max-w-full items-center gap-1.5 rounded-lg px-2 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
                          on
                            ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                            : "bg-hover/60 text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line)] hover:bg-hover hover:text-ink",
                        )}
                      >
                        <ProjectIcon icon={iconFor(p)} size={14} />
                        <span className="truncate">{p.name}</span>
                        {p.active_sessions > 0 && <StatusDot status="running" className="scale-[0.6]" />}
                      </Link>
                    );
                  })}
                </div>
              </div>
            ))}
            {data.workspaces && roomForAnother(data.workspaces) && (
              <Link
                to="/w/new"
                onClick={onNavigate}
                className="flex h-7 items-center gap-2 rounded-md px-1 text-[12.5px] text-ink-4 transition-colors hover:text-ink-2"
              >
                <Plus className="size-3.5" />
                New workspace
              </Link>
            )}
          </div>
        </Section>
      </div>
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

function Section({
  title,
  n,
  tone,
  children,
}: {
  title: string;
  n?: number;
  tone?: "waiting";
  children: ReactNode;
}) {
  return (
    <section className="mt-4">
      <h3 className="mb-1 flex items-center gap-2 px-2.5 text-[12px] font-medium text-ink-4">
        {title}
        {!!n && (
          <span
            className={cn(
              "inline-flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums",
              tone === "waiting" ? "bg-blue text-white" : "bg-hover text-ink-3",
            )}
          >
            {n}
          </span>
        )}
      </h3>
      <div className="flex flex-col gap-px">{children}</div>
    </section>
  );
}

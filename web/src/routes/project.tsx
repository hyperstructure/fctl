// A project: its sessions, its autopilot, schedules and reviews, its memory, and its settings, each
// a tab (lib/project-page.ts says which, and what an address opens).

import { Link, useParams, useSearch } from "@tanstack/react-router";
import { BookMarked, CalendarClock, GitBranch, Pencil, ScanEye, Settings } from "lucide-react";
import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import { AutopilotTab } from "@/components/project/autopilot";
import { ProjectIconPicker } from "@/components/project/icon-picker";
import { MemoryTab } from "@/components/project/memory";
import { ProjectMark } from "@/components/project/project-icon";
import { ReviewsTab } from "@/components/project/reviews";
import { ScheduleButton, SchedulesTab } from "@/components/project/schedules";
import { ProjectSettings } from "@/components/project/settings";
import {
  AttachButton,
  DraftTray,
  DropHint,
  useAttachments,
  useFileDrop,
} from "@/components/session/attachments";
import { SessionRow } from "@/components/session/row";
import {
  StartButton,
  StartChoices,
  useKeepDraft,
  useStartChoice,
  useStartSession,
} from "@/components/session/start";
import { startKeys } from "@/components/session/task-box";
import {
  AgentFloorIcon,
  AutopilotIcon,
  Banner,
  Empty,
  GithubIcon,
  Skeleton,
  Textarea,
  TextareaControls,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { NoRunnerNote } from "@/components/workspace/no-runners";
import type * as G from "@/gen";
import { filesFromPaste } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { draftId, forgetDraft, loadText } from "@/lib/draft";
import { asked } from "@/lib/models";
import { projectPlaceholder } from "@/lib/placeholders";
import { manages, PROJECT_TABS, type ProjectTab, shownView, tabOf } from "@/lib/project-page";
import { useMe, useProject, useSessions } from "@/lib/queries";
import { repoLabel, repoSummary } from "@/lib/repos";
import { sections } from "@/lib/sessions";
import { usePageTaskBox } from "@/lib/task-box";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

const LABELS: Record<ProjectTab, string> = {
  sessions: "Sessions",
  autopilot: "Autopilot",
  schedules: "Schedules",
  reviews: "Reviews",
  memory: "Memory",
  settings: "Settings",
};

/**
 * Each tab's icon, as the rest of the app draws what the tab holds: the Agent Floor's mark for its
 * sessions, the schedule button's calendar, the eye that reviews are, the bookmark of notes kept,
 * the sidebar's gear. Autopilot draws its own mark (`TabIcon`).
 */
const ICONS: Record<Exclude<ProjectTab, "autopilot">, (className: string) => ReactNode> = {
  sessions: (className) => <AgentFloorIcon className={className} />,
  schedules: (className) => <CalendarClock className={className} aria-hidden="true" />,
  reviews: (className) => <ScanEye className={className} aria-hidden="true" />,
  memory: (className) => <BookMarked className={className} aria-hidden="true" />,
  settings: (className) => <Settings className={className} aria-hidden="true" />,
};

/**
 * A tab's icon: in the accent on the open tab, as its underline is, and lighter as the pointer
 * comes. The autopilot's says how it is, as its mark does elsewhere: a comet round it while it works,
 * pink while it's paused.
 */
function TabIcon({
  tab,
  here,
  autopilot,
}: {
  tab: ProjectTab;
  here: boolean;
  autopilot?: G.ProjectAutopilot;
}) {
  const on = !!autopilot?.state && autopilot.state !== "off";
  const paused = tab === "autopilot" && autopilot?.state === "paused";
  const working = tab === "autopilot" && on && !!autopilot?.working;
  const className = cn(
    "size-4 shrink-0 transition-colors duration-150",
    paused ? "text-pink" : here ? "text-accent" : working ? "text-ink" : "text-ink-4 group-hover:text-ink-3",
  );
  if (tab === "autopilot") return <AutopilotIcon live={working} className={className} />;
  return ICONS[tab](className);
}

/** The project's icon beside its name: its maintainers click it to choose another. */
function HeaderIcon({ project, manage }: { project: G.Project; manage: boolean }) {
  const [picking, setPicking] = useState(false);
  const icon = <ProjectMark project={project} tile={48} />;
  if (!manage) return icon;
  return (
    <>
      <button
        type="button"
        onClick={() => setPicking(true)}
        aria-label={`Change ${project.name}'s icon`}
        title="Change its icon"
        className="group relative shrink-0 rounded-[14px] transition-transform duration-150 ease-[var(--ease-out)] hover:scale-[1.04] active:scale-95"
      >
        {icon}
        <span className="absolute -right-1 -bottom-1 flex size-5 items-center justify-center rounded-full bg-raised text-ink-3 opacity-0 shadow-[0_0_0_1px_var(--c-line-strong),0_2px_6px_rgb(0_0_0/0.25)] transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100">
          <Pencil className="size-2.5" />
        </span>
      </button>
      <ProjectIconPicker project={project} open={picking} onClose={() => setPicking(false)} />
    </>
  );
}

export function ProjectPage() {
  const { slug } = useParams({ from: "/app/p/$slug" });
  const { tab: wanted, schedule, linear } = useSearch({ from: "/app/p/$slug" });
  const project = useProject(slug);
  const p = project.data;
  const manage = manages(p?.role);
  // What the address asks for, where the project has it and the person may see it.
  const view = p ? shownView(wanted, p) : (wanted ?? "sessions");
  const tab = tabOf(view);
  // Autopilots are experimental: the tab shows where the workspace lets projects have one, or it has one.
  const tabs = PROJECT_TABS.filter((t) => t !== "autopilot" || !!p?.autopilot);
  const repos = p && repoSummary(p);
  const workspace = freshest(useWorkspaceIndex(), p?.workspace);
  if (project.isError)
    return (
      <div className="mx-auto max-w-xl px-6 py-24">
        <Banner tone="error">This project does not exist, or you cannot see it.</Banner>
      </div>
    );
  return (
    <div className="mx-auto max-w-[920px] px-6 pt-12 pb-24">
      {p ? (
        <header className="animate-rise">
          {workspace && (
            <Link
              to="/w/$slug"
              params={{ slug: workspace.slug }}
              className="mb-3 inline-flex max-w-full items-center gap-2 text-[13px] text-ink-3 transition-colors hover:text-ink"
            >
              <WorkspaceAvatar workspace={workspace} size={18} />
              <span className="truncate">{workspace.name}</span>
            </Link>
          )}
          {/* A name or a link with no spaces wraps on a phone instead of widening the page. */}
          <div className="flex items-center gap-4">
            <HeaderIcon project={p} manage={manage} />
            <h1 className="min-w-0 font-display text-[38px] leading-[1.1] font-medium tracking-[-0.022em] [overflow-wrap:anywhere]">
              {p.name}
            </h1>
          </div>
          {p.description && (
            <p className="mt-2 max-w-2xl text-[15px] leading-relaxed text-ink-3 [overflow-wrap:anywhere]">
              {p.description}
            </p>
          )}
          <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-[12.5px] text-ink-3">
            {repos ? (
              <span className="inline-flex items-center gap-1.5">
                {p.repo && (
                  <a
                    href={`https://github.com/${p.repo.owner}/${p.repo.name}`}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center gap-1.5 font-mono text-ink-2 transition-colors hover:text-ink"
                  >
                    <GithubIcon className="size-3.5" />
                    {repos.name}
                  </a>
                )}
                {repos.more && (
                  <Link
                    to="/p/$slug"
                    params={{ slug }}
                    search={{ tab: "repositories" }}
                    className="transition-colors hover:text-ink"
                  >
                    {p.repo ? repos.more : repoLabel(p)}
                  </Link>
                )}
              </span>
            ) : (
              <span>No repository</span>
            )}
            <span className="inline-flex items-center gap-1.5 font-mono">
              <GitBranch className="size-3.5 text-ink-4" />
              {p.default_branch}
            </span>
            <span>
              {p.total_sessions} {p.total_sessions === 1 ? "session" : "sessions"}
              {p.active_sessions > 0 && <span className="text-accent"> · {p.active_sessions} active</span>}
            </span>
          </div>
        </header>
      ) : (
        <div>
          <Skeleton className="h-10 w-80" />
          <Skeleton className="mt-4 h-4 w-48" />
        </div>
      )}
      <Tabs slug={slug} tabs={tabs} tab={tab} autopilot={p?.autopilot} />
      <div className="mt-8">
        {p && view === "sessions" && <Sessions project={p} />}
        {p && view === "autopilot" && <AutopilotTab key={p.id} project={p} />}
        {p && view === "schedules" && <SchedulesTab project={p} open={schedule} />}
        {p && view === "reviews" && <ReviewsTab project={p} />}
        {p && tab === "memory" && (
          <MemoryTab project={p} part={view === "retrospectives" ? "retrospectives" : "notes"} />
        )}
        {p && tab === "settings" && <ProjectSettings project={p} view={view} linear={linear} />}
      </div>
    </div>
  );
}

/**
 * The project's tabs, each an address: the default, Sessions, has none, so the page's address stays
 * clean. Where they don't fit (phones) they scroll sideways, and the open one scrolls into view.
 */
function Tabs({
  slug,
  tabs,
  tab,
  autopilot,
}: {
  slug: string;
  tabs: ProjectTab[];
  tab: ProjectTab;
  autopilot?: G.ProjectAutopilot;
}) {
  const strip = useRef<HTMLElement>(null);
  const shown = tabs.join();
  // biome-ignore lint/correctness/useExhaustiveDependencies: again as another tab opens, or the tabs change
  useLayoutEffect(() => {
    const nav = strip.current;
    const here = nav?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!nav || !here || nav.scrollWidth <= nav.clientWidth) return;
    const a = here.getBoundingClientRect();
    const v = nav.getBoundingClientRect();
    if (a.left >= v.left && a.right <= v.right) return;
    // Sideways only: the page stays where it is.
    nav.scrollLeft += a.left + a.width / 2 - (v.left + v.width / 2);
  }, [tab, shown]);
  const asks = autopilot?.asks ?? 0;
  return (
    <nav ref={strip} aria-label="Project" className="-mx-6 mt-8 overflow-x-auto px-6 [scrollbar-width:none]">
      <div className="flex w-max min-w-full gap-1 border-b border-line">
        {tabs.map((t) => {
          const here = tab === t;
          return (
            <Link
              key={t}
              to="/p/$slug"
              params={{ slug }}
              search={{ tab: t === "sessions" ? undefined : t }}
              // The page says which tab is open: a page of Memory or Settings is under its tab, and
              // TanStack's Link, matching search partly, would call Sessions open on every tab.
              aria-current={here ? "page" : undefined}
              activeOptions={{ exact: true }}
              className={cn(
                "group relative inline-flex shrink-0 items-center gap-2 px-3 py-2.5 text-[13.5px] transition-colors duration-150",
                here ? "text-ink" : "text-ink-3 hover:text-ink",
              )}
            >
              <TabIcon tab={t} here={here} autopilot={autopilot} />
              {LABELS[t]}
              {t === "autopilot" && asks > 0 && (
                <span
                  title={`It asks you ${asks === 1 ? "one thing" : `${asks} things`}`}
                  className="inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-blue px-1 text-[10px] font-semibold text-white tabular-nums"
                >
                  {asks}
                </span>
              )}
              {here && (
                <span className="absolute inset-x-2 -bottom-px h-[2px] rounded-full bg-accent forced-colors:bg-[CanvasText] forced-color-adjust-none" />
              )}
            </Link>
          );
        })}
      </div>
    </nav>
  );
}

function Sessions({ project }: { project: G.Project }) {
  const [filter, setFilter] = useState<"active" | "all" | "mine">("all");
  const mine = filter === "mine" || undefined;
  // Every session at work, however long ago it started. By the project's id, as the sidebar asks for
  // them under it: without `mine`, one request serves both.
  const active = useSessions({ project: project.id, status: "active", limit: 50, mine });
  const history = filter !== "active";
  const ended = useSessions({ project: project.id, status: "ended", limit: 100, mine }, { enabled: history });
  const { working, recent, older } = sections(
    history ? [active.data?.sessions, ended.data?.sessions] : [active.data?.sessions],
    Date.now(),
  );
  const canStart = project.role !== "viewer";
  const { data: me } = useMe();
  return (
    <div className="flex flex-col gap-10">
      {/* Another project's page opens its own draft. */}
      {canStart && me && <StartForm key={project.id} project={project} user={me.user.id} />}
      <div>
        <div className="mb-8 flex items-center gap-1">
          {(["all", "active", "mine"] as const).map((f) => (
            <button
              key={f}
              type="button"
              aria-pressed={filter === f}
              onClick={() => setFilter(f)}
              className={cn(
                "rounded-full px-3 py-1 text-[12.5px] capitalize transition-colors",
                filter === f
                  ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                  : "text-ink-3 hover:text-ink",
              )}
            >
              {f}
            </button>
          ))}
        </div>
        {active.isPending || (history && ended.isPending) ? (
          <div className="space-y-3 py-3">
            <Skeleton className="h-11" />
            <Skeleton className="h-11" />
          </div>
        ) : history && !working.length && !recent.length && !older.length ? (
          <Empty title="No sessions yet">
            Describe a task above and an agent starts on it in a fresh sandbox.
          </Empty>
        ) : (
          // In sections, as on the Agent Floor: the sessions at work, then the ended ones that moved in
          // the last day, whose times read in minutes and hours, then the rest.
          <div className="flex flex-col gap-12">
            <SessionSection title="Working now" note={`${working.length} active`}>
              {working.length ? (
                <SessionList sessions={working} />
              ) : (
                <p className="border-y border-line py-6 text-[13.5px] text-ink-4">
                  {mine ? "No agents are working for you." : "No agents are working."}
                  {canStart && " Give one a task above."}
                </p>
              )}
            </SessionSection>
            {history && recent.length > 0 && (
              <SessionSection title="Recently">
                <SessionList sessions={recent} />
              </SessionSection>
            )}
            {history && older.length > 0 && (
              <SessionSection title="Older">
                <SessionList sessions={older} />
              </SessionSection>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

/** A section of a project's sessions, as the Agent Floor has them. */
function SessionSection({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section>
      <div className="mb-2 flex items-baseline justify-between">
        <h2 className="text-[13px] font-semibold text-ink">{title}</h2>
        {note && <span className="text-[12px] text-ink-4">{note}</span>}
      </div>
      {children}
    </section>
  );
}

/** Sessions in their project's list: each one's number, branch and pull requests. */
function SessionList({ sessions }: { sessions: G.Session[] }) {
  return (
    <div>
      {sessions.map((s) => (
        <SessionRow key={s.id} s={s} inProject />
      ))}
    </div>
  );
}

/**
 * The Sessions tab's task box. What is written in it stays in this browser until a session starts
 * with it, apart from the home page's draft and other projects'.
 */
function StartForm({ project, user }: { project: G.Project; user: string }) {
  const keep = draftId(user, project.id);
  const [kept] = useState(() => loadText(keep));
  const [prompt, setPrompt] = useState(kept?.prompt ?? "");
  // A line picked as the project opens, which keeps up with its name.
  const [placeholder] = useState(projectPlaceholder);
  const { choice, picked, pick } = useStartChoice(project, kept);
  const ref = useRef<HTMLTextAreaElement>(null);
  usePageTaskBox(ref, true);
  const files = useAttachments({ project: project.slug }, { keep });
  const draft = useKeepDraft(keep, {
    kept,
    textarea: ref,
    text: { prompt, ...picked },
    files: files.drafts.length,
  });
  const { start, starting } = useStartSession(draft.forget);
  const go = () => start(project.slug, prompt, files, asked(project, choice));
  const attach = (f: File[], pasted?: boolean) => {
    files.add(f, { pasted });
    ref.current?.focus();
  };
  const dragging = useFileDrop(attach, !starting);
  const workspace = useWorkspaceIndex().get(project.workspace?.id ?? "");
  return (
    <div>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          go();
        }}
        className={cn(
          "relative rounded-2xl border border-line-strong bg-surface/80 transition-[border-color,box-shadow] focus-within:border-accent/45 focus-within:shadow-[0_0_0_4px_var(--c-accent-soft)]",
          dragging && "border-accent/50",
        )}
      >
        <DraftTray
          drafts={files.drafts}
          onRemove={files.remove}
          onRetry={files.retry}
          readOnly={starting}
          className="px-4 pt-4"
        />
        <Textarea
          ref={ref}
          bare
          minRows={2}
          maxRows={12}
          value={prompt}
          readOnly={starting}
          onChange={(e) => setPrompt(e.target.value)}
          onBlur={draft.save}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              go();
            } else if (e.key === "Escape" && !e.nativeEvent.isComposing) {
              // Out of the box, the page's keys work again.
              e.currentTarget.blur();
            }
          }}
          onPaste={(e) => {
            const pasted = filesFromPaste(e.clipboardData);
            if (!pasted.length || starting) return;
            e.preventDefault();
            attach(pasted, true);
          }}
          placeholder={placeholder(project.name)}
          className="block px-5 pt-4 font-display text-[17px] leading-[1.5] placeholder:text-ink-4"
          aria-label="Task for a new session"
        />
        {/* More room above the pickers where they take a line of their own. On a phone everything
            keeps one line, down to 360 px with a model named by its ID and the GitHub picker: the
            gaps are narrower there, and the pickers go to the end without the spacer and its
            gaps. */}
        <TextareaControls
          textarea={ref}
          className="flex-wrap gap-3 pt-2 pr-3 pb-3 pl-2.5 max-sm:gap-2 sm:max-lg:pt-3"
        >
          <AttachButton onFiles={(f) => attach(f)} disabled={starting} />
          <span className="flex-1 max-sm:hidden" />
          <StartChoices
            project={project}
            choice={choice}
            onPick={pick}
            issues
            disabled={starting}
            className="max-sm:ml-auto"
          />
          {/* Set apart from the pickers, as on the home page. On a phone its room goes to the
              button. */}
          <span className="ml-4 text-[11.5px] text-ink-4 max-sm:hidden">{startKeys()}</span>
          <ScheduleButton
            project={project}
            prompt={prompt}
            choice={choice}
            files={files.drafts.length}
            disabled={starting}
            // The draft stays here until a schedule is set up with it.
            onSaved={() => void forgetDraft(keep)}
          />
          <StartButton starting={starting} disabled={!prompt.trim() && !files.drafts.length}>
            Start a session
          </StartButton>
        </TextareaControls>
        <DropHint show={dragging} />
      </form>
      <NoRunnerNote workspace={workspace} className="mt-3" />
    </div>
  );
}

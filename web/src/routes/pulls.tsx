// Pull requests: what agents opened in your projects, each once, sorted by what it needs next: an
// answer its agent waits for, a review, a fix for its checks, its agent to finish, its turn in a
// merge queue. The sessions that list them come from the sessions API (`?pulls=any`, archived ones
// and those of schedules that hide theirs too), which the feed keeps current: states, checks and
// reviews change here as GitHub tells the control plane, and a row that moves says so for a moment.
//
// The keyboard: j and k (or the arrows) move through the list, Enter opens the session, o opens the
// pull request on GitHub, c copies its link, / filters, and the arrows move between the tabs.

import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import {
  ArrowUpRight,
  CircleAlert,
  CircleCheck,
  Layers,
  MessageCircleQuestion,
  Search,
  X,
} from "lucide-react";
import { type ReactNode, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ProjectMark } from "@/components/project/project-icon";
import { PullRequestChecksIcon, PullRequestStateIcon } from "@/components/session/pull-request";
import {
  Avatar,
  Banner,
  Button,
  ChecksIcon,
  Empty,
  EyeIcon,
  Kbd,
  PullRequestIcon,
  ReviewIcon,
  Select,
  type SelectOption,
  Skeleton,
  Spinner,
  StatusDot,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { setPageTitle } from "@/lib/attention";
import { cn } from "@/lib/cn";
import { agoShort, stamp } from "@/lib/format";
import { useHotkey } from "@/lib/hotkeys";
import {
  askingOf,
  countStates,
  filterWords,
  type ListedPull,
  listPulls,
  moved,
  NEEDS,
  type Need,
  needOf,
  PULL_STATES,
  PULLS_QUERY,
  type PullSize,
  type PullState,
  peopleOf,
  pullMatches,
  sizeOf,
  standing,
  stateOf,
} from "@/lib/pull-list";
import {
  checksState,
  checksText,
  prFullRef,
  prLabel,
  prReview,
  prReviewLook,
  prReviewText,
} from "@/lib/pulls";
import { keepPullsLeft, usePullsLeft, usePullsMine } from "@/lib/pulls-view";
import { keys, useProjects, useWorkspaces } from "@/lib/queries";
import { issues, type ReviewLook } from "@/lib/reviews";

/**
 * What the page's address says it shows. One that names no tab comes from elsewhere (the sidebar,
 * the command palette): the page shows the tab and the filter it showed last in this browser tab,
 * and puts them in its address as they change. Mine isn't in it: this browser keeps it.
 */
export interface PullsSearch {
  state?: PullState;
  /** `w:<slug>` for a workspace's, `p:<slug>` for a project's; none for all. */
  scope?: string;
  q?: string;
}

const TITLES: Record<PullState, string> = { open: "Open", merged: "Merged", closed: "Closed" };

export function PullsPage() {
  const search = useSearch({ from: "/app/pulls" });
  const navigate = useNavigate({ from: "/pulls" });
  const workspaces = useWorkspaces().data?.workspaces ?? [];
  const projects = useProjects().data?.projects ?? [];
  const [mine, setMine] = usePullsMine();
  const left = usePullsLeft();
  // Come from elsewhere: the tab and the filter as they were left (PullsSearch).
  const arrived = search.state === undefined;
  const state = search.state ?? left.state ?? "open";
  const filtering = search.q ?? (arrived ? left.q : undefined);
  useEffect(() => keepPullsLeft({ state, q: filtering }), [state, filtering]);
  const [kind, slug] = (search.scope ?? "").split(":");
  const scope = { workspace: kind === "w" ? slug : undefined, project: kind === "p" ? slug : undefined };
  // Every session that lists one, whatever its state: each pull request takes the state the session
  // that heard of it last says, which one that heard of it earlier may not.
  const q = { ...PULLS_QUERY, ...scope, mine: mine || undefined };
  const key = keys.sessions(q);
  const sessions = useQuery({ queryKey: key, queryFn: () => api.sessions(q) });
  const all = useMemo(() => listPulls(sessions.data?.sessions ?? []), [sessions.data]);
  const words = useMemo(() => filterWords(filtering ?? ""), [filtering]);
  // The tabs count what the filter finds, so that they say where the rest of it is.
  const matching = useMemo(
    () => (words.length ? all.filter((l) => pullMatches(l, filtering ?? "")) : all),
    [all, words, filtering],
  );
  const counts = useMemo(() => countStates(matching), [matching]);
  const shown = useMemo(() => matching.filter((l) => stateOf(l) === state), [matching, state]);
  const changed = useChangedRows(all, JSON.stringify(key));
  // The tab and the filter shown go in the address with the change, so that it says them from then on.
  const set = (next: Partial<PullsSearch>) =>
    void navigate({
      search: (s: PullsSearch) => ({ ...s, state, q: filtering, ...next }),
      replace: true,
    });
  const filter = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useHotkey("/", () => filter.current?.focus());
  const onListKey = useListKeys(list);
  useEffect(() => {
    setPageTitle("Pull requests");
    return () => setPageTitle(undefined);
  }, []);

  return (
    <div className="mx-auto max-w-[920px] px-6 pt-12 pb-24 max-sm:px-4 max-sm:pt-8">
      <header className="animate-rise">
        <h1 className="flex items-center gap-3 font-display text-[38px] leading-[1.1] font-medium tracking-[-0.022em] max-sm:text-[32px]">
          Pull requests
        </h1>
        <p className="mt-2 max-w-2xl text-[15px] leading-relaxed text-ink-3">
          What agents opened in {scopeName(search.scope, workspaces, projects)}, sorted by what each needs
          next. Their states and checks change here as GitHub reports them.
        </p>
      </header>

      {/* Beside the page it stays in view, so that the tabs and the filter are at hand anywhere in a long list. */}
      <div className="z-10 mt-8 bg-canvas/85 backdrop-blur-xl md:sticky md:top-0 md:-mx-6 md:mt-6 md:px-6 md:pt-2">
        <div className="flex flex-wrap items-end justify-between gap-x-4 gap-y-3 border-b border-line">
          <Tabs
            state={state}
            counts={counts}
            loading={sessions.isPending}
            onPick={(s) => set({ state: s })}
          />
          <div className="flex w-full items-center gap-2 pb-2 sm:w-auto">
            <label className="flex h-7 min-w-0 flex-1 items-center gap-2 rounded-md border border-line bg-sunken/60 px-2 text-[12.5px] transition-colors focus-within:border-line-strong sm:w-56 sm:flex-none">
              <Search className="size-3.5 shrink-0 text-ink-4" />
              <input
                ref={filter}
                value={filtering ?? ""}
                onChange={(e) => set({ q: e.target.value || undefined })}
                onKeyDown={(e) => {
                  if (e.key === "Escape") {
                    set({ q: undefined });
                    e.currentTarget.blur();
                  } else if (e.key === "ArrowDown") {
                    e.preventDefault();
                    list.current?.querySelector<HTMLElement>("[data-row]")?.focus();
                  }
                }}
                placeholder="Filter by title, repo or #"
                aria-label="Filter pull requests"
                className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-ink-4"
              />
              {filtering ? (
                <button
                  type="button"
                  aria-label="Clear the filter"
                  onClick={() => {
                    set({ q: undefined });
                    filter.current?.focus();
                  }}
                  className="rounded p-0.5 text-ink-4 hover:text-ink"
                >
                  <X className="size-3" />
                </button>
              ) : (
                <span className="[@media(hover:none)]:hidden">
                  <Kbd>/</Kbd>
                </span>
              )}
            </label>
            <ScopeSelect value={search.scope} onChange={(scope) => set({ scope })} />
            <button
              type="button"
              aria-pressed={mine}
              onClick={() => setMine(!mine)}
              title="Only those of sessions you started"
              className={cn(
                "h-7 shrink-0 rounded-md border px-2.5 text-[12.5px] transition-colors",
                mine
                  ? "border-accent/45 bg-accent-soft text-ink"
                  : "border-line text-ink-3 hover:border-line-strong hover:bg-hover hover:text-ink",
              )}
            >
              Mine
            </button>
          </div>
        </div>
      </div>

      <div
        ref={list}
        id="pulls-list"
        role="tabpanel"
        aria-labelledby={`pulls-tab-${state}`}
        className="mt-6"
        onKeyDown={onListKey}
      >
        {sessions.isError && sessions.data && (
          // A refresh that failed keeps what loaded on screen, and says it may be behind.
          <p
            role="status"
            className="mb-4 flex flex-wrap items-center gap-x-2 gap-y-1 text-[12.5px] text-ink-3"
          >
            <CircleAlert className="size-3.5 shrink-0 text-danger" />
            <span>This list didn't refresh: {sessions.error.message}</span>
            <button
              type="button"
              onClick={() => void sessions.refetch()}
              className="rounded text-ink-2 underline decoration-line-strong underline-offset-2 transition-colors hover:text-ink hover:decoration-ink-3"
            >
              Try again
            </button>
          </p>
        )}
        {sessions.isError && !sessions.data ? (
          <Banner tone="error" className="mt-2">
            <span className="flex flex-wrap items-center justify-between gap-3">
              The pull requests didn't load: {sessions.error.message}
              <Button size="sm" onClick={() => void sessions.refetch()}>
                Try again
              </Button>
            </span>
          </Banner>
        ) : sessions.isPending ? (
          <ul aria-label="Loading" className="border-t border-line">
            {[0, 1, 2, 3].map((i) => (
              <RowSkeleton key={i} wide={i % 2 === 0} />
            ))}
          </ul>
        ) : shown.length === 0 ? (
          <Nothing
            state={state}
            q={filtering}
            mine={mine}
            scope={search.scope && scopeName(search.scope, workspaces, projects)}
            counts={counts}
            onPick={(s) => set({ state: s })}
            onClear={() => set({ q: undefined })}
          />
        ) : state === "open" ? (
          <OpenList shown={shown} words={words} changed={changed} />
        ) : (
          byWhen(shown).map(([title, these]) => (
            <Section key={title} title={title} n={these.length}>
              {these.map((l) => (
                <PullRow key={l.pr.url} l={l} words={words} changed={changed.has(l.pr.url)} />
              ))}
            </Section>
          ))
        )}
        {sessions.data?.next_before && (
          <p className="mt-6 text-center text-[12.5px] text-ink-4">
            From the newest {PULLS_QUERY.limit} sessions with pull requests. Narrow it to a project for older
            ones.
          </p>
        )}
      </div>
      <p className="mt-10 hidden text-center text-[12px] text-ink-4 md:block">
        <Kbd>j</Kbd> <Kbd>k</Kbd> move · <Kbd>↵</Kbd> opens the session · <Kbd>o</Kbd> opens it on GitHub ·{" "}
        <Kbd>c</Kbd> copies its link · <Kbd>/</Kbd> filters
      </p>
    </div>
  );
}

/**
 * Open, Merged and Closed, with how many each has: tabs, which the arrows move between, under a line
 * that slides to the one picked.
 */
function Tabs({
  state,
  counts,
  loading,
  onPick,
}: {
  state: PullState;
  counts: Record<PullState, number>;
  loading: boolean;
  onPick: (s: PullState) => void;
}) {
  const tabs = useRef<Partial<Record<PullState, HTMLButtonElement | null>>>({});
  const [line, setLine] = useState<{ left: number; width: number }>();
  // Measured again as the counts change the tabs' widths.
  // biome-ignore lint/correctness/useExhaustiveDependencies: counts and loading change the widths.
  useLayoutEffect(() => {
    const tab = tabs.current[state];
    if (tab) setLine({ left: tab.offsetLeft, width: tab.offsetWidth });
  }, [state, counts, loading]);
  const go = (e: React.KeyboardEvent) => {
    const at = PULL_STATES.indexOf(state);
    const n = PULL_STATES.length;
    const to =
      e.key === "ArrowRight"
        ? (at + 1) % n
        : e.key === "ArrowLeft"
          ? (at + n - 1) % n
          : e.key === "Home"
            ? 0
            : e.key === "End"
              ? n - 1
              : undefined;
    const next = to === undefined ? undefined : PULL_STATES[to];
    if (!next) return;
    e.preventDefault();
    onPick(next);
    tabs.current[next]?.focus();
  };
  return (
    <div role="tablist" aria-label="States" onKeyDown={go} className="relative -mb-px flex gap-1">
      {PULL_STATES.map((s) => (
        <button
          key={s}
          ref={(el) => {
            tabs.current[s] = el;
          }}
          id={`pulls-tab-${s}`}
          type="button"
          role="tab"
          aria-selected={state === s}
          aria-controls="pulls-list"
          tabIndex={state === s ? 0 : -1}
          onClick={() => onPick(s)}
          className={cn(
            "flex items-center gap-2 rounded-t-md px-3 py-2.5 text-[13.5px] transition-colors outline-none focus-visible:bg-hover",
            state === s ? "text-ink" : "text-ink-3 hover:text-ink",
          )}
        >
          {TITLES[s]}
          {loading ? (
            <span aria-hidden="true" className="h-[15px] w-5 animate-pulse rounded-full bg-hover" />
          ) : (
            <span
              className={cn(
                "min-w-5 rounded-full px-1.5 text-center text-[11px] tabular-nums transition-colors",
                state === s ? "bg-hover text-ink-2" : "text-ink-4",
              )}
            >
              {counts[s]}
            </span>
          )}
        </button>
      ))}
      {line && (
        <span
          aria-hidden="true"
          className="pointer-events-none absolute -bottom-px h-[2px] rounded-full bg-accent transition-[left,width] duration-300 ease-[var(--ease-out)] motion-reduce:transition-none"
          style={{ left: line.left + 8, width: Math.max(0, line.width - 16) }}
        />
      )}
    </div>
  );
}

/** Your projects by id, for their icons. */
function useProjectIndex(): Map<string, G.Project> {
  const projects = useProjects().data?.projects;
  return useMemo(() => new Map((projects ?? []).map((p) => [p.id, p])), [projects]);
}

/** What the page lists, by name: "your projects", "Romanesco's projects", "Shop". */
function scopeName(
  scope: string | undefined,
  workspaces: readonly G.Workspace[],
  projects: readonly G.Project[],
): string {
  if (!scope) return "your projects";
  const [kind, slug = ""] = scope.split(":");
  if (kind === "w") return `${workspaces.find((w) => w.slug === slug)?.name ?? slug}'s projects`;
  return projects.find((p) => p.slug === slug)?.name ?? slug;
}

/** Which workspace or project the page shows: one list, its workspaces' projects under each. */
function ScopeSelect({ value, onChange }: { value?: string; onChange: (v: string | undefined) => void }) {
  const workspaces = useWorkspaces().data?.workspaces ?? [];
  const projects = useProjects().data?.projects ?? [];
  const options: SelectOption[] = [
    { value: "all", label: "All projects", icon: <Layers className="size-3.5" /> },
    ...workspaces.flatMap((w) => [
      {
        value: `w:${w.slug}`,
        label: w.name,
        item: `All of ${w.name}`,
        icon: <WorkspaceAvatar workspace={w} size={14} title={false} />,
        section: { key: w.id, label: w.name },
      },
      ...projects
        .filter((p) => p.workspace?.id === w.id)
        .map((p) => ({
          value: `p:${p.slug}`,
          label: `${w.name} / ${p.name}`,
          item: p.name,
          short: p.name,
          icon: <ProjectMark project={p} size={14} />,
          section: { key: w.id, label: w.name },
        })),
    ]),
  ];
  return (
    <Select
      size="sm"
      value={value ?? "all"}
      options={options}
      onChange={(v) => onChange(v === "all" ? undefined : v)}
      aria-label="Which projects"
      narrow="fit"
    />
  );
}

/** How each group of open pull requests is marked: by what it needs next. */
const NEED_MARKS: Record<Need, ReactNode> = {
  asking: <MessageCircleQuestion className="size-3.5 text-blue" />,
  ready: <PullRequestIcon state="open" className="size-3.5 text-pr-open" />,
  failing: <ChecksIcon state="failed" className="size-3.5 text-checks-failed" />,
  working: <StatusDot status="running" />,
  queued: <PullRequestIcon state="queued" className="size-3.5 text-pr-queued" />,
  draft: <PullRequestIcon state="draft" className="size-3.5 text-pr-draft" />,
};

/**
 * The open ones, in groups by what each needs next, those that need a person first. When none does,
 * a line says so above the rest.
 */
function OpenList({
  shown,
  words,
  changed,
}: {
  shown: ListedPull[];
  words: string[];
  changed: ReadonlySet<string>;
}) {
  const groups = NEEDS.map((n) => ({ ...n, these: shown.filter((l) => needOf(l) === n.need) }));
  const waiting = groups
    .filter((g) => g.need === "asking" || g.need === "ready" || g.need === "failing")
    .some((g) => g.these.length > 0);
  return (
    <>
      {!waiting && (
        <p className="mb-6 flex animate-rise items-center gap-2 px-3 text-[13px] text-ink-3">
          <CircleCheck className="size-4 shrink-0 text-checks-passed" />
          Nothing waits for an answer or a review: agents are still at work on these.
        </p>
      )}
      {groups.map(({ need, title, hint, these }) =>
        these.length ? (
          <Section key={need} title={title} hint={hint} n={these.length} mark={NEED_MARKS[need]}>
            {these.map((l) => (
              <PullRow key={l.pr.url} l={l} words={words} changed={changed.has(l.pr.url)} />
            ))}
          </Section>
        ) : null,
      )}
    </>
  );
}

function Section({
  title,
  hint,
  n,
  mark,
  children,
}: {
  title: string;
  hint?: string;
  n: number;
  mark?: ReactNode;
  children: ReactNode;
}) {
  return (
    <section className="mb-8 animate-rise last:mb-0">
      <h2 className="mb-1.5 flex items-center gap-2 px-3 text-[13px] font-semibold text-ink">
        {mark && (
          <span aria-hidden="true" className="flex size-4 shrink-0 items-center justify-center">
            {mark}
          </span>
        )}
        {title}
        <span className="min-w-5 rounded-full bg-hover px-1.5 text-center text-[11px] font-medium text-ink-3 tabular-nums">
          {n}
        </span>
        {hint && <span className="hidden truncate text-[12px] font-normal text-ink-4 sm:inline">{hint}</span>}
      </h2>
      <ul aria-label={title} className="border-t border-line">
        {children}
      </ul>
    </section>
  );
}

/** When a merged or closed one was merged or closed; an open one, when it last moved. */
const when = (l: ListedPull) => (stateOf(l) === "open" ? moved(l) : (l.pr.state_changed_at ?? moved(l)));

/** What its time says in full, by its tab. */
const WHEN_SAYS: Record<PullState, string> = { open: "Updated", merged: "Merged", closed: "Closed" };

/** Merged and closed ones by when that happened, newest first: today, yesterday, this week, earlier. */
function byWhen(list: ListedPull[]): [string, ListedPull[]][] {
  const day = 86_400_000;
  const start = new Date();
  start.setHours(0, 0, 0, 0);
  const today = start.getTime();
  const buckets: [string, (t: number) => boolean][] = [
    ["Today", (t) => t >= today],
    ["Yesterday", (t) => t >= today - day],
    ["This week", (t) => t >= today - 6 * day],
    ["Earlier", () => true],
  ];
  const out = new Map<string, ListedPull[]>(buckets.map(([name]) => [name, []]));
  for (const l of [...list].sort((a, b) => when(b) - when(a))) {
    const [name] = buckets.find(([, fits]) => fits(when(l))) ?? ["Earlier"];
    out.get(name)?.push(l);
  }
  return [...out.entries()].filter(([, these]) => these.length > 0);
}

/**
 * The pull requests whose standing changed since the list last came, for a moment: a row that moves
 * to another group, or whose checks or review change, shows it as the feed brings the change. Not as
 * the list first comes, nor when the page asks for another one.
 */
function useChangedRows(list: ListedPull[], asked: string): ReadonlySet<string> {
  const seen = useRef<{ asked: string; standing: Map<string, string> }>(undefined);
  const [changed, setChanged] = useState<ReadonlySet<string>>(new Set());
  useEffect(() => {
    if (!list.length) return;
    const now = new Map(list.map((l) => [l.pr.url, standing(l)]));
    const before = seen.current?.asked === asked ? seen.current.standing : undefined;
    seen.current = { asked, standing: now };
    if (!before) return;
    const these = [...now].filter(([url, s]) => before.get(url) !== s).map(([url]) => url);
    if (these.length) setChanged((lit) => new Set([...lit, ...these]));
  }, [list, asked]);
  // They go dark once the last of them has had its moment, whatever else comes meanwhile: a timer of
  // their own, which only a change among them starts again.
  useEffect(() => {
    if (!changed.size) return;
    const done = setTimeout(() => setChanged(new Set()), LIT_MS);
    return () => clearTimeout(done);
  }, [changed]);
  return changed;
}

/** How long a row that changed stays lit: its animation's length (`--animate-row-moved`). */
const LIT_MS = 2000;

/**
 * A pull request: its state, title and repository, the sessions that list it, its checks, fctl's
 * review and its size, who started its sessions and when it last moved; under it, what its agent
 * asks, with the answers it offers. The row opens its newest session; the arrow, the pull request on
 * GitHub.
 */
function PullRow({ l, words, changed }: { l: ListedPull; words: string[]; changed: boolean }) {
  const { pr, sessions } = l;
  const projects = useProjectIndex();
  const [s, ...others] = sessions;
  const tab = stateOf(l);
  const asking = tab === "open" ? askingOf(l) : undefined;
  const at = when(l);
  if (!s) return null;
  const title = pr.title ?? s.title;
  return (
    <li
      className={cn(
        "group border-b border-line last:border-b-0",
        changed && "animate-row-moved motion-reduce:animate-none",
      )}
    >
      <div className="flex items-start">
        <Link
          to="/s/$id"
          params={{ id: s.id }}
          data-row
          aria-label={`${title}: ${prLabel(pr)}`}
          data-github={pr.url}
          data-ref={prFullRef(pr)}
          className="flex min-w-0 flex-1 items-start gap-3.5 rounded-md py-3 pr-1 pl-3 outline-none transition-colors hover:bg-hover/40 focus-visible:bg-hover/60 focus-visible:shadow-[inset_0_0_0_1.5px_var(--c-accent)]"
        >
          <PullRequestStateIcon pr={pr} className="mt-[3px] size-4" />
          <div className="min-w-0 flex-1">
            <div className="truncate text-[14.5px] text-ink">
              <Marked text={title} words={words} />
            </div>
            <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[12px] text-ink-4 max-sm:gap-x-3">
              <span className="shrink-0 font-mono text-ink-3">
                <Marked text={prFullRef(pr)} words={words} />
              </span>
              <Dot />
              <span className="inline-flex min-w-0 items-center gap-1.5">
                <ProjectMark
                  project={projects.get(s.project_id) ?? { id: s.project_id, name: s.project_slug }}
                  size={13}
                />
                <StatusDot status={s.status} className="scale-75" />
                <span className="truncate">
                  {s.project_slug}#{s.number}
                  {others.length > 0 && ` and ${others.length} more`}
                </span>
                {s.archived && (
                  <span
                    title="Its session was archived. The pull request stays here until it's merged or closed."
                    className="shrink-0 rounded border border-line px-1 text-[10.5px] leading-4 text-ink-4"
                  >
                    Archived
                  </span>
                )}
              </span>
              <Checks pr={pr} />
              <Review pr={pr} />
              <Size size={sizeOf(l)} />
            </div>
          </div>
          <People people={peopleOf(l)} />
          <time
            dateTime={new Date(at).toISOString()}
            title={`${WHEN_SAYS[tab]} ${stamp(at)}`}
            className="w-9 shrink-0 pt-0.5 text-right text-[12px] text-ink-4 tabular-nums"
          >
            {agoShort(at)}
          </time>
        </Link>
        <a
          href={pr.url}
          target="_blank"
          rel="noreferrer"
          aria-label={`Open ${prFullRef(pr)} on GitHub`}
          title="Open it on GitHub"
          className="mt-2.5 mr-1 ml-0.5 flex size-7 shrink-0 items-center justify-center rounded-md text-ink-4 transition-[color,background-color] duration-150 group-hover:text-ink-3 hover:bg-hover hover:text-ink"
        >
          <ArrowUpRight className="size-3.5" />
        </a>
      </div>
      {asking?.question && <Ask s={asking} question={asking.question} />}
    </li>
  );
}

/** Between the marks of a row's second line; on phones, where it wraps, the gaps do instead. */
const Dot = ({ className }: { className?: string }) => (
  <span aria-hidden="true" className={cn("max-sm:hidden", className)}>
    ·
  </span>
);

/** The filter's words in `text`, marked. */
function Marked({ text, words }: { text: string; words: string[] }) {
  if (!words.length) return text;
  const pattern = new RegExp(
    `(${words.map((w) => w.replace(/[.*+?^${}()|[\]\\]/g, "\\$&")).join("|")})`,
    "gi",
  );
  return text.split(pattern).map((part, i) =>
    // Split on a group: the matches are every other part.
    i % 2 === 1 ? (
      <mark key={i} className="rounded-[3px] bg-highlight/30 px-px text-ink">
        {part}
      </mark>
    ) : (
      part
    ),
  );
}

/** Its checks: how many passed, failed or still run, and while they run, how far they got. */
function Checks({ pr }: { pr: G.PullRequestRef }) {
  const text = checksText(pr);
  const c = pr.checks;
  if (!text || !c) return null;
  const total = c.passed + c.failed + c.pending;
  return (
    <>
      <Dot />
      <span className="inline-flex shrink-0 items-center gap-1.5">
        <PullRequestChecksIcon pr={pr} className="size-3" />
        {text}
        {checksState(pr) === "pending" && total > 0 && (
          <span aria-hidden="true" className="flex h-[3px] w-8 overflow-hidden rounded-full bg-line">
            <span className="h-full bg-checks-passed" style={{ width: `${(c.passed / total) * 100}%` }} />
            <span className="h-full bg-checks-failed" style={{ width: `${(c.failed / total) * 100}%` }} />
          </span>
        )}
      </span>
    </>
  );
}

/**
 * fctl's review of it, in a few words (in full where the pointer rests), under its eye, so that it
 * reads apart from the checks: watching while it waits or reads, green once it found nothing.
 */
function Review({ pr }: { pr: G.PullRequestRef }) {
  const review = prReview(pr);
  if (!review) return null;
  const look = prReviewLook(review);
  const said: Record<ReviewLook, [string, ReactNode]> = {
    waiting: ["Review waits for a runner", <EyeIcon key="eye" className="size-3 text-ink-4" />],
    reviewing: ["Reviewing", <EyeIcon key="eye" reading className="size-3 text-accent" />],
    passed: ["Reviewed, no issues", <EyeIcon key="eye" className="size-3 text-checks-passed" />],
    commented: [
      review.findings > 0
        ? `Review found ${issues(review.findings)}`
        : review.still_open > 0
          ? `${issues(review.still_open)} still open`
          : "Review found issues",
      <ReviewIcon key="comment" kind="comment" className="size-3 text-blue" />,
    ],
    failed: ["Review failed", <EyeIcon key="eye" className="size-3 text-danger" />],
    superseded: ["Review stopped", <EyeIcon key="eye" className="size-3 text-ink-4" />],
    stopped: ["Review stopped", <EyeIcon key="eye" className="size-3 text-ink-4" />],
  };
  const [says, icon] = said[look];
  return (
    <>
      <Dot className="max-sm:hidden" />
      <span
        title={prReviewText(review)}
        className={cn(
          "inline-flex shrink-0 items-center gap-1 max-sm:hidden",
          look === "commented" && "text-blue",
        )}
      >
        {icon}
        {says}
      </span>
    </>
  );
}

/** How much it changes, for its reviewer: lines added and removed (files in full where the pointer rests). */
function Size({ size }: { size?: PullSize }) {
  if (!size) return null;
  return (
    <>
      <Dot className="max-sm:hidden" />
      <span
        title={`${size.files} ${size.files === 1 ? "file" : "files"} changed`}
        className="shrink-0 tabular-nums max-sm:hidden"
      >
        <span className="text-added/80">+{size.additions}</span>{" "}
        <span className="text-danger/80">−{size.deletions}</span>
      </span>
    </>
  );
}

/** Who started its sessions: up to three faces, overlapping. */
function People({ people }: { people: G.UserRef[] }) {
  if (!people.length) return null;
  const names = people.map((p) => p.name);
  return (
    <span
      title={`Started by ${names.length > 1 ? `${names.slice(0, -1).join(", ")} and ${names.at(-1)}` : names[0]}`}
      className="flex shrink-0 -space-x-1.5 max-sm:hidden"
    >
      {people.slice(0, 3).map((p) => (
        <Avatar key={p.id} user={p} size={20} ring />
      ))}
    </span>
  );
}

/**
 * What its agent asks, under the row: the question, and the answers it offers, each sent as the
 * reply as it is, for those who may message the agent; a link to write another in the session.
 * Answered, the question goes and the row moves on as the feed says so.
 */
function Ask({ s, question }: { s: G.Session; question: G.SessionQuestion }) {
  const [sending, setSending] = useState<string>();
  const [sent, setSent] = useState<string>();
  const options = question.options ?? [];
  const canAnswer = (s.role === "editor" || s.role === "owner") && s.status !== "failed";
  const answer = async (text: string) => {
    if (sending || sent) return;
    setSending(text);
    try {
      await api.sendMessage(s.id, { text, client_id: `pulls-${s.id}-${question.asked_at}` });
      setSent(text);
      toast(`Sent to ${s.project_slug}#${s.number}: the agent reads it at its next step`);
    } catch (e) {
      toast(e instanceof Error ? e.message : "Not sent", "error");
    } finally {
      setSending(undefined);
    }
  };
  return (
    <div className="mr-10 mb-3 ml-[42px] rounded-lg border border-blue/25 bg-[linear-gradient(to_bottom,var(--c-blue-soft),transparent)] px-3 py-2.5 max-sm:mr-3">
      <p className="flex items-start gap-2 text-[13px] leading-relaxed text-ink">
        <MessageCircleQuestion aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-blue" />
        <span className="min-w-0">
          <span className="sr-only">
            {s.project_slug}#{s.number} asks:{" "}
          </span>
          <span className="line-clamp-3 whitespace-pre-line [overflow-wrap:anywhere]">{question.text}</span>
        </span>
        <span className="ml-auto shrink-0 pt-px text-[11.5px] text-ink-4" title={stamp(question.asked_at)}>
          {agoShort(question.asked_at)}
        </span>
      </p>
      <div className="mt-2 flex flex-wrap items-center gap-1.5 pl-[22px]">
        {canAnswer &&
          options.map((o) => (
            <button
              key={o}
              type="button"
              disabled={!!sending || !!sent}
              onClick={() => void answer(o)}
              className={cn(
                "inline-flex min-h-7 max-w-full items-center gap-1.5 rounded-full border px-3 py-0.5 text-left text-[12.5px] transition-[background-color,border-color,color,transform,opacity] duration-150 active:scale-[0.97]",
                sending === o || sent === o
                  ? "border-blue/50 bg-blue-soft text-ink"
                  : "border-line-strong bg-raised text-ink-2 hover:border-blue/45 hover:bg-blue-soft hover:text-ink disabled:opacity-50",
              )}
            >
              {sending === o && <Spinner className="size-3" />}
              {sent === o && <ChecksIcon state="passed" className="size-3 text-checks-passed" />}
              <span className="[overflow-wrap:anywhere]">{o}</span>
            </button>
          ))}
        <Link
          to="/s/$id"
          params={{ id: s.id }}
          className="rounded px-1 text-[12px] text-ink-3 underline-offset-2 transition-colors hover:text-ink hover:underline"
        >
          {canAnswer ? (options.length ? "Answer otherwise" : "Answer in the session") : "Open the session"}
        </Link>
      </div>
    </div>
  );
}

/** A row as it loads: its marks' places, so that the list doesn't jump as it comes. */
function RowSkeleton({ wide }: { wide: boolean }) {
  return (
    <li className="flex items-start gap-3.5 border-b border-line py-3 pr-11 pl-3">
      <Skeleton className="mt-[3px] size-4 rounded-full" />
      <div className="min-w-0 flex-1">
        <Skeleton className={cn("h-4", wide ? "w-3/5" : "w-2/5")} />
        <Skeleton className={cn("mt-2 h-3", wide ? "w-2/5" : "w-1/2")} />
      </div>
      <Skeleton className="size-5 rounded-full max-sm:hidden" />
      <Skeleton className="mt-0.5 h-3 w-7" />
    </li>
  );
}

function Nothing({
  state,
  q,
  mine,
  scope,
  counts,
  onPick,
  onClear,
}: {
  state: PullState;
  q?: string;
  mine?: boolean;
  /** The workspace or project it is narrowed to, by name. */
  scope?: string;
  /** What each tab has, as narrowed. */
  counts: Record<PullState, number>;
  onPick: (s: PullState) => void;
  onClear: () => void;
}) {
  const narrowed = !!q || !!mine || !!scope;
  if (narrowed) {
    // "Nothing open from your sessions in Shop matches “safari”."
    const said = [
      `Nothing ${state}`,
      mine && "from your sessions",
      scope && `in ${scope}`,
      q && `matches “${q}”`,
    ]
      .filter(Boolean)
      .join(" ");
    const elsewhere = PULL_STATES.filter((s) => s !== state && counts[s] > 0);
    return (
      <Empty
        icon={<Search className="size-6" />}
        title="None here"
        action={
          <span className="flex flex-wrap justify-center gap-2">
            {elsewhere.map((s) => (
              <Button key={s} size="sm" onClick={() => onPick(s)}>
                See {counts[s]} {TITLES[s].toLowerCase()}
              </Button>
            ))}
            {q && (
              <Button size="sm" variant="ghost" onClick={onClear}>
                Clear the filter
              </Button>
            )}
          </span>
        }
      >
        {said}.
      </Empty>
    );
  }
  return (
    <Empty
      icon={
        <PullRequestIcon
          state={state === "merged" ? "merged" : state === "closed" ? "closed" : "open"}
          className="size-6"
        />
      }
      title={
        state === "open"
          ? "No open pull requests"
          : state === "merged"
            ? "Nothing merged yet"
            : "Nothing closed"
      }
    >
      {state === "open"
        ? "Agents open pull requests as they finish their work. They show here, with their checks and what each needs next."
        : "Pull requests show here once they're merged or closed, newest first."}
    </Empty>
  );
}

/**
 * j and k (or the arrows) move through the rows, o opens the focused one on GitHub and c copies its
 * link. Enter opens it, as a link does. Not while typing.
 */
function useListKeys(list: React.RefObject<HTMLDivElement | null>) {
  const rows = () => [...(list.current?.querySelectorAll<HTMLElement>("[data-row]") ?? [])];
  // The row the focus is on: its link, the GitHub arrow beside it, or what its agent asks.
  const focused = () => {
    const row = (document.activeElement as HTMLElement | null)
      ?.closest("li")
      ?.querySelector<HTMLElement>("[data-row]");
    return row && rows().includes(row) ? row : undefined;
  };
  const move = (by: number) => {
    const all = rows();
    if (!all.length) return;
    const at = all.indexOf(focused() as HTMLElement);
    const next = all[Math.max(0, Math.min(all.length - 1, at < 0 ? 0 : at + by))];
    next?.focus();
    next?.scrollIntoView({ block: "nearest" });
  };
  useHotkey("j", () => move(1));
  useHotkey("k", () => move(-1));
  useHotkey("o", () => {
    const url = focused()?.dataset.github;
    if (url) window.open(url, "_blank", "noopener,noreferrer");
  });
  useHotkey("c", () => {
    const row = focused();
    const url = row?.dataset.github;
    if (!url) return;
    navigator.clipboard?.writeText(url).then(
      () => toast(`Copied the link to ${row.dataset.ref ?? "it"}`),
      () => toast("The link didn't copy", "error"),
    );
  });
  // The arrows move only within the list, so that they scroll the page everywhere else.
  return (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    if (!focused()) return;
    e.preventDefault();
    move(e.key === "ArrowDown" ? 1 : -1);
  };
}

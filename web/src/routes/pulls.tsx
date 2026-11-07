// Pull requests: what agents opened in your projects, each once, sorted by what it needs next:
// review, a fix for its checks, its agent to finish, its turn in a merge queue. The sessions
// that list them come from the sessions API (`?pulls=any`), which the feed keeps current, so
// states, checks and reviews change here as GitHub tells the control plane.
//
// The keyboard: j and k (or the arrows) move through the list, Enter opens the session, o opens
// the pull request on GitHub, and / filters.

import { useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { ArrowUpRight, FolderGit2, Layers, Search, X } from "lucide-react";
import { type ReactNode, useMemo, useRef } from "react";
import {
  PullRequestChecksIcon,
  PullRequestReviewMark,
  PullRequestStateIcon,
} from "@/components/session/pull-request";
import {
  Avatar,
  Empty,
  Kbd,
  PullRequestIcon,
  Select,
  type SelectOption,
  Skeleton,
  StatusDot,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { agoShort, stamp } from "@/lib/format";
import { useHotkey } from "@/lib/hotkeys";
import {
  countStates,
  type ListedPull,
  listPulls,
  moved,
  NEEDS,
  type Need,
  needOf,
  PULL_STATES,
  type PullState,
  pullMatches,
  stateOf,
} from "@/lib/pull-list";
import { checksText, prFullRef, prLabel, prReview, prReviewText } from "@/lib/pulls";
import { keys, useProjects, useWorkspaces } from "@/lib/queries";

/** What the page's address says it shows. */
export interface PullsSearch {
  state?: PullState;
  /** `w:<slug>` for a workspace's, `p:<slug>` for a project's; none for all. */
  scope?: string;
  mine?: boolean;
  q?: string;
}

/** Sessions the page looks through, newest first. */
const LOOK_AT = 200;

const TITLES: Record<PullState, string> = { open: "Open", merged: "Merged", closed: "Closed" };

export function PullsPage() {
  const search = useSearch({ from: "/app/pulls" });
  const navigate = useNavigate({ from: "/pulls" });
  const state = search.state ?? "open";
  const [kind, slug] = (search.scope ?? "").split(":");
  const scope = { workspace: kind === "w" ? slug : undefined, project: kind === "p" ? slug : undefined };
  // Every session that lists one, whatever its state: each pull request takes the state the session
  // that heard of it last says, which one that heard of it earlier may not.
  const q = { pulls: "any" as const, limit: LOOK_AT, ...scope, mine: search.mine };
  const sessions = useQuery({ queryKey: keys.sessions(q), queryFn: () => api.sessions(q) });
  const all = useMemo(() => listPulls(sessions.data?.sessions ?? []), [sessions.data]);
  const counts = useMemo(() => countStates(all), [all]);
  const shown = useMemo(
    () => all.filter((l) => stateOf(l) === state && pullMatches(l, search.q ?? "")),
    [all, state, search.q],
  );
  const set = (next: Partial<PullsSearch>) =>
    void navigate({ search: (s: PullsSearch) => ({ ...s, ...next }), replace: true });
  const filter = useRef<HTMLInputElement>(null);
  const list = useRef<HTMLDivElement>(null);
  useHotkey("/", () => filter.current?.focus());
  const onListKey = useListKeys(list);

  return (
    <div className="mx-auto max-w-[920px] px-6 pt-12 pb-24">
      <header className="animate-rise">
        <h1 className="flex items-center gap-3 font-display text-[38px] leading-[1.1] font-medium tracking-[-0.022em]">
          Pull requests
        </h1>
        <p className="mt-2 max-w-2xl text-[15px] leading-relaxed text-ink-3">
          What agents opened in {scopeName(search.scope)}, sorted by what each needs next. Their states and
          checks change here as GitHub reports them.
        </p>
      </header>

      <div className="mt-8 flex flex-wrap items-end justify-between gap-x-4 gap-y-3 border-b border-line">
        <nav aria-label="States" className="-mb-px flex gap-1">
          {PULL_STATES.map((s) => (
            <button
              key={s}
              type="button"
              aria-pressed={state === s}
              onClick={() => set({ state: s === "open" ? undefined : s })}
              className={cn(
                "relative flex items-center gap-2 px-3 py-2.5 text-[13.5px] transition-colors",
                state === s ? "text-ink" : "text-ink-3 hover:text-ink",
              )}
            >
              {TITLES[s]}
              <span
                className={cn(
                  "min-w-5 rounded-full px-1.5 text-center text-[11px] tabular-nums",
                  state === s ? "bg-hover text-ink-2" : "text-ink-4",
                )}
              >
                {sessions.isPending ? "·" : counts[s]}
              </span>
              {state === s && (
                <span className="absolute inset-x-2 -bottom-px h-[2px] rounded-full bg-accent" />
              )}
            </button>
          ))}
        </nav>
        <div className="flex w-full items-center gap-2 pb-2 sm:w-auto">
          <label className="flex h-7 min-w-0 flex-1 items-center gap-2 rounded-md border border-line bg-sunken/60 px-2 text-[12.5px] transition-colors focus-within:border-line-strong sm:w-56 sm:flex-none">
            <Search className="size-3.5 shrink-0 text-ink-4" />
            <input
              ref={filter}
              value={search.q ?? ""}
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
            {search.q ? (
              <button
                type="button"
                aria-label="Clear the filter"
                onClick={() => set({ q: undefined })}
                className="rounded p-0.5 text-ink-4 hover:text-ink"
              >
                <X className="size-3" />
              </button>
            ) : (
              <Kbd>/</Kbd>
            )}
          </label>
          <ScopeSelect value={search.scope} onChange={(scope) => set({ scope })} />
          <button
            type="button"
            aria-pressed={!!search.mine}
            onClick={() => set({ mine: search.mine ? undefined : true })}
            title="Only those of sessions you started"
            className={cn(
              "h-7 shrink-0 rounded-md border px-2.5 text-[12.5px] transition-colors",
              search.mine
                ? "border-accent/45 bg-accent-soft text-ink"
                : "border-line text-ink-3 hover:border-line-strong hover:bg-hover hover:text-ink",
            )}
          >
            Mine
          </button>
        </div>
      </div>

      {/* biome-ignore lint/a11y/noStaticElementInteractions: the arrows move between the rows' links. */}
      <div ref={list} className="mt-6" onKeyDown={onListKey}>
        {sessions.isPending ? (
          <div className="flex flex-col gap-3 pt-2">
            {[0, 1, 2].map((i) => (
              <Skeleton key={i} className="h-14" />
            ))}
          </div>
        ) : shown.length === 0 ? (
          <Nothing state={state} filtered={!!search.q || !!search.mine || !!search.scope} />
        ) : state === "open" ? (
          NEEDS.map(({ need, title, hint }) => {
            const these = shown.filter((l) => needOf(l) === need);
            return these.length ? (
              <Section key={need} title={title} hint={hint} n={these.length} need={need}>
                {these.map((l) => (
                  <PullRow key={l.pr.url} l={l} />
                ))}
              </Section>
            ) : null;
          })
        ) : (
          byWhen(shown).map(([title, these]) => (
            <Section key={title} title={title} n={these.length}>
              {these.map((l) => (
                <PullRow key={l.pr.url} l={l} />
              ))}
            </Section>
          ))
        )}
        {sessions.data?.next_before && (
          <p className="mt-6 text-center text-[12.5px] text-ink-4">
            From the newest {LOOK_AT} sessions with pull requests. Narrow it to a project for older ones.
          </p>
        )}
      </div>
      <p className="mt-10 hidden text-center text-[12px] text-ink-4 md:block">
        <Kbd>j</Kbd> <Kbd>k</Kbd> move · <Kbd>↵</Kbd> opens the session · <Kbd>o</Kbd> opens it on GitHub ·{" "}
        <Kbd>/</Kbd> filters
      </p>
    </div>
  );
}

/** "your projects", or the workspace's or project's name as the scope says. */
function scopeName(scope: string | undefined): string {
  if (!scope) return "your projects";
  const [kind, slug] = scope.split(":");
  return kind === "w" ? `${slug}'s projects` : `${slug}`;
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
          icon: <FolderGit2 className="size-3.5" />,
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

function Section({
  title,
  hint,
  n,
  need,
  children,
}: {
  title: string;
  hint?: string;
  n: number;
  need?: Need;
  children: ReactNode;
}) {
  return (
    <section className="mb-8 animate-rise last:mb-0">
      <h2 className="mb-1 flex items-baseline gap-2 px-3 text-[13px] font-semibold text-ink">
        <span className={cn(need === "failing" && "text-danger", need === "ready" && "text-ink")}>
          {title}
        </span>
        <span className="text-[12px] font-normal text-ink-4 tabular-nums">{n}</span>
        {hint && (
          <span className="hidden truncate text-[12px] font-normal text-ink-4 sm:inline">· {hint}</span>
        )}
      </h2>
      <ul aria-label={title} className="border-t border-line">
        {children}
      </ul>
    </section>
  );
}

/** Merged and closed ones by when that happened: today, yesterday, this week, earlier. */
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
  const out = new Map<string, ListedPull[]>();
  for (const l of list) {
    const t = l.pr.state_changed_at ?? moved(l);
    const [name] = buckets.find(([, fits]) => fits(t)) ?? ["Earlier"];
    out.set(name, [...(out.get(name) ?? []), l]);
  }
  return [...out.entries()];
}

/**
 * A pull request: its state, title and repository, the sessions that list it, its checks and
 * fctl's review, who started its session and when it last moved. The row opens its newest session;
 * the arrow, the pull request on GitHub.
 */
function PullRow({ l }: { l: ListedPull }) {
  const { pr, sessions } = l;
  const [s, ...others] = sessions;
  const checks = checksText(pr);
  const review = prReview(pr);
  const when = moved(l);
  if (!s) return null;
  return (
    <li className="group flex items-start border-b border-line last:border-b-0">
      <Link
        to="/s/$id"
        params={{ id: s.id }}
        data-row
        aria-label={`${pr.title ?? s.title}: ${prLabel(pr)}`}
        data-github={pr.url}
        className="flex min-w-0 flex-1 items-start gap-3.5 rounded-md py-3 pr-1 pl-3 outline-none transition-colors hover:bg-hover/40 focus-visible:bg-hover/60 focus-visible:shadow-[inset_0_0_0_1.5px_var(--c-accent)]"
      >
        <PullRequestStateIcon pr={pr} className="mt-[3px] size-4" />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14.5px] text-ink">{pr.title ?? s.title}</div>
          <div className="mt-1 flex min-w-0 flex-wrap items-center gap-x-2 gap-y-0.5 text-[12px] text-ink-4">
            <span className="shrink-0 font-mono text-ink-3">{prFullRef(pr)}</span>
            <span aria-hidden="true">·</span>
            <span className="inline-flex min-w-0 items-center gap-1.5">
              <StatusDot status={s.status} className="scale-75" />
              <span className="truncate">
                {s.project_slug}#{s.number}
                {others.length > 0 && ` and ${others.length} more`}
              </span>
            </span>
            {checks && (
              <>
                <span aria-hidden="true">·</span>
                <span className="inline-flex shrink-0 items-center gap-1">
                  <PullRequestChecksIcon pr={pr} className="size-3" />
                  {checks}
                </span>
              </>
            )}
            {review && (
              <>
                <span aria-hidden="true">·</span>
                <span className="inline-flex shrink-0 items-center gap-1 max-sm:hidden">
                  <PullRequestReviewMark pr={pr} className="size-3" />
                  {prReviewText(review)}
                </span>
              </>
            )}
          </div>
        </div>
        <Avatar user={s.created_by} size={20} />
        <time
          dateTime={new Date(when).toISOString()}
          title={stamp(when)}
          className="w-9 shrink-0 pt-0.5 text-right text-[12px] text-ink-4 tabular-nums"
        >
          {agoShort(when)}
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
    </li>
  );
}

function Nothing({ state, filtered }: { state: PullState; filtered: boolean }) {
  if (filtered)
    return (
      <Empty icon={<Search className="size-6" />} title="None here">
        Nothing matches the filter, the projects or Mine. Clear them to see the rest.
      </Empty>
    );
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
 * j and k (or the arrows) move through the rows, o opens the focused one on GitHub. Enter opens it,
 * as a link does. Not while typing.
 */
function useListKeys(list: React.RefObject<HTMLDivElement | null>) {
  const rows = () => [...(list.current?.querySelectorAll<HTMLElement>("[data-row]") ?? [])];
  const move = (by: number) => {
    const all = rows();
    if (!all.length) return;
    const at = all.indexOf(document.activeElement as HTMLElement);
    const next = all[Math.max(0, Math.min(all.length - 1, at < 0 ? 0 : at + by))];
    next?.focus();
    next?.scrollIntoView({ block: "nearest" });
  };
  useHotkey("j", () => move(1));
  useHotkey("k", () => move(-1));
  useHotkey("o", () => {
    const url = (document.activeElement as HTMLElement | null)?.dataset.github;
    if (url) window.open(url, "_blank", "noopener,noreferrer");
  });
  // The arrows move only within the list, so that they scroll the page everywhere else.
  return (e: React.KeyboardEvent) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    if (!rows().includes(document.activeElement as HTMLElement)) return;
    e.preventDefault();
    move(e.key === "ArrowDown" ? 1 : -1);
  };
}

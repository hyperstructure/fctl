// A project's schedules: tasks fctl starts a session for on its own, once at a set time, again and
// again, or each time something happens on GitHub (an issue opened, a pull request merged…), on the
// Claude subscription of the person each runs for. Everyone who sees the project sees them and what
// each run started; people who start sessions set them up and run one now, on their own subscription;
// a schedule's owner, and the project's maintainers and owners, change, pause and delete it (changing
// someone else's makes it theirs).

import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  ArrowRight,
  ArrowUpRight,
  BookMarked,
  CalendarClock,
  ChevronDown,
  CircleCheck,
  CircleDot,
  CircleX,
  Ellipsis,
  GraduationCap,
  Package,
  Pause,
  Pencil,
  Play,
  Plus,
  Repeat,
  Rocket,
  Siren,
  SkipForward,
  Tag,
  Trash2,
  TriangleAlert,
  UserRoundCheck,
  Webhook,
  Zap,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { GithubAccessIcon } from "@/components/github-access";
import { StartChoices } from "@/components/session/start";
import {
  Avatar,
  Banner,
  Button,
  CopyButton,
  Dialog,
  Field,
  IconButton,
  Input,
  Menu,
  MenuItem,
  MenuSeparator,
  Meta,
  PullRequestIcon,
  Select,
  type SelectOption,
  Skeleton,
  StatusDot,
  Switch,
  Textarea,
  TextareaControls,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago, agoShort, stamp } from "@/lib/format";
import { githubLevel, lowerAccess, onGithub, tracksTickets } from "@/lib/github-access";
import { siteUrl } from "@/lib/install";
import { asked, type Choice, effortLabel, modelLabel, projectChoice } from "@/lib/models";
import { keys, queryClient, useMe, useProjectRepos } from "@/lib/queries";
import {
  browserZone,
  canSchedule,
  clockOf,
  cronProblem,
  cronRepeat,
  DEFAULT_REPEAT,
  describeCron,
  describeEvent,
  EVENT_KINDS,
  EVENT_NAMES,
  type Every,
  eventBy,
  eventSaid,
  fromWall,
  moment,
  nameFromTask,
  nextRuns,
  ordinal,
  orList,
  parseCron,
  type Repeat as RepeatChoice,
  readRunOn,
  repeatCron,
  runOnContext,
  runOutcome,
  runTrigger,
  scheduleRights,
  scheduleWhen,
  sentence,
  takesBranches,
  takesBy,
  takesLabels,
  WEEKDAY_NAMES,
  wallParts,
  zoneCity,
  zoneOffset,
  zoneRegion,
  zones,
} from "@/lib/schedules";

// ───────────────────────────── ideas ─────────────────────────────

/** A schedule to start from: what it asks of its agents, and when: at times, or on an event. */
interface Idea {
  key: string;
  name: string;
  /** What it does, in a line. */
  line: string;
  icon: ReactNode;
  prompt: string;
  repeat?: RepeatChoice;
  /** What it waits for on GitHub, for one that runs on an event rather than at times. */
  event?: G.ScheduleEvent;
  /** What its agents may do on GitHub, when less than the project allows does. */
  github?: G.GithubAccess;
  /**
   * It works on the project's tickets: only where its agents reach a tracker (`any`), or that tracker
   * (GitHub's issues start events; Linear's issues come to a schedule only at its times).
   */
  tickets?: "any" | "github" | "linear";
  /** It runs often enough that its sessions are best kept off session lists. */
  hide?: boolean;
}

/**
 * The ideas for `project`: those about tickets only where its agents reach the tracker they need, and
 * those that wait for events only where they work on GitHub.
 */
function ideasFor(project: G.Project): Idea[] {
  const tracker = project.settings.tickets?.tracker ?? "github";
  return IDEAS.filter(
    (i) =>
      (!i.tickets || (i.tickets === "any" ? tracksTickets(project) : tracker === i.tickets)) &&
      (!i.event || onGithub(project)),
  );
}

/** When an idea runs, in words: "Every Monday at 09:00", "When an issue is opened". */
const ideaWhen = (i: Idea) =>
  i.event
    ? sentence(describeEvent(i.event))
    : sentence(describeCronText(repeatCron(i.repeat ?? DEFAULT_REPEAT)));

const at = (every: Every, hour: number, weekdays: number[] = [1]): RepeatChoice => ({
  ...DEFAULT_REPEAT,
  every,
  hour,
  minute: 0,
  weekdays,
});

const IDEAS: Idea[] = [
  {
    key: "triage",
    name: "Triage new issues",
    line: "Labels, questions and links for each issue as it's opened.",
    icon: <CircleDot />,
    prompt:
      "Triage the issue that was just opened. Add the labels that fit, from those the tracker has. Where something is missing to act on it, ask for it in a comment; say there which issues it duplicates and which code it concerns. Leave it alone when someone on the team already answered it.",
    event: { kind: "issue_opened" },
    // Labels and comments, and nothing else: an issue that asks for more can't get it.
    github: "triage",
    tickets: "github",
    hide: true,
  },
  {
    // Linear's issues come to schedules only at their times.
    key: "triage-daily",
    name: "Triage new issues",
    line: "Labels, questions and links for each issue opened since the last run.",
    icon: <CircleDot />,
    prompt:
      "Triage the issues opened since the previous run. For each, add the labels that fit, from those the tracker has. Where something is missing to act on it, ask for it in a comment, and say there which issues it duplicates and which code it concerns.",
    repeat: at("weekday", 8),
    github: "triage",
    tickets: "linear",
  },
  {
    key: "labeled",
    name: "Work on labeled issues",
    line: "Label an issue fctl to hand it to an agent, which opens a pull request.",
    icon: <Tag />,
    prompt:
      "Work on the issue that was just labeled: read it and its comments, find the code it concerns, make the change it asks for, test it, and open a pull request that says Fixes and the issue's number. Where it's unclear what it asks, or it takes a decision only a person can make, ask in a comment on the issue instead, and stop.",
    event: { kind: "issue_labeled", labels: ["fctl"] },
    tickets: "github",
  },
  {
    key: "merged",
    name: "Learn from merged pull requests",
    line: "What people said in the review of each agent's pull request, kept in AGENTS.md or a skill.",
    icon: <EventIcon kind="pull_request_merged" />,
    prompt:
      "Read the review of the pull request that just merged: what people wrote on it and on its lines, the changes they asked for, and the commits that answered them. Where it shows something agents working here should know or do differently (a convention, a command, a trap), and AGENTS.md or the skills don't say it yet, open a pull request that adds it where it belongs, in the voice the file already has. When people said nothing worth keeping, change nothing and say so in a sentence.",
    event: { kind: "pull_request_merged", by: "agents" },
    hide: true,
  },
  {
    key: "checks",
    name: "Fix failing checks on main",
    line: "When the default branch's checks fail, an agent finds why and opens a fix.",
    icon: <Siren />,
    prompt:
      "The checks of the newest commit on the default branch failed. Find out why: read the failed runs' logs (gh run view --log-failed) and the commits since the checks last passed. When a pull request already fixes it, or a run again passes (a flaky test), say so and stop. Otherwise fix the cause rather than the test, and open a pull request that says what broke and why.",
    event: { kind: "checks_failed" },
  },
  {
    key: "release",
    name: "Try each release",
    line: "Each release installed and tried as people would, and what doesn't work filed.",
    icon: <Rocket />,
    prompt:
      "Try the release that was just published as people would: install it the way its README or install guide says, and try what its notes say changed. File an issue for each problem you find, with how to reproduce it. When all is well, say so in a sentence.",
    event: { kind: "release_published" },
  },
  {
    key: "dependencies",
    name: "Update dependencies",
    line: "A pull request with the week's patch and minor releases, tested.",
    icon: <Package />,
    prompt:
      "Update the dependencies that have new patch or minor releases. Run the tests and the linters, fix what the updates break, and open one pull request that lists each update. Leave major versions out: list them in the pull request's description, with what upgrading would take.",
    repeat: at("week", 9, [1]),
  },
  {
    key: "memory",
    name: "Tidy the project's memory",
    line: "Duplicate and outdated notes marked, overlapping ones merged.",
    icon: <BookMarked />,
    prompt:
      "Tidy the project's memory, which every agent starts with (fctl memory). Mark duplicates with fctl memory duplicate, and mark stale the notes the repository no longer bears out, after checking each, with fctl memory stale. Where notes overlap, add one that says all they do with fctl memory add --replaces. Change no code.",
    repeat: at("week", 16, [5]),
    github: "read",
  },
  {
    key: "lessons",
    name: "Learn from the week's reviews",
    line: "What agents keep tripping over, added to AGENTS.md or filed as issues.",
    icon: <GraduationCap />,
    prompt:
      "Read the review comments on this week's pull requests in this project's repositories, fctl's review agents' findings among them. Where agents keep making the same mistake or running into the same trouble, open a pull request that adds what they should know to AGENTS.md, or file an issue for what needs fixing. End with a short summary of what you found.",
    repeat: at("week", 15, [5]),
  },
];

/** What a new schedule's form starts with: blank, an idea, or a task from the Sessions tab. */
export interface Draft {
  name?: string;
  prompt?: string;
  repeat?: RepeatChoice;
  /** What it waits for on GitHub, for an idea that runs on an event. */
  event?: G.ScheduleEvent;
  /** Its sessions stay off session lists. */
  hide?: boolean;
  model?: string;
  effort?: string;
  github?: G.GithubAccess;
  /** Once a schedule is set up with it: the task box it came from forgets the draft it kept. */
  onSaved?: () => void;
}

/** Tasks handed over from a task box, by project id, for the form to start with. */
const handed = new Map<string, Draft>();

/** Hands a task to the project's Schedules tab, which opens its form with it. */
export function handOver(project: string, draft: Draft) {
  handed.set(project, draft);
}

/**
 * Schedule, beside a task box's Start: the task, with the model, effort level and GitHub access
 * picked for it, goes to the project's Schedules tab, to run later or again and again. Schedules
 * take no files. On a phone it is its symbol alone, and at every width when `bare`.
 */
export function ScheduleButton({
  project,
  prompt,
  choice,
  files,
  disabled,
  bare,
  onSaved,
  className,
}: {
  project: Pick<G.Project, "id" | "slug">;
  prompt: string;
  choice: Choice;
  /** How many files the task has. */
  files: number;
  disabled?: boolean;
  bare?: boolean;
  onSaved?: () => void;
  className?: string;
}) {
  const navigate = useNavigate();
  return (
    <button
      type="button"
      title={
        files
          ? "Schedules take no files: start it now, or remove them to schedule it"
          : bare
            ? "Schedule: run this task later, again and again, or on an event"
            : "Run this task later, again and again, or each time something happens on GitHub"
      }
      disabled={!prompt.trim() || files > 0 || disabled}
      onClick={() => {
        handOver(project.id, {
          prompt,
          model: choice.model,
          effort: choice.effort,
          github: choice.github,
          onSaved,
        });
        void navigate({ to: "/p/$slug", params: { slug: project.slug }, search: { tab: "schedules" } });
      }}
      className={cn(
        "flex h-9 shrink-0 items-center gap-1.5 rounded-full text-[13px] font-medium text-ink-2 transition-colors duration-150 hover:bg-hover hover:text-ink disabled:text-ink-4 disabled:hover:bg-transparent",
        bare ? "w-9 justify-center" : "px-3 max-sm:px-2",
        className,
      )}
    >
      <CalendarClock className="size-4" aria-hidden="true" />
      <span className={bare ? "sr-only" : "max-sm:sr-only"}>Schedule</span>
    </button>
  );
}

const fromIdea = (i: Idea): Draft => ({
  name: i.name,
  prompt: i.prompt,
  repeat: i.repeat,
  event: i.event,
  hide: i.hide,
  github: i.github,
});

// ───────────────────────────── the tab ─────────────────────────────

/** Changes to a project's schedules: the list shows them at once. */
function saved(project: G.Project, s: G.Schedule, removed = false) {
  queryClient.setQueryData<G.ScheduleList>(keys.schedules(project.id), (list) => {
    if (!list) return list;
    const others = list.schedules.filter((x) => x.id !== s.id);
    if (removed) return { schedules: others };
    const at = list.schedules.findIndex((x) => x.id === s.id);
    const schedules = [...list.schedules];
    if (at >= 0) schedules[at] = s;
    else schedules.push(s);
    return { schedules };
  });
  void queryClient.invalidateQueries({ queryKey: keys.scheduleRuns(s.id) });
}

export function SchedulesTab({ project, open }: { project: G.Project; open?: string }) {
  const list = useQuery({
    queryKey: keys.schedules(project.id),
    queryFn: () => api.schedules(project.id),
    // Runs start on their own: what they started shows within the minute.
    refetchInterval: 30_000,
  });
  // Read on mount, and dropped after: React may call the initializer twice.
  const [draft, setDraft] = useState<Draft | undefined>(() => handed.get(project.id));
  useEffect(() => {
    handed.delete(project.id);
  }, [project.id]);
  const [opened, setOpened] = useState<string | undefined>(open);
  useEffect(() => {
    if (open) setOpened(open);
  }, [open]);
  const schedules = list.data?.schedules ?? [];
  if (list.isError) return <Banner tone="error">{list.error.message}</Banner>;
  if (!list.data)
    return (
      <div className="space-y-3">
        <Skeleton className="h-14" />
        <Skeleton className="h-14" />
      </div>
    );
  if (schedules.length === 0 && !draft) return <Intro project={project} onStart={setDraft} />;
  return (
    <div className="flex flex-col gap-6">
      {draft ? (
        <Editor
          key="new"
          project={project}
          draft={draft}
          onDone={(s) => {
            setDraft(undefined);
            if (s) setOpened(s.id);
          }}
        />
      ) : (
        <Header project={project} schedules={schedules} onNew={() => setDraft({})} />
      )}
      {schedules.length > 0 && (
        <ul className="border-t border-line">
          {schedules.map((s) => (
            <Item
              key={s.id}
              project={project}
              schedule={s}
              open={opened === s.id}
              onToggle={() => setOpened(opened === s.id ? undefined : s.id)}
            />
          ))}
        </ul>
      )}
    </div>
  );
}

/** How many there are, which runs next, and the button that sets up another. */
function Header({
  project,
  schedules,
  onNew,
}: {
  project: G.Project;
  schedules: G.Schedule[];
  onNew: () => void;
}) {
  const next = schedules
    .filter((s) => s.state === "active" && s.next_run_at)
    .sort((a, b) => (a.next_run_at ?? 0) - (b.next_run_at ?? 0))[0];
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-6 gap-y-3">
      <p className="min-w-0 text-[13.5px] leading-relaxed text-ink-3">
        {schedules.length === 1 ? "1 schedule" : `${schedules.length} schedules`}
        {next?.next_run_at && (
          <>
            {" · "}
            <span className="text-ink-2">{next.name}</span> runs next,{" "}
            <time dateTime={new Date(next.next_run_at).toISOString()} title={stamp(next.next_run_at)}>
              {soon(next.next_run_at, next.timezone)}
            </time>
          </>
        )}
      </p>
      {canSchedule(project.role) && (
        <Button variant="primary" icon={<Plus className="size-4" />} onClick={onNew}>
          New schedule
        </Button>
      )}
    </div>
  );
}

/** When a run comes: "in 25 min", "in 3 hours", or its day and time from half a day on. */
function soon(at: number, zone: string, now = Date.now()): string {
  const min = Math.round((at - now) / 60_000);
  if (min < 1) return "now";
  if (min < 60) return `in ${min} min`;
  const hours = Math.round(min / 60);
  if (min < 12 * 60) return hours === 1 ? "in an hour" : `in ${hours} hours`;
  return `on ${moment(at, zone, now)}`;
}

// ───────────────────────────── before there are any ─────────────────────────────

const POINTS = [
  {
    key: "when",
    icon: <Repeat />,
    text: "Every hour, day, weekday, week or month in your time zone, once at a set time, on any cron schedule, or only when asked.",
  },
  {
    key: "events",
    icon: <Zap />,
    text: "Each time something happens on GitHub: an issue is opened or labeled, a pull request opens or merges, a release is published, checks fail on main.",
  },
  {
    key: "skips",
    icon: <SkipForward />,
    text: "Runs don't pile up: one is skipped while the last is still at the same work, or while the plan it runs on is at its limit.",
  },
  {
    key: "run",
    icon: <Webhook />,
    text: (
      <>
        Run one when you choose, here or with{" "}
        <code className="font-mono text-[12.5px]">fctl schedules run</code>, and from other tools with an
        access token.
      </>
    ),
  },
];

/** What schedules do, for a project that has none, with ideas to start from. */
function Intro({ project, onStart }: { project: G.Project; onStart: (d: Draft) => void }) {
  const can = canSchedule(project.role);
  const ideas = ideasFor(project);
  // Those that wait for something first: they are what the project's agents can't do from a timetable.
  const groups = [
    { key: "events", label: "When something happens on GitHub", ideas: ideas.filter((i) => i.event) },
    { key: "times", label: "At set times", ideas: ideas.filter((i) => !i.event) },
  ].filter((g) => g.ideas.length > 0);
  return (
    <div className="flex animate-rise flex-col gap-12 py-2">
      <div className="grid items-center gap-x-12 gap-y-10 md:grid-cols-[minmax(0,1fr)_300px]">
        <div>
          <h2 className="font-display text-[28px] leading-[1.15] font-medium tracking-[-0.018em] text-ink">
            Sessions that start on their own
          </h2>
          <p className="mt-3 max-w-xl text-[15px] leading-relaxed text-ink-3">
            Give an agent a task, and a time or something to wait for: once, again and again, or each time it
            happens. Each run is a session of its own, on your Claude subscription, that you follow like any
            other.
          </p>
          <ul className="mt-6 flex max-w-xl flex-col gap-3">
            {POINTS.map((p) => (
              <li key={p.key} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
                <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-3 [&_svg]:size-4">
                  {p.icon}
                </span>
                {/* One item of the row, however many parts its text has (code among them). */}
                <span className="min-w-0">{p.text}</span>
              </li>
            ))}
          </ul>
        </div>
        {/* On a phone the ideas below say it, and come sooner. */}
        <div className="max-sm:hidden">
          <Preview />
        </div>
      </div>
      {can ? (
        <section aria-labelledby="ideas-heading">
          <div className="mb-3 flex flex-wrap items-end justify-between gap-3">
            <h3 id="ideas-heading" className="text-[14px] font-semibold text-ink">
              Start from an idea
            </h3>
            <Button size="sm" icon={<Plus className="size-3.5" />} onClick={() => onStart({})}>
              Start from scratch
            </Button>
          </div>
          <div className="flex flex-col gap-6">
            {groups.map((g) => (
              <div key={g.key}>
                <h4 id={`ideas-${g.key}`} className="mb-1.5 text-[12.5px] text-ink-4">
                  {g.label}
                </h4>
                <ul aria-labelledby={`ideas-${g.key}`} className="border-t border-line">
                  {g.ideas.map((i) => (
                    <li key={i.key}>
                      <button
                        type="button"
                        onClick={() => onStart(fromIdea(i))}
                        className="group flex w-full items-center gap-3.5 border-b border-line px-3 py-3 text-left transition-colors hover:bg-hover/40 focus-visible:bg-hover/40 focus-visible:outline-none"
                      >
                        <span
                          aria-hidden="true"
                          className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-raised text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line)] [&_svg]:size-4"
                        >
                          {i.icon}
                        </span>
                        <span className="min-w-0 flex-1">
                          <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
                            <span className="text-[14px] text-ink">{i.name}</span>
                            {i.github && onGithub(project) && (
                              <IdeaAccess access={lowerAccess(i.github, projectChoice(project).github)} />
                            )}
                          </span>
                          <span className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">{i.line}</span>
                          <span className="mt-1 block text-[12px] text-ink-4 sm:hidden">{ideaWhen(i)}</span>
                        </span>
                        <span className="max-w-[40%] shrink-0 text-right text-[12.5px] text-ink-4 max-sm:hidden">
                          {ideaWhen(i)}
                        </span>
                        <ArrowRight className="size-3.5 shrink-0 text-ink-4 transition-transform duration-200 group-hover:translate-x-0.5" />
                      </button>
                    </li>
                  ))}
                </ul>
              </div>
            ))}
          </div>
        </section>
      ) : (
        <p className="text-[13px] text-ink-4">People who start sessions in this project set up schedules.</p>
      )}
    </div>
  );
}

/** What an idea's agents may do on GitHub, when it holds them to less than pushing. */
function IdeaAccess({ access }: { access: G.GithubAccess }) {
  if (access === "push") return null;
  const level = githubLevel(access);
  return (
    <span className="inline-flex items-center gap-1 text-[12px] text-ink-4" title={level.description}>
      <GithubAccessIcon access={access} className="size-3" />
      {level.label}
    </span>
  );
}

/** A cron expression in words, in the browser's way of writing times. */
function describeCronText(text: string): string {
  const parsed = parseCron(text);
  return "error" in parsed ? text : describeCron(parsed.cron, clockOf);
}

/**
 * What a schedule is: one that runs at its times, through the week, and one that runs on an event, with
 * the issues that started its sessions.
 */
function Preview() {
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  const card = "overflow-hidden rounded-xl border border-line bg-surface";
  const head = (icon: ReactNode, name: string, when: string) => (
    <div className="flex items-center gap-2.5 border-b border-line px-3.5 py-3">
      <span className="flex size-7 items-center justify-center rounded-lg bg-raised text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line)] [&_svg]:size-3.5">
        {icon}
      </span>
      <div className="min-w-0">
        <p className="text-[13px] font-medium text-ink">{name}</p>
        <p className="text-[11.5px] text-ink-3">{when}</p>
      </div>
    </div>
  );
  return (
    <div aria-hidden="true" className="relative select-none max-md:max-w-sm">
      <div className={cn(card, "shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]")}>
        {head(<Repeat />, "Update dependencies", `Every Monday at ${clockOf(9, 0)}`)}
        <div className="grid grid-cols-7 gap-1 px-3.5 pt-3 pb-3.5">
          {days.map((d, i) => (
            <div key={d} className="flex flex-col items-center gap-1.5">
              <span className="text-[10.5px] text-ink-4">{d}</span>
              <span
                className={cn(
                  "flex h-7 w-full items-center justify-center rounded-md",
                  i === 0 ? "bg-highlight text-highlight-ink" : "bg-sunken/70",
                )}
              >
                {i === 0 && <Play className="size-3 fill-current" />}
              </span>
            </div>
          ))}
        </div>
        <div className="border-t border-line px-3.5 py-2.5 font-mono text-[11px] text-ink-4">0 9 * * 1</div>
      </div>
      <div className={cn(card, "mt-3 shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]")}>
        {head(<CircleDot />, "Triage new issues", "When an issue is opened")}
        <div className="flex flex-col gap-1.5 px-3.5 py-2.5 text-[12px]">
          {[
            ["#58", "Export fails on Safari", "running", "#212"],
            ["#57", "Add a dark mode", "idle", "#209"],
          ].map(([issue, title, status, session]) => (
            <div key={issue} className="flex items-center gap-2">
              <CircleDot className="size-3 shrink-0 text-pr-open" />
              <span className="font-mono text-ink-4">{issue}</span>
              <span className="min-w-0 flex-1 truncate text-ink-2">{title}</span>
              <ArrowRight className="size-3 shrink-0 text-ink-4" />
              <StatusDot status={status as G.SessionStatus} />
              <span className="font-mono text-ink-4">{session}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

// ───────────────────────────── a schedule ─────────────────────────────

/** How a schedule shows where it stands: its symbol, in the tone of its state. */
function Glyph({ schedule }: { schedule: G.Schedule }) {
  const trouble = !!schedule.problem || !!schedule.paused_reason;
  const icon =
    schedule.state === "done" ? (
      <CircleCheck />
    ) : trouble ? (
      <TriangleAlert />
    ) : schedule.event ? (
      <EventIcon kind={schedule.event.kind} />
    ) : schedule.cron ? (
      <Repeat />
    ) : schedule.at !== undefined ? (
      <CalendarClock />
    ) : (
      <Webhook />
    );
  return (
    <span
      aria-hidden="true"
      className={cn(
        "relative mt-px flex size-8 shrink-0 items-center justify-center rounded-lg bg-raised shadow-[inset_0_0_0_1px_var(--c-line)] [&_svg]:size-4",
        trouble ? "text-amber" : schedule.state === "active" ? "text-ink" : "text-ink-4",
      )}
    >
      {icon}
      {schedule.state === "active" && !trouble && (
        <span className="absolute -top-0.5 -right-0.5 size-2 rounded-full bg-checks-passed ring-2 ring-canvas" />
      )}
    </span>
  );
}

/**
 * The facts under a schedule's name: when it runs, when next, what its last run did, and what its
 * agents may do on GitHub when that is less than pushing, as a session says beside its status.
 */
function Facts({ project, schedule: s }: { project: G.Project; schedule: G.Schedule }) {
  const here = useMemo(browserZone, []);
  const last = s.last_run;
  const own = projectChoice(project).github;
  const github = lowerAccess(s.github_access ?? own, own);
  return (
    <Meta
      parts={[
        { key: "when", node: <span className="text-ink-3">{scheduleWhen(s)}</span> },
        onGithub(project) &&
          github !== "push" && {
            key: "github",
            node: (
              <span className="inline-flex items-center gap-1" title={githubLevel(github).description}>
                <GithubAccessIcon access={github} className="size-3" />
                {githubLevel(github).label}
              </span>
            ),
          },
        s.timezone !== here &&
          (!!s.cron || s.at !== undefined) && {
            key: "zone",
            node: zoneCity(s.timezone),
            className: "max-sm:hidden",
          },
        s.state === "paused"
          ? { key: "state", node: <span className="text-ink-3">Paused</span> }
          : s.state === "done"
            ? { key: "state", node: "Ran once" }
            : // One that runs once says when only where it waits past its time.
              !!s.next_run_at &&
              (s.at === undefined || s.next_run_at > s.at + 60_000) && {
                key: "next",
                node: (
                  <time
                    dateTime={new Date(s.next_run_at ?? 0).toISOString()}
                    title={stamp(s.next_run_at ?? 0)}
                  >
                    {s.at === undefined ? "Next" : "Waits, and runs"} {soon(s.next_run_at ?? 0, s.timezone)}
                  </time>
                ),
              },
        last && {
          key: "last",
          node: (
            <span className="inline-flex items-center gap-1" title={stamp(last.at)}>
              <RunIcon run={last} className="size-3" />
              {last.outcome === "started" ? "Last run" : `Last run ${runOutcome(last).toLowerCase()},`}{" "}
              {ago(last.at)}
            </span>
          ),
          className: "max-sm:hidden",
        },
      ]}
    />
  );
}

function RunIcon({ run, className }: { run: G.ScheduleRun; className?: string }) {
  if (run.outcome === "failed") return <CircleX className={cn("text-danger", className)} />;
  if (run.outcome === "skipped") return <SkipForward className={cn("text-amber", className)} />;
  return <CircleCheck className={cn("text-checks-passed", className)} />;
}

/** A schedule in the list: its row, which opens its details or its form. */
function Item({
  project,
  schedule: s,
  open,
  onToggle,
}: {
  project: G.Project;
  schedule: G.Schedule;
  open: boolean;
  onToggle: () => void;
}) {
  const { data: me } = useMe();
  const mine = !!me && s.owner?.id === me.user.id;
  const rights = scheduleRights(project.role, mine);
  const [editing, setEditing] = useState(false);
  const [dialog, setDialog] = useState<"delete" | "resume" | "take" | "run">();
  const ref = useRef<HTMLLIElement>(null);
  const panel = `schedule-${s.id}`;
  const run = useRunNow(s);
  const update = useMutation({
    mutationFn: (b: G.UpdateScheduleRequest) => api.updateSchedule(s.id, b),
    onSuccess: (n, b) => {
      saved(project, n);
      setDialog(undefined);
      toast(
        b.enabled === false
          ? "Paused: it starts nothing until it's resumed"
          : b.enabled
            ? n.next_run_at
              ? `Resumed: it runs next ${soon(n.next_run_at, n.timezone)}`
              : "Resumed"
            : "It runs on your Claude subscription now",
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  // Opened from a session's link: in view.
  useEffect(() => {
    if (open) ref.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
  }, [open]);
  const resume = () => {
    if (mine) update.mutate({ enabled: true });
    else setDialog("resume");
  };
  return (
    <li ref={ref} className="border-b border-line">
      <div
        className={cn(
          "group relative flex items-start gap-3.5 px-3 py-3.5 transition-colors",
          open ? "bg-hover/20" : "hover:bg-hover/30",
        )}
      >
        <Glyph schedule={s} />
        <button
          type="button"
          aria-expanded={open}
          aria-controls={panel}
          onClick={onToggle}
          className="min-w-0 flex-1 text-left outline-none after:absolute after:inset-0 after:rounded focus-visible:after:outline-2 focus-visible:after:outline-offset-[-2px] focus-visible:after:outline-accent"
        >
          <span className="flex items-start gap-2">
            {/* On a phone a long name wraps: beside the buttons, little of it would show. */}
            <span className="min-w-0 text-[14.5px] text-ink [overflow-wrap:anywhere] sm:truncate">
              {s.name}
            </span>
            {/* On the name's first line, where it wraps. */}
            <ChevronDown
              className={cn(
                "mt-1 size-3.5 shrink-0 text-ink-4 transition-transform duration-200",
                open && "rotate-180",
              )}
            />
          </span>
          <Facts project={project} schedule={s} />
          {(s.paused_reason || s.problem) && (
            <span className="mt-1 block text-[12.5px] leading-relaxed text-amber [overflow-wrap:anywhere]">
              {s.paused_reason ? `fctl paused it. ${s.paused_reason}` : s.problem}
            </span>
          )}
        </button>
        {/* Its person, Run now and its menu: one group, each centered on the others, and the
            group on the name's line. Over the row's button, which covers the row. */}
        <div className="relative -mt-[5px] flex shrink-0 items-center gap-1.5">
          {s.owner && (
            <span
              className="flex max-sm:hidden"
              title={
                s.cron || s.at !== undefined || s.event
                  ? `Runs on ${s.owner.name}'s Claude subscription`
                  : `Set up by ${s.owner.name}`
              }
            >
              <Avatar user={s.owner} size={20} />
            </span>
          )}
          {rights.run && s.enabled && (
            <Button
              size="sm"
              variant="ghost"
              icon={<Play className="size-3.5" />}
              loading={run.isPending}
              // One that waits for events runs on the issue, pull request or branch you name.
              onClick={() => (s.event ? setDialog("run") : run.mutate(undefined))}
              aria-label={`Run ${s.name} now, on your Claude subscription`}
              title="Run now, on your Claude subscription"
              className="max-sm:px-2 sm:min-w-24 [&>span]:max-sm:sr-only"
            >
              {/* One that waits for events asks what to run it on first. */}
              <span>{s.event ? "Run now…" : "Run now"}</span>
            </Button>
          )}
          {/* Paused, it starts nothing: what it needs is to be resumed. */}
          {!s.enabled && rights.edit && (
            <Button
              size="sm"
              variant="ghost"
              icon={<Play className="size-3.5" />}
              loading={update.isPending}
              onClick={resume}
              aria-label={mine ? `Resume ${s.name}` : `Resume ${s.name} on your Claude subscription`}
              title={mine ? "Resume it" : "Resume it on your Claude subscription"}
              className="max-sm:px-2 sm:min-w-24 [&>span]:max-sm:sr-only"
            >
              <span>Resume</span>
            </Button>
          )}
          {(rights.edit || rights.manage) && (
            <Menu
              trigger={(toggle, menuOpen) => (
                <IconButton
                  label="Schedule actions"
                  aria-haspopup="menu"
                  aria-expanded={menuOpen}
                  onClick={toggle}
                >
                  <Ellipsis className="size-4" />
                </IconButton>
              )}
            >
              {(close) => (
                <>
                  {rights.edit && (
                    <MenuItem
                      icon={<Pencil />}
                      onClick={() => {
                        close();
                        setEditing(true);
                        if (!open) onToggle();
                      }}
                    >
                      Edit
                    </MenuItem>
                  )}
                  {s.enabled && rights.manage && (
                    <MenuItem
                      icon={<Pause />}
                      onClick={() => {
                        close();
                        update.mutate({ enabled: false });
                      }}
                    >
                      Pause
                    </MenuItem>
                  )}
                  {!s.enabled && rights.edit && (
                    <MenuItem
                      icon={<Play />}
                      onClick={() => {
                        close();
                        resume();
                      }}
                    >
                      {mine ? "Resume" : "Resume on my subscription"}
                    </MenuItem>
                  )}
                  {!mine && s.enabled && rights.edit && (
                    <MenuItem
                      icon={<UserRoundCheck />}
                      onClick={() => {
                        close();
                        setDialog("take");
                      }}
                    >
                      Run it on my subscription
                    </MenuItem>
                  )}
                  {rights.manage && (
                    <>
                      <MenuSeparator />
                      <MenuItem
                        danger
                        icon={<Trash2 />}
                        onClick={() => {
                          close();
                          setDialog("delete");
                        }}
                      >
                        Delete…
                      </MenuItem>
                    </>
                  )}
                </>
              )}
            </Menu>
          )}
        </div>
      </div>
      {open && (
        <div id={panel} className="animate-rise px-3 pt-1 pb-6 sm:pl-[57px]">
          {editing ? (
            <Editor project={project} schedule={s} onDone={() => setEditing(false)} />
          ) : (
            <Details
              project={project}
              schedule={s}
              mine={mine}
              rights={rights}
              onEdit={() => setEditing(true)}
              onPause={() => update.mutate({ enabled: false })}
              onResume={resume}
              pending={update.isPending}
            />
          )}
        </div>
      )}
      <Dialog
        open={dialog === "resume" || dialog === "take"}
        onClose={() => setDialog(undefined)}
        title={dialog === "resume" ? "Resume it on your subscription?" : "Run it on your subscription?"}
        description={`It ran on ${s.owner?.name ?? "someone else"}'s Claude subscription. From now on its runs start on yours, and it's yours to change.`}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDialog(undefined)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            loading={update.isPending}
            onClick={() =>
              update.mutate(dialog === "resume" ? { enabled: true, run_as_me: true } : { run_as_me: true })
            }
          >
            {dialog === "resume" ? "Resume" : "Run it on mine"}
          </Button>
        </div>
      </Dialog>
      <DeleteSchedule
        project={project}
        schedule={s}
        open={dialog === "delete"}
        onClose={() => setDialog(undefined)}
      />
      {s.event && (
        <RunOnDialog
          project={project}
          schedule={s}
          event={s.event}
          open={dialog === "run"}
          onClose={() => setDialog(undefined)}
        />
      )}
    </li>
  );
}

/** Runs a schedule now, in a session on your subscription, with what you add for it, and opens it. */
function useRunNow(s: G.Schedule) {
  const navigate = useNavigate();
  return useMutation({
    mutationFn: (context?: string) => api.runSchedule(s.id, context ? { context } : {}),
    onSuccess: (r) => {
      void queryClient.invalidateQueries({ queryKey: keys.schedules(s.project_id) });
      void queryClient.invalidateQueries({ queryKey: keys.scheduleRuns(s.id) });
      if (r.session) return navigate({ to: "/s/$id", params: { id: r.session.id } });
      toast(r.reason ?? "It started no session", "error");
    },
    onError: (e) => toast(e.message, "error"),
  });
}

/** What a schedule asks, how its sessions start, where they run, and its runs. */
function Details({
  project,
  schedule: s,
  mine,
  rights,
  onEdit,
  onPause,
  onResume,
  pending,
}: {
  project: G.Project;
  schedule: G.Schedule;
  mine: boolean;
  rights: ReturnType<typeof scheduleRights>;
  onEdit: () => void;
  onPause: () => void;
  onResume: () => void;
  pending: boolean;
}) {
  const own = projectChoice(project);
  const coming = useMemo(() => {
    if (s.state !== "active" || !s.next_run_at) return [];
    if (!s.cron) return [s.next_run_at];
    const parsed = parseCron(s.cron);
    if ("error" in parsed) return [s.next_run_at];
    return [s.next_run_at, ...nextRuns(parsed.cron, s.timezone, s.next_run_at, 2)];
  }, [s]);
  return (
    <div className="flex flex-col gap-6">
      <div className="grid gap-x-8 gap-y-5 md:grid-cols-[minmax(0,1fr)_320px]">
        <div className="min-w-0">
          <h4 className="mb-1.5 text-[12.5px] font-medium text-ink-3">Task</h4>
          <Task text={s.prompt} />
        </div>
        <dl className="grid grid-cols-[88px_minmax(0,1fr)] content-start gap-x-3 gap-y-2 text-[12.5px] max-md:row-start-1">
          <Fact k="Runs">{scheduleWhen(s)}</Fact>
          {(s.cron || s.at !== undefined) && (
            <Fact k="Time zone">
              {zoneCity(s.timezone)}
              {zoneCity(s.timezone) !== zoneOffset(s.timezone) && (
                <span className="text-ink-4"> {zoneOffset(s.timezone)}</span>
              )}
            </Fact>
          )}
          {coming.length > 0 && (
            <Fact k={coming.length > 1 ? "Next runs" : "Next run"}>
              <span className="flex flex-col">
                {coming.map((t) => (
                  <time key={t} dateTime={new Date(t).toISOString()}>
                    {moment(t, s.timezone)}
                  </time>
                ))}
              </span>
            </Fact>
          )}
          <Fact k="Model">{s.model ? modelLabel(s.model) : `The project's (${modelLabel(own.model)})`}</Fact>
          <Fact k="Effort">
            {s.effort ? effortLabel(s.effort) : `The project's (${effortLabel(own.effort)})`}
          </Fact>
          {onGithub(project) && (
            <Fact k="On GitHub">
              {githubLevel(lowerAccess(s.github_access ?? own.github, own.github)).label}
            </Fact>
          )}
          {s.hide_sessions && <Fact k="Sessions">Kept off session lists: they're under its runs</Fact>}
          {s.base_branch && (
            <Fact k="From">
              <span className="font-mono">{s.base_branch}</span>
            </Fact>
          )}
          {!s.cron && s.at === undefined && !s.event ? (
            <Fact k="Runs on">The subscription of whoever runs it</Fact>
          ) : (
            s.owner && (
              <Fact k="Runs on">
                <span className="inline-flex items-center gap-1.5">
                  <Avatar user={s.owner} size={16} />
                  {mine ? "Your subscription" : `${s.owner.name}'s subscription`}
                </span>
              </Fact>
            )
          )}
        </dl>
      </div>
      <Runs schedule={s} />
      {rights.run && <ToolsDisclosure schedule={s} />}
      <div className="flex flex-wrap items-center gap-2 border-t border-line pt-4">
        {rights.edit && (
          <Button size="sm" icon={<Pencil className="size-3.5" />} onClick={onEdit}>
            Edit
          </Button>
        )}
        {s.enabled && rights.manage && (
          <Button
            size="sm"
            variant="ghost"
            icon={<Pause className="size-3.5" />}
            loading={pending}
            onClick={onPause}
          >
            Pause
          </Button>
        )}
        {!s.enabled && rights.edit && (
          <Button
            size="sm"
            variant="ghost"
            icon={<Play className="size-3.5" />}
            loading={pending}
            onClick={onResume}
          >
            {mine ? "Resume" : "Resume on my subscription"}
          </Button>
        )}
        <span className="flex-1" />
        <span className="text-[12px] text-ink-4">
          {s.updated_by
            ? `Changed by ${s.updated_by.name}, ${ago(s.updated_at)}`
            : `Set up ${ago(s.created_at)}`}
        </span>
      </div>
    </div>
  );
}

/**
 * Running it from another tool, a click away: open from the start for a schedule that runs when asked,
 * which is what it's for.
 */
function ToolsDisclosure({ schedule: s }: { schedule: G.Schedule }) {
  const timed = !!s.cron || s.at !== undefined || !!s.event;
  const [open, setOpen] = useState(!timed);
  const id = `tools-${s.id}`;
  return (
    <section>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen(!open)}
        className="flex items-center gap-1.5 text-[12.5px] font-medium text-ink-3 transition-colors hover:text-ink"
      >
        <Webhook className="size-3.5" aria-hidden="true" />
        Run it from another tool
        <ChevronDown className={cn("size-3.5 transition-transform duration-200", open && "rotate-180")} />
      </button>
      {open && (
        <div id={id} className="mt-2 animate-rise">
          <FromTools schedule={s} />
        </div>
      )}
    </section>
  );
}

/** How another tool runs a schedule: the request, ready to copy, and where the guide says more. */
function FromTools({ schedule: s }: { schedule: G.Schedule }) {
  const request = [
    `curl -X POST ${location.origin}/api/v1/schedules/${s.id}/runs \\`,
    '  -H "Authorization: Bearer $FCTL_TOKEN" -H "Content-Type: application/json" \\',
    `  -d '{"context": "What the tool knows of why it runs"}'`,
  ].join("\n");
  return (
    <div className="flex flex-col gap-2">
      <div className="relative rounded-lg border border-line bg-sunken/70">
        <pre className="overflow-x-auto py-2.5 pr-24 pl-3.5 font-mono text-[11.5px] leading-[1.7] text-ink-2">
          {request}
        </pre>
        <CopyButton text={request} label="Copy" className="absolute top-1.5 right-1.5 bg-sunken" />
      </div>
      <p className="text-[12.5px] leading-relaxed text-ink-4">
        With an access token that runs sessions in this project (Settings → Access tokens). Each request
        starts a session on the token's person's subscription, and its agent takes what <code>context</code>{" "}
        says as information, not instructions.{" "}
        <a
          href={`${siteUrl()}/docs/automation/#start-agents-from-other-tools`}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-0.5 text-ink-3 underline-offset-2 hover:text-ink hover:underline"
        >
          How <ArrowUpRight className="size-3" />
        </a>
      </p>
    </div>
  );
}

function Fact({ k, children }: { k: string; children: ReactNode }) {
  return (
    <>
      <dt className="text-ink-4">{k}</dt>
      <dd className="min-w-0 text-ink-2 [overflow-wrap:anywhere]">{children}</dd>
    </>
  );
}

/** A task as its agents get it; a long one folds after a few lines. */
function Task({ text }: { text: string }) {
  const [all, setAll] = useState(false);
  const long = text.length > 420 || text.split("\n").length > 7;
  return (
    <div className="rounded-lg border border-line bg-sunken/50 px-3.5 py-2.5">
      <p
        className={cn(
          "text-[13.5px] leading-[1.6] whitespace-pre-wrap text-ink-2 [overflow-wrap:anywhere]",
          long && !all && "line-clamp-6",
        )}
      >
        {text}
      </p>
      {long && (
        <button
          type="button"
          onClick={() => setAll(!all)}
          className="mt-1.5 text-[12.5px] text-ink-3 underline-offset-2 hover:text-ink hover:underline"
        >
          {all ? "Show less" : "Show all"}
        </button>
      )}
    </div>
  );
}

/** A schedule's runs, newest first: the sessions they started, and why the others started none. */
function Runs({ schedule: s }: { schedule: G.Schedule }) {
  const runs = useInfiniteQuery({
    queryKey: keys.scheduleRuns(s.id),
    queryFn: ({ pageParam }) => api.scheduleRuns(s.id, { before: pageParam, limit: 5 }),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next_before ?? undefined,
    refetchInterval: 30_000,
  });
  const items = runs.data?.pages.flatMap((p) => p.runs) ?? [];
  return (
    <section aria-label="Runs">
      <h4 className="mb-1.5 text-[12.5px] font-medium text-ink-3">Runs</h4>
      {runs.isPending ? (
        <Skeleton className="h-10" />
      ) : items.length === 0 ? (
        <p className="text-[13px] text-ink-4">
          {s.state === "active" && s.next_run_at
            ? `None yet. The first starts ${soon(s.next_run_at, s.timezone)}.`
            : s.state === "active" && s.event
              ? `None yet. The first starts ${describeEvent(s.event)}.`
              : "None yet."}
        </p>
      ) : (
        <ul className="divide-y divide-line rounded-lg border border-line">
          {items.map((r) => (
            <RunRow key={r.id} run={r} />
          ))}
        </ul>
      )}
      {runs.hasNextPage && (
        <Button
          size="sm"
          variant="ghost"
          className="mt-2"
          loading={runs.isFetchingNextPage}
          onClick={() => void runs.fetchNextPage()}
        >
          Older runs
        </Button>
      )}
    </section>
  );
}

function RunRow({ run: r }: { run: G.ScheduleRun }) {
  const when = (
    <time
      dateTime={new Date(r.at).toISOString()}
      title={stamp(r.at)}
      className="shrink-0 text-[12px] text-ink-4 tabular-nums"
    >
      {agoShort(r.at)}
    </time>
  );
  return (
    <li className="group relative flex items-start gap-2.5 px-3 py-2.5 text-[13px]">
      <RunIcon run={r} className="mt-[3px] size-3.5 shrink-0" />
      <div className="min-w-0 flex-1">
        {r.session ? (
          <Link
            to="/s/$id"
            params={{ id: r.session.id }}
            className="flex min-w-0 items-center gap-2 text-ink outline-none after:absolute after:inset-0 hover:text-ink focus-visible:after:outline-2 focus-visible:after:outline-accent"
          >
            <span className="shrink-0 font-mono text-[12px] text-ink-4">#{r.session.number}</span>
            <span className="truncate">{r.session.title}</span>
            {r.status && <StatusDot status={r.status} className="shrink-0" />}
          </Link>
        ) : (
          <span className={r.outcome === "failed" ? "text-danger" : "text-ink-2"}>{runOutcome(r)}</span>
        )}
        {r.event ? (
          <RunEvent event={r.event} />
        ) : (
          <span className="mt-0.5 block text-[12px] text-ink-4">{runTrigger(r)}</span>
        )}
        {r.reason && (
          <span className="mt-0.5 block text-[12.5px] leading-relaxed text-ink-3 [overflow-wrap:anywhere]">
            {r.reason}
          </span>
        )}
      </div>
      {when}
    </li>
  );
}

/**
 * What happened on GitHub that started a run, a link to it: "@octocat opened acme/web#41: The login
 * page hangs". Over the row's link to the session, which covers the row.
 */
function RunEvent({ event: e }: { event: G.ScheduleRunEvent }) {
  return (
    <a
      href={e.url}
      target="_blank"
      rel="noreferrer"
      title="Open it on GitHub"
      className="relative mt-0.5 flex max-w-full items-start gap-1.5 text-[12px] leading-[1.45] text-ink-4 transition-colors hover:text-ink-2"
    >
      <EventIcon kind={e.kind} className="mt-[2px] size-3 shrink-0" />
      {/* Two lines at most: on a phone, what it's about comes after who did what. */}
      <span className="line-clamp-2 min-w-0 [overflow-wrap:anywhere]">
        {sentence(eventSaid(e))}
        {e.title && <span className="text-ink-3">: {e.title}</span>}
      </span>
      <ArrowUpRight className="mt-[2px] size-3 shrink-0" />
    </a>
  );
}

/** What Run now asks of a schedule that waits for events: what to run it on, by kind. */
const RUN_ON: Partial<Record<G.ScheduleEventKind, { label: string; placeholder: string; problem: string }>> =
  {
    issue_opened: {
      label: "On the issue",
      placeholder: "#41, or its link",
      problem: "Write the issue's number, like #41, or paste its link",
    },
    issue_labeled: {
      label: "On the issue",
      placeholder: "#41, or its link",
      problem: "Write the issue's number, like #41, or paste its link",
    },
    pull_request_opened: {
      label: "On the pull request",
      placeholder: "#128, or its link",
      problem: "Write the pull request's number, like #128, or paste its link",
    },
    pull_request_merged: {
      label: "On the pull request",
      placeholder: "#128, or its link",
      problem: "Write the pull request's number, like #128, or paste its link",
    },
    release_published: {
      label: "On the release",
      placeholder: "v1.2.0, or its link",
      problem: "Write the release's tag, like v1.2.0, or paste its link",
    },
    checks_failed: {
      label: "On the branch",
      placeholder: "main",
      problem: "Write a branch's name, like main",
    },
  };

/**
 * Run now, for a schedule that waits for events: on the issue, pull request, release or branch you name,
 * which its agent gets after the task, as it would have got what happened, or on nothing, with the task
 * alone. It runs on your subscription, as any Run now does.
 */
function RunOnDialog({
  project,
  schedule: s,
  event,
  open,
  onClose,
}: {
  project: G.Project;
  schedule: G.Schedule;
  event: G.ScheduleEvent;
  open: boolean;
  onClose: () => void;
}) {
  const [on, setOn] = useState("");
  const [tried, setTried] = useState(false);
  const run = useRunNow(s);
  const ask = RUN_ON[event.kind];
  // A bare number is in the first repository it waits on, else the project's own.
  const repo = event.repos?.[0] ?? (project.repo ? `${project.repo.owner}/${project.repo.name}` : "");
  const read = on.trim() ? readRunOn(event.kind, on, repo) : undefined;
  const problem = on.trim() && !read ? ask?.problem : undefined;
  const go = () => {
    setTried(true);
    if (problem) return;
    run.mutate(read ? runOnContext(event.kind, read, event.labels?.[0]) : undefined, { onSuccess: onClose });
  };
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Run ${s.name} now?`}
      description="It starts a session on your Claude subscription. Its agent gets the task with what you name here, as it would get what happened on GitHub."
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          go();
        }}
      >
        {ask && (
          <Field
            label={ask.label}
            hint={
              event.kind === "checks_failed"
                ? "Its agent looks into the checks of the branch's newest commit."
                : "Empty: the task alone, on nothing in particular."
            }
            error={tried ? problem : undefined}
          >
            {(id) => (
              <Input
                id={id}
                value={on}
                onChange={(e) => setOn(e.target.value)}
                placeholder={event.kind === "checks_failed" ? project.default_branch : ask.placeholder}
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
                // The dialog asks for this one thing.
                autoFocus
              />
            )}
          </Field>
        )}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            icon={<Play className="size-3.5" />}
            loading={run.isPending}
          >
            Run now
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/** Asks before deleting: the sessions it started stay. */
function DeleteSchedule({
  project,
  schedule: s,
  open,
  onClose,
}: {
  project: G.Project;
  schedule: G.Schedule;
  open: boolean;
  onClose: () => void;
}) {
  const remove = useMutation({
    mutationFn: () => api.deleteSchedule(s.id),
    onSuccess: () => {
      saved(project, s, true);
      // The sessions it kept off the lists come back to them, and to the projects' counts.
      if (s.hide_sessions)
        for (const queryKey of [["sessions"], keys.projects])
          void queryClient.invalidateQueries({ queryKey });
      toast(`Deleted ${s.name}`);
      onClose();
    },
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Delete ${s.name}?`}
      description={
        s.hide_sessions
          ? "It starts no more sessions. The sessions it started stay, with their transcripts, and show in session lists again."
          : "It starts no more sessions. The sessions it started stay, with their transcripts."
      }
    >
      <div className="flex flex-col gap-5">
        {remove.error && <Banner tone="error">{remove.error.message}</Banner>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="danger" loading={remove.isPending} onClick={() => remove.mutate()}>
            Delete
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

// ───────────────────────────── the form ─────────────────────────────

type Mode = "repeat" | "once" | "cron" | "event" | "asked";

/** The ways a schedule runs, as the form offers them: a segmented control, and a list on a phone. */
const MODES: SelectOption<Mode>[] = [
  { value: "repeat", label: "Repeats", description: "Every hour, day, weekday, week or month" },
  { value: "once", label: "Once", description: "On a day, at a time" },
  { value: "cron", label: "Cron", description: "At a cron expression's times" },
  { value: "event", label: "On an event", description: "Each time something happens on GitHub" },
  { value: "asked", label: "When asked", description: "When someone, or another tool, runs it" },
];

const EVERY: SelectOption<Every>[] = [
  { value: "hour", label: "hour" },
  { value: "day", label: "day" },
  { value: "weekday", label: "weekday", description: "Monday to Friday" },
  { value: "week", label: "week" },
  { value: "month", label: "month" },
];

/** Monday first. */
const WEEK = [1, 2, 3, 4, 5, 6, 0];

const pad = (n: number) => String(n).padStart(2, "0");
const timeOf = (r: RepeatChoice) => `${pad(r.hour)}:${pad(r.minute)}`;

/** The day `days` after `date` ("2026-10-30"). */
function addDays(date: string, days: number): string {
  const t = Date.parse(`${date}T00:00:00Z`) + days * 86_400_000;
  return new Date(t).toISOString().slice(0, 10);
}

/** What the time zone list offers: yours first, then UTC, then every zone by its region. */
function zoneOptions(here: string, chosen: string): SelectOption[] {
  const now = Date.now();
  const all = zones();
  if (!all.includes(chosen)) all.push(chosen);
  const option = (z: string, section: { key: string; label: string }, note?: string) => ({
    value: z,
    label: `${zoneCity(z)} · ${zoneOffset(z, now)}`,
    item: zoneCity(z),
    description: note ? `${zoneOffset(z, now)} · ${note}` : zoneOffset(z, now),
    section,
  });
  return [
    option(here, { key: "yours", label: "Your time zone" }, area(here)),
    ...all
      .filter((x) => x !== here)
      .map((x) =>
        option(
          x,
          { key: zoneRegion(x) || "utc", label: zoneRegion(x) || "UTC" },
          zoneRegion(x) ? area(x) : undefined,
        ),
      ),
  ];
}

/** "America/Argentina": where a zone is, which its city's name may not say. */
const area = (zone: string) => zone.split("/").slice(0, -1).join("/").replaceAll("_", " ");

/**
 * Sets up a schedule, or changes one: its name, its task with the model, effort level and GitHub
 * access its sessions start with, and when it runs, with its next runs as you choose.
 */
function Editor({
  project,
  schedule,
  draft,
  onDone,
}: {
  project: G.Project;
  schedule?: G.Schedule;
  draft?: Draft;
  onDone: (s?: G.Schedule) => void;
}) {
  const { data: me } = useMe();
  const here = useMemo(browserZone, []);
  const own = projectChoice(project);
  const mine = !schedule || (!!me && schedule.owner?.id === me.user.id);
  const [name, setName] = useState(schedule?.name ?? draft?.name ?? "");
  const [prompt, setPrompt] = useState(schedule?.prompt ?? draft?.prompt ?? "");
  const [choice, setChoice] = useState<Choice>(() => ({
    model: schedule?.model ?? draft?.model ?? own.model,
    effort: schedule?.effort ?? draft?.effort ?? own.effort,
    github: lowerAccess(schedule?.github_access ?? draft?.github ?? own.github, own.github),
  }));
  const [hidden, setHidden] = useState(schedule?.hide_sessions ?? draft?.hide ?? false);
  const [zone, setZone] = useState(schedule?.timezone ?? here);
  const [pickZone, setPickZone] = useState(false);
  const initialRepeat = (schedule?.cron && cronRepeat(schedule.cron)) || draft?.repeat || DEFAULT_REPEAT;
  const [mode, setMode] = useState<Mode>(() => {
    if (!schedule) return draft?.event ? "event" : "repeat";
    if (schedule.event) return "event";
    if (schedule.cron) return cronRepeat(schedule.cron) ? "repeat" : "cron";
    return schedule.at !== undefined ? "once" : "asked";
  });
  const [event, setEvent] = useState<EventForm>(() => eventForm(schedule?.event ?? draft?.event));
  const [repeat, setRepeat] = useState<RepeatChoice>(initialRepeat);
  const [cron, setCron] = useState(schedule?.cron ?? repeatCron(initialRepeat));
  const [once, setOnce] = useState(() => {
    if (schedule?.at !== undefined && schedule.state !== "done") return wallParts(zone, schedule.at);
    const tomorrow = wallParts(zone, Date.now() + 86_400_000);
    return { date: tomorrow.date, time: "09:00" };
  });
  const textarea = useRef<HTMLTextAreaElement>(null);
  const form = useRef<HTMLFormElement>(null);
  useEffect(() => {
    // A new one's form comes into view, ready for its task.
    if (schedule) return;
    form.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    if (!draft?.prompt) textarea.current?.focus({ preventScroll: true });
  }, [schedule, draft]);

  const expression = mode === "repeat" ? repeatCron(repeat) : cron;
  const atTime = mode === "once" ? fromWall(zone, once.date, once.time) : Number.NaN;
  const whenProblem =
    mode === "asked"
      ? undefined
      : mode === "event"
        ? eventProblem(event)
        : mode === "once"
          ? Number.isNaN(atTime)
            ? "Choose a day and a time"
            : atTime < Date.now() - 60_000
              ? "That time has passed: choose one ahead"
              : undefined
          : cronProblem(expression);
  const upcoming = useMemo(() => {
    if (whenProblem || mode === "asked" || mode === "event") return [];
    if (mode === "once") return [atTime];
    const parsed = parseCron(expression);
    return "error" in parsed ? [] : nextRuns(parsed.cron, zone, Date.now(), 3);
  }, [mode, atTime, expression, zone, whenProblem]);

  const save = useMutation({
    mutationFn: async () => {
      const asks = asked(project, choice);
      // Without a time, it runs when asked: a new one gets neither, a change clears the cron. A time
      // ends what it waited for on GitHub, and an event its time.
      const timing =
        mode === "once"
          ? { at: atTime }
          : mode === "event"
            ? { event: toEvent(event) }
            : mode === "asked"
              ? schedule
                ? { cron: "" }
                : {}
              : { cron: expression };
      if (!schedule) {
        const body: G.CreateScheduleRequest = {
          name: name.trim() || undefined,
          prompt,
          timezone: zone,
          ...timing,
          ...asks,
          hide_sessions: hidden || undefined,
        };
        return api.createSchedule(project.id, body);
      }
      const body: G.UpdateScheduleRequest = {
        name: name.trim(),
        prompt,
        timezone: zone,
        ...timing,
        model: asks.model ?? "",
        effort: asks.effort ?? "",
        github_access: asks.github_access ?? "",
        hide_sessions: hidden,
      };
      return api.updateSchedule(schedule.id, body);
    },
    onSuccess: (s) => {
      saved(project, s);
      // Its sessions leave the session lists and the projects' counts, or come back to them.
      if (schedule && schedule.hide_sessions !== s.hide_sessions)
        for (const queryKey of [["sessions"], keys.projects])
          void queryClient.invalidateQueries({ queryKey });
      if (!schedule) draft?.onSaved?.();
      toast(
        schedule
          ? "Saved: its next run goes by it"
          : s.next_run_at
            ? `Set up: it runs ${soon(s.next_run_at, s.timezone)}`
            : s.event
              ? `Set up: it runs ${describeEvent(s.event)}`
              : "Set up: it runs when asked",
      );
      onDone(s);
    },
  });
  const failure = save.error instanceof ApiFailure ? save.error : undefined;
  const fieldError = (...fields: string[]) =>
    failure?.field && fields.includes(failure.field) ? failure.message : undefined;
  const placeholderName = nameFromTask(prompt) || "Name it";
  const zoneList = useMemo(() => (pickZone ? zoneOptions(here, zone) : []), [pickZone, here, zone]);
  const ready = !!prompt.trim() && !whenProblem;
  const takesOver = !!schedule && !mine;
  /** Moves to another way of running, with what carries over: a repeat's times, as cron and back. */
  const pickMode = (m: Mode) => {
    if (m === "cron" && mode === "repeat") setCron(repeatCron(repeat));
    if (m === "repeat" && mode === "cron") {
      const r = cronRepeat(cron);
      if (r) setRepeat(r);
    }
    setMode(m);
  };

  return (
    <form
      ref={form}
      onSubmit={(e) => {
        e.preventDefault();
        if (ready) save.mutate();
      }}
      className={cn(
        "flex animate-rise scroll-mt-24 flex-col gap-6 rounded-2xl border border-line bg-surface/70 p-4 sm:p-5",
        !schedule && "shadow-[0_24px_60px_-40px_rgb(0_0_0/0.5)]",
      )}
      aria-label={schedule ? `Change ${schedule.name}` : "New schedule"}
    >
      {!schedule && (
        <div className="-mx-4 flex items-center gap-2 overflow-x-auto px-4 [scrollbar-width:none] sm:-mx-5 sm:px-5">
          <span className="shrink-0 text-[12.5px] text-ink-4">Start from</span>
          {ideasFor(project).map((i) => (
            <button
              key={i.key}
              type="button"
              onClick={() => {
                setName(i.name);
                setPrompt(i.prompt);
                if (i.event) {
                  setMode("event");
                  setEvent(eventForm(i.event));
                } else {
                  const r = i.repeat ?? DEFAULT_REPEAT;
                  setMode("repeat");
                  setRepeat(r);
                  setCron(repeatCron(r));
                }
                setHidden(!!i.hide);
                setChoice((c) => ({ ...c, github: lowerAccess(i.github ?? own.github, own.github) }));
              }}
              className={cn(
                "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-[12.5px] transition-colors duration-150 [&_svg]:size-3.5",
                prompt === i.prompt
                  ? "border-line-strong bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                  : "border-line text-ink-3 hover:bg-hover hover:text-ink",
              )}
            >
              {i.icon}
              {i.name}
            </button>
          ))}
        </div>
      )}
      <div className="flex flex-col gap-3">
        <div>
          <input
            value={name}
            onChange={(e) => setName(e.target.value)}
            placeholder={placeholderName}
            aria-label="Name"
            maxLength={80}
            className="w-full border-b border-transparent bg-transparent pb-1 font-display text-[20px] leading-snug font-medium tracking-[-0.01em] text-ink outline-none transition-colors placeholder:text-ink-4 hover:border-line focus:border-line-strong"
          />
          {fieldError("name") && <p className="mt-1 text-[12.5px] text-danger">{fieldError("name")}</p>}
        </div>
        <div
          className={cn(
            "relative rounded-2xl border border-line-strong bg-surface/80 transition-[border-color,box-shadow] focus-within:border-accent/45 focus-within:shadow-[0_0_0_4px_var(--c-accent-soft)]",
            fieldError("prompt") && "border-danger/60",
          )}
        >
          <Textarea
            ref={textarea}
            bare
            minRows={3}
            maxRows={14}
            value={prompt}
            onChange={(e) => setPrompt(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                e.preventDefault();
                if (ready) save.mutate();
              }
            }}
            aria-label="Task"
            placeholder="What should an agent do each time?"
            className="block px-4 pt-3.5 font-display text-[16px] leading-[1.55] placeholder:text-ink-4"
          />
          <TextareaControls textarea={textarea} className="flex-wrap justify-end gap-3 pt-1 pr-3 pb-3 pl-3">
            <StartChoices
              project={project}
              choice={choice}
              onPick={(c) => setChoice((x) => ({ ...x, ...c }))}
            />
          </TextareaControls>
        </div>
        {fieldError("prompt", "model", "effort", "github_access") && (
          <p className="text-[12.5px] text-danger">
            {fieldError("prompt", "model", "effort", "github_access")}
          </p>
        )}
      </div>

      <fieldset className="flex flex-col gap-4">
        <legend className="sr-only">When it runs</legend>
        <div className="flex flex-wrap items-center gap-x-4 gap-y-2">
          <span className="text-[12.5px] font-medium text-ink-2" aria-hidden="true">
            When
          </span>
          {/* Five don't fit a phone's row: there they are a list, each with what it means. */}
          <div className="w-full sm:hidden">
            <Select aria-label="When it runs" value={mode} onChange={pickMode} options={MODES} />
          </div>
          <div className="inline-flex rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)] max-sm:hidden">
            {MODES.map((m) => (
              <button
                key={m.value}
                type="button"
                aria-pressed={mode === m.value}
                title={m.description as string}
                onClick={() => pickMode(m.value)}
                className={cn(
                  "h-7 rounded-full px-3.5 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
                  mode === m.value
                    ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25),inset_0_0_0_1px_var(--c-line-strong)]"
                    : "text-ink-3 hover:text-ink",
                )}
              >
                {m.label}
              </button>
            ))}
          </div>
        </div>

        {mode === "repeat" && <RepeatFields repeat={repeat} onChange={setRepeat} />}
        {mode === "once" && <OnceFields zone={zone} value={once} onChange={setOnce} />}
        {mode === "cron" && <CronField value={cron} onChange={setCron} />}
        {mode === "event" && (
          <EventFields
            project={project}
            value={event}
            onChange={setEvent}
            problem={whenProblem}
            failure={failure?.field?.startsWith("event") ? failure : undefined}
          />
        )}
        {mode === "asked" && (
          <div className="flex flex-col gap-3">
            <p className="max-w-xl text-[13.5px] leading-relaxed text-ink-3">
              It has no time of its own: it runs when someone runs it, here or with{" "}
              <code className="font-mono text-[12.5px]">fctl schedules run</code>, or when another tool does
              with an access token, such as an issue tracker sending what it knows of the case.
            </p>
            {schedule ? (
              <FromTools schedule={schedule} />
            ) : (
              <p className="text-[12.5px] text-ink-4">Once it's set up, it shows how other tools run it.</p>
            )}
          </div>
        )}

        <div
          className={cn(
            "flex flex-wrap items-center gap-x-2 gap-y-2 text-[12.5px] text-ink-3",
            // Its times are only in the note its runs' agents get.
            (mode === "asked" || mode === "event") && "hidden",
          )}
        >
          {pickZone ? (
            <div className="w-full max-w-sm">
              {/* A time to run once stays the same on the clock, in the new zone. */}
              <Select
                aria-label="Time zone"
                value={zone}
                onChange={(v) => {
                  setZone(v);
                  setPickZone(false);
                }}
                options={zoneList}
                className="w-full"
              />
            </div>
          ) : (
            <>
              <span>
                In {zone === here ? "your time zone, " : ""}
                <span className="text-ink-2">{zoneCity(zone)}</span> ({zoneOffset(zone)})
              </span>
              <button
                type="button"
                onClick={() => setPickZone(true)}
                className="text-ink-3 underline decoration-line-strong underline-offset-2 hover:text-ink hover:decoration-ink-3"
              >
                Change
              </button>
            </>
          )}
          {fieldError("timezone") && <span className="w-full text-danger">{fieldError("timezone")}</span>}
        </div>

        {mode !== "event" && (
          <Upcoming
            times={upcoming}
            zone={zone}
            here={here}
            problem={fieldError("cron", "at") ?? whenProblem}
            once={mode === "once"}
          />
        )}
      </fieldset>

      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <p className="text-[12.5px] font-medium text-ink-2">Keep its sessions off session lists</p>
          <p className="mt-1 max-w-xl text-[12.5px] leading-relaxed text-ink-3">
            {hidden
              ? "Its sessions stay out of Working now, the sidebar and the project's sessions. They're listed under its runs, and search finds them."
              : "Its sessions show in session lists like any other. Turn it on for a schedule that runs often, such as an hourly triage or one for each new issue."}
          </p>
        </div>
        <Switch checked={hidden} onChange={setHidden} label="Keep its sessions off session lists" />
      </div>

      {save.error && !failure?.field && <Banner tone="error">{save.error.message}</Banner>}
      {failure?.field === "event" && <Banner tone="error">{failure.message}</Banner>}
      {failure?.field === "claude" && (
        <Banner tone="error">
          {failure.message}{" "}
          <Link to="/settings" search={{ tab: "claude" }} className="underline underline-offset-2">
            Connect Claude
          </Link>
        </Banner>
      )}

      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-3 border-t border-line pt-4">
        <p className="flex min-w-0 items-center gap-2 text-[12.5px] leading-snug text-ink-3">
          {me && <Avatar user={me.user} size={18} />}
          {mode === "asked"
            ? "Each run starts on the Claude subscription of whoever runs it."
            : takesOver
              ? `Saving runs it on your Claude subscription, instead of ${schedule?.owner?.name ?? "its owner"}'s.`
              : "Its runs start on your Claude subscription."}
        </p>
        <div className="flex gap-2 max-sm:ml-auto">
          <Button variant="ghost" onClick={() => onDone()}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={save.isPending} disabled={!ready}>
            {schedule ? "Save" : "Set up schedule"}
          </Button>
        </div>
      </div>
    </form>
  );
}

/** "Every [week] on [Mon]… at [09:00]": the form's sentence for a schedule that repeats. */
function RepeatFields({ repeat, onChange }: { repeat: RepeatChoice; onChange: (r: RepeatChoice) => void }) {
  const set = (r: Partial<RepeatChoice>) => onChange({ ...repeat, ...r });
  const minutes: SelectOption[] = Array.from({ length: 12 }, (_, i) => i * 5)
    .concat(repeat.minute % 5 ? [repeat.minute] : [])
    .sort((a, b) => a - b)
    .map((m) => ({ value: String(m), label: `:${pad(m)}` }));
  const days: SelectOption[] = Array.from({ length: 31 }, (_, i) => i + 1).map((d) => ({
    value: String(d),
    label: `the ${ordinal(d)}`,
    description: d > 28 ? "Months without it are skipped" : undefined,
  }));
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 text-[14px] text-ink-2">
        <span>Every</span>
        <Select
          aria-label="Every"
          value={repeat.every}
          onChange={(every) => set({ every })}
          options={EVERY}
          className="w-[8.5rem]"
        />
        {/* Each word stays with its field where the sentence wraps. */}
        {repeat.every === "month" && (
          <span className="inline-flex items-center gap-2">
            on
            <Select
              aria-label="Day of the month"
              value={String(repeat.day)}
              onChange={(d) => set({ day: Number(d) })}
              options={days}
              className="w-[7.5rem]"
            />
          </span>
        )}
        {repeat.every === "hour" ? (
          <span className="inline-flex items-center gap-2">
            at
            <Select
              aria-label="Minute"
              value={String(repeat.minute)}
              onChange={(m) => set({ minute: Number(m) })}
              options={minutes}
              className="w-[5.5rem]"
            />
            past the hour
          </span>
        ) : (
          <span className="inline-flex items-center gap-2">
            at
            <span className="w-[8.5rem]">
              <Input
                type="time"
                aria-label="Time"
                required
                value={timeOf(repeat)}
                onChange={(e) => {
                  const [h, m] = e.target.value.split(":").map(Number);
                  if (h !== undefined && m !== undefined && !Number.isNaN(h) && !Number.isNaN(m))
                    set({ hour: h, minute: m });
                }}
                className="tabular-nums"
              />
            </span>
          </span>
        )}
      </div>
      {repeat.every === "week" && (
        <fieldset className="grid max-w-md grid-cols-7 gap-1.5">
          <legend className="sr-only">On these days</legend>
          {WEEK.map((d) => {
            const on = repeat.weekdays.includes(d);
            const name = WEEKDAY_NAMES[d] ?? "";
            return (
              <button
                key={d}
                type="button"
                aria-pressed={on}
                aria-label={name}
                onClick={() => {
                  const next = on ? repeat.weekdays.filter((x) => x !== d) : [...repeat.weekdays, d];
                  // At least one day: the last can't go.
                  if (next.length) set({ weekdays: next });
                }}
                className={cn(
                  "h-9 rounded-md border text-[12.5px] font-medium transition-[background-color,border-color,color,transform] duration-150 active:scale-95",
                  on
                    ? "border-highlight bg-highlight text-highlight-ink"
                    : "border-line-strong text-ink-3 hover:border-ink-4 hover:text-ink",
                )}
              >
                {name.slice(0, 3)}
              </button>
            );
          })}
        </fieldset>
      )}
    </div>
  );
}

/** A day and a time to run once, with the times people pick most. */
function OnceFields({
  zone,
  value,
  onChange,
}: {
  zone: string;
  value: { date: string; time: string };
  onChange: (v: { date: string; time: string }) => void;
}) {
  const now = Date.now();
  const today = wallParts(zone, now);
  const inHour = wallParts(zone, Math.ceil((now + 3_600_000) / 300_000) * 300_000);
  const weekday = new Date(`${today.date}T12:00:00Z`).getUTCDay();
  const monday = addDays(today.date, (8 - weekday) % 7 || 7);
  const picks = [
    { label: "In an hour", ...inHour },
    today.time < "17:30" && { label: "This evening", date: today.date, time: "18:00" },
    { label: "Tomorrow morning", date: addDays(today.date, 1), time: "09:00" },
    { label: "Monday morning", date: monday, time: "09:00" },
  ].filter((p): p is { label: string; date: string; time: string } => !!p);
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-center gap-2 text-[14px] text-ink-2">
        <span>On</span>
        <span className="w-[10.5rem]">
          <Input
            type="date"
            aria-label="Day"
            required
            min={today.date}
            value={value.date}
            onChange={(e) => onChange({ ...value, date: e.target.value })}
            className="tabular-nums"
          />
        </span>
        <span>at</span>
        <span className="w-[8.5rem]">
          <Input
            type="time"
            aria-label="Time"
            required
            value={value.time}
            onChange={(e) => onChange({ ...value, time: e.target.value })}
            className="tabular-nums"
          />
        </span>
      </div>
      <div className="flex flex-wrap gap-1.5">
        {picks.map((p) => {
          const on = p.date === value.date && p.time === value.time;
          return (
            <button
              key={p.label}
              type="button"
              aria-pressed={on}
              onClick={() => onChange({ date: p.date, time: p.time })}
              className={cn(
                "h-7 rounded-full border px-2.5 text-[12.5px] transition-colors duration-150",
                on
                  ? "border-line-strong bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                  : "border-line text-ink-3 hover:bg-hover hover:text-ink",
              )}
            >
              {p.label}
            </button>
          );
        })}
      </div>
    </div>
  );
}

/** A cron expression, and what it says in words as you type. */
function CronField({ value, onChange }: { value: string; onChange: (v: string) => void }) {
  const parsed = parseCron(value);
  return (
    <div className="flex flex-col gap-1.5">
      <Input
        aria-label="Cron expression"
        value={value}
        onChange={(e) => onChange(e.target.value)}
        placeholder="0 9 * * 1-5"
        className="max-w-sm font-mono"
        autoComplete="off"
        autoCapitalize="off"
        autoCorrect="off"
        spellCheck={false}
      />
      <p className="text-[12.5px] text-ink-3">
        {"error" in parsed ? (
          "The minute, the hour, the day of the month, the month and the day of the week, like 0 9 * * 1-5 for weekdays at 09:00."
        ) : (
          <>
            Runs <span className="text-ink-2">{describeCron(parsed.cron, clockOf)}</span>.
          </>
        )}
      </p>
    </div>
  );
}

/** When it runs next, as you choose: in its zone, and in yours when that is another. */
function Upcoming({
  times,
  zone,
  here,
  problem,
  once,
}: {
  times: number[];
  zone: string;
  here: string;
  problem?: string;
  once: boolean;
}) {
  if (problem)
    return (
      <p className="animate-rise text-[12.5px] text-danger" role="alert">
        {problem}
      </p>
    );
  if (times.length === 0) return null;
  return (
    <div className="flex flex-col gap-2 rounded-xl bg-sunken/60 px-3.5 py-3 shadow-[inset_0_0_0_1px_var(--c-line)]">
      <span className="text-[12px] text-ink-4">{once ? "It runs" : "Its next runs"}</span>
      <ol className="flex flex-col gap-1.5 sm:flex-row sm:flex-wrap sm:gap-x-6">
        {times.map((t, i) => (
          <li key={t} className="flex items-baseline gap-2 text-[13px] text-ink-2 tabular-nums">
            <span
              className={cn(
                "size-1.5 shrink-0 self-center rounded-full",
                i === 0 ? "bg-highlight" : "bg-ink-4",
              )}
            />
            <time dateTime={new Date(t).toISOString()}>{moment(t, zone)}</time>
            {zone !== here && <span className="text-[12px] text-ink-4">{moment(t, here)} your time</span>}
          </li>
        ))}
      </ol>
    </div>
  );
}

// ───────────────────────────── events ─────────────────────────────

/** An event's kind, as GitHub draws what it happens to: an issue, a label, a pull request, a release. */
function EventIcon({ kind, className }: { kind: G.ScheduleEventKind; className?: string }) {
  switch (kind) {
    case "issue_opened":
      return <CircleDot className={className} />;
    case "issue_labeled":
      return <Tag className={className} />;
    case "pull_request_opened":
      return <PullRequestIcon state="open" className={className} />;
    case "pull_request_merged":
      return <PullRequestIcon state="merged" className={className} />;
    case "release_published":
      return <Rocket className={className} />;
    case "checks_failed":
      return <Siren className={className} />;
    default:
      return <Zap className={className} />;
  }
}

/** What each kind is for, under its name. */
const EVENT_LINES: Record<G.ScheduleEventKind, string> = {
  issue_opened: "Triage each issue as it comes in",
  issue_labeled: "Hand an issue to an agent with a label",
  pull_request_opened: "Each new one, and a draft once it's ready",
  pull_request_merged: "Learn from each merge and its review",
  release_published: "Try or announce each release",
  checks_failed: "Find and fix what broke, on main by default",
  other: "",
};

/** Whose issues or pull requests start runs, as the form offers it. */
const BY: SelectOption<G.ScheduleEventBy>[] = [
  { value: "anyone", label: "Anyone", description: "People outside the project and apps too" },
  {
    value: "writers",
    label: "People who can write to it",
    description: "Those GitHub says may write to the repository, and fctl's agents",
  },
  { value: "agents", label: "fctl's agents", description: "Only what the project's agents opened" },
];

/** What the form keeps of an event as you choose: its labels and branches as typed. */
interface EventForm {
  kind: G.ScheduleEventKind;
  /** The repositories it waits on: none for those the project's agents write to. */
  repos: string[];
  labels: string;
  branches: string;
  /** Absent: the kind's own default (`eventBy`). */
  by?: G.ScheduleEventBy;
}

function eventForm(e?: G.ScheduleEvent): EventForm {
  return {
    kind: e?.kind ?? "issue_opened",
    repos: e?.repos ?? [],
    labels: (e?.labels ?? []).join(", "),
    branches: (e?.branches ?? []).join(", "),
    by: e?.by,
  };
}

/** "bug, crash" as ["bug", "crash"]. */
const items = (text: string) =>
  text
    .split(",")
    .map((t) => t.trim())
    .filter(Boolean);

/** What the API takes: only what the kind has. */
function toEvent(f: EventForm): G.ScheduleEvent {
  const labels = takesLabels(f.kind) ? items(f.labels) : [];
  const branches = takesBranches(f.kind) ? items(f.branches) : [];
  return {
    kind: f.kind,
    repos: f.repos.length ? f.repos : undefined,
    labels: labels.length ? labels : undefined,
    branches: branches.length ? branches : undefined,
    by: takesBy(f.kind) && f.by ? f.by : undefined,
  };
}

/** What keeps the form from saving an event: an issue's label that names none. */
const eventProblem = (f: EventForm) =>
  f.kind === "issue_labeled" && items(f.labels).length === 0 ? "Name the label that starts a run" : undefined;

const fullName = (r: Pick<G.ProjectRepo, "owner" | "name">) => `${r.owner}/${r.name ?? "*"}`;
/** The repository list's choice of those the project's agents write to: no repository's name. */
const ALL_REPOS = "*";
const writes = (r: G.ProjectRepo) => r.primary || r.access === "write";

/**
 * What a schedule waits for on GitHub: the kinds to choose from, each with what it is for, what narrows
 * the chosen one down (its repository, label, branch and people), and what each run gets, with why runs
 * wouldn't start here when they wouldn't (the GitHub App isn't subscribed to the event, say).
 */
function EventFields({
  project,
  value,
  onChange,
  problem,
  failure,
}: {
  project: G.Project;
  value: EventForm;
  onChange: (v: EventForm) => void;
  /** What keeps it from saving (`eventProblem`), said under the field it concerns. */
  problem?: string;
  /** The control plane's word on a field of the event. */
  failure?: ApiFailure;
}) {
  // The list the tab shows says which kinds fctl wouldn't hear of here.
  const list = useQuery({ queryKey: keys.schedules(project.id), queryFn: () => api.schedules(project.id) });
  const unheard = list.data?.unheard ?? [];
  // Owner-wide grants name no repository to wait on.
  const repos = (useProjectRepos(project.id).data?.repos ?? []).filter((r) => r.name);
  const { kind } = value;
  const set = (v: Partial<EventForm>) => onChange({ ...value, ...v });
  const error = (field: string) => (failure?.field === field ? failure.message : undefined);
  const by = eventBy({ kind, by: value.by });
  // An empty value is a Select's placeholder: those its agents write to are a choice of their own.
  const repoOptions: SelectOption[] = [
    {
      value: ALL_REPOS,
      label: "Its repositories",
      description: `Those its agents write to${repos.some(writes) ? `: ${orList(repos.filter(writes).map(fullName))}` : ""}`,
    },
    ...repos.map((r) => ({
      value: fullName(r),
      label: fullName(r),
      description: writes(r) ? undefined : "Read only: its agents can't write to it",
    })),
    ...(value.repos.length > 1 ? [{ value: value.repos.join(","), label: orList(value.repos) }] : []),
  ];
  const where = value.repos.length
    ? orList(value.repos)
    : repos.some(writes)
      ? orList(repos.filter(writes).map(fullName))
      : "the repositories its agents write to";
  return (
    <div className="flex flex-col gap-4">
      <fieldset className="grid gap-2 sm:grid-cols-2">
        <legend className="sr-only">What it waits for</legend>
        {EVENT_KINDS.map((k) => {
          const on = k === kind;
          const away = unheard.find((u) => u.kind === k);
          return (
            <button
              key={k}
              type="button"
              aria-pressed={on}
              // Whose it takes starts at the kind's own default: anyone's pull requests opened is a risk
              // to choose, not to carry over.
              onClick={() => set({ kind: k, by: k === kind ? value.by : undefined })}
              className={cn(
                "flex items-start gap-3 rounded-xl border px-3 py-2.5 text-left transition-[background-color,border-color,box-shadow] duration-150",
                on
                  ? "border-line-strong bg-raised shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                  : "border-line hover:bg-hover",
              )}
            >
              <span
                aria-hidden="true"
                className={cn(
                  "mt-px flex size-7 shrink-0 items-center justify-center rounded-lg transition-colors duration-150 [&_svg]:size-3.5",
                  on ? "bg-highlight text-highlight-ink" : "bg-sunken/70 text-ink-3",
                )}
              >
                <EventIcon kind={k} />
              </span>
              <span className="min-w-0 flex-1">
                <span className="flex items-center gap-1.5 text-[13.5px] text-ink">
                  {EVENT_NAMES[k]}
                  {away && (
                    <TriangleAlert
                      className="size-3 shrink-0 text-amber"
                      aria-label="fctl doesn't hear of it here"
                    />
                  )}
                </span>
                <span className="mt-0.5 block text-[12px] leading-snug text-ink-3">{EVENT_LINES[k]}</span>
              </span>
            </button>
          );
        })}
      </fieldset>

      {(repos.length > 1 || takesLabels(kind) || takesBranches(kind) || takesBy(kind)) && (
        <div className="grid items-start gap-x-4 gap-y-3 sm:grid-cols-2">
          {repos.length > 1 && (
            <Field label="In" error={error("event.repos")}>
              {(id) => (
                <Select
                  id={id}
                  value={value.repos.length ? value.repos.join(",") : ALL_REPOS}
                  options={repoOptions}
                  onChange={(v) => set({ repos: v === ALL_REPOS ? [] : v.split(",") })}
                />
              )}
            </Field>
          )}
          {takesLabels(kind) && (
            <Field
              label={kind === "issue_labeled" ? "Label" : "With the label"}
              hint={
                kind === "issue_labeled"
                  ? "The label that starts a run. Several, comma-separated: any of them."
                  : "Empty: any. Several, comma-separated: any of them."
              }
              error={error("event.labels") ?? problem}
            >
              {(id) => (
                <Input
                  id={id}
                  value={value.labels}
                  onChange={(e) => set({ labels: e.target.value })}
                  placeholder={kind === "issue_labeled" ? "fctl" : "Any label"}
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                />
              )}
            </Field>
          )}
          {takesBranches(kind) && (
            <Field
              label={kind === "checks_failed" ? "On the branch" : "Into the branch"}
              hint={
                kind === "checks_failed"
                  ? "Empty: each repository's default branch."
                  : "Empty: any. Several, comma-separated: any of them."
              }
              error={error("event.branches")}
            >
              {(id) => (
                <Input
                  id={id}
                  value={value.branches}
                  onChange={(e) => set({ branches: e.target.value })}
                  placeholder={kind === "checks_failed" ? project.default_branch : "Any branch"}
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  className="font-mono"
                />
              )}
            </Field>
          )}
          {takesBy(kind) && (
            <Field
              label="Opened by"
              hint={
                kind === "pull_request_opened" && by === "anyone"
                  ? "An outsider's pull request starts a run too: write the task so that its agent doesn't run their code."
                  : undefined
              }
              error={error("event.by")}
            >
              {(id) => <Select id={id} value={by} options={BY} onChange={(b) => set({ by: b })} />}
            </Field>
          )}
        </div>
      )}

      <div className="flex flex-col gap-2 rounded-xl bg-sunken/60 px-3.5 py-3 shadow-[inset_0_0_0_1px_var(--c-line)]">
        <span className="text-[12px] text-ink-4">Each time</span>
        <p className="text-[13px] leading-relaxed text-ink-2">
          {eachRun(value, where)}
          {takesBy(kind) &&
            by === "writers" &&
            " Only those of people who can write to the repository, and of fctl's agents."}
          {takesBy(kind) && by === "agents" && " Only those fctl's agents opened."}
        </p>
        <p className="text-[12px] leading-relaxed text-ink-4">
          Whoever opens an issue or a pull request may be someone who can't instruct your agents, so they take
          what GitHub says as information, not instructions. At most 20 runs start in an hour.
        </p>
        {unheard.find((u) => u.kind === kind) && (
          <p className="flex gap-1.5 text-[12.5px] leading-relaxed text-amber" role="status">
            <TriangleAlert className="mt-[3px] size-3.5 shrink-0" aria-hidden="true" />
            {unheard.find((u) => u.kind === kind)?.reason}
          </p>
        )}
      </div>
    </div>
  );
}

/** What starts a run, and what its agent gets, for the form's event in `where`. */
function eachRun(f: EventForm, where: string): string {
  const labels = orList(items(f.labels));
  const branches = orList(items(f.branches));
  switch (f.kind) {
    case "issue_opened":
      return `An issue opened in ${where}${labels ? ` with the label ${labels}` : ""} starts a session of its own, whose agent gets the issue's title, text and labels after the task.`;
    case "issue_labeled":
      return `An issue in ${where} that gets the label ${labels || "you name"} starts a session of its own, whose agent gets the issue after the task. Taking the label off and putting it back on runs it again.`;
    case "pull_request_opened":
      return `A pull request opened in ${where}${branches ? ` into ${branches}` : ""}, or a draft marked ready for review, starts a session of its own, whose agent gets its title, text and branches after the task.`;
    case "pull_request_merged":
      return `A pull request that merges in ${where}${branches ? ` into ${branches}` : ""} starts a session of its own, whose agent gets its title, text and who merged it after the task, and reads its review on GitHub.`;
    case "release_published":
      return `A release published in ${where} starts a session of its own, whose agent gets its tag, name and notes after the task.`;
    case "checks_failed":
      return `Once all the checks of the newest commit on ${branches || "the default branch"} in ${where} end with one or more failed, a session starts, whose agent gets the commit and the checks that failed. While it works, the branch's next failures start none.`;
    default:
      return "fctl doesn't know this kind of event: choose another.";
  }
}

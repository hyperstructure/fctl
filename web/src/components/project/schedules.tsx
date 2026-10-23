// A project's schedules: tasks fctl starts a session for on its own, once at a set time or again and
// again, on the Claude subscription of the person each runs for. Everyone who sees the project sees
// them and what each run started; people who start sessions set them up and run one now, on their own
// subscription; a schedule's owner, and the project's maintainers and owners, change, pause and delete
// it (changing someone else's makes it theirs).

import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  ArrowRight,
  ArrowUpRight,
  BookMarked,
  CalendarClock,
  ChevronDown,
  CircleCheck,
  CircleX,
  Ellipsis,
  GraduationCap,
  Inbox,
  Package,
  Pause,
  Pencil,
  Play,
  Plus,
  Repeat,
  SkipForward,
  Trash2,
  TriangleAlert,
  UserRoundCheck,
  Webhook,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { StartChoices } from "@/components/session/start";
import {
  Avatar,
  Banner,
  Button,
  CopyButton,
  Dialog,
  IconButton,
  Input,
  Menu,
  MenuItem,
  MenuSeparator,
  Meta,
  Select,
  type SelectOption,
  Skeleton,
  StatusDot,
  Textarea,
  TextareaControls,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago, agoShort, stamp } from "@/lib/format";
import { githubLevel, lowerAccess, onGithub } from "@/lib/github-access";
import { siteUrl } from "@/lib/install";
import { asked, type Choice, effortLabel, modelLabel, projectChoice } from "@/lib/models";
import { keys, queryClient, useMe } from "@/lib/queries";
import {
  browserZone,
  canSchedule,
  clockOf,
  cronProblem,
  cronRepeat,
  DEFAULT_REPEAT,
  describeCron,
  type Every,
  fromWall,
  moment,
  nameFromTask,
  nextRuns,
  ordinal,
  parseCron,
  type Repeat as RepeatChoice,
  repeatCron,
  runOutcome,
  runTrigger,
  scheduleRights,
  scheduleWhen,
  sentence,
  WEEKDAY_NAMES,
  wallParts,
  zoneCity,
  zoneOffset,
  zoneRegion,
  zones,
} from "@/lib/schedules";

// ───────────────────────────── ideas ─────────────────────────────

/** A schedule to start from: what it asks of its agents, and when. */
interface Idea {
  key: string;
  name: string;
  /** What it does, in a line. */
  line: string;
  icon: ReactNode;
  prompt: string;
  repeat: RepeatChoice;
  /** What its agents may do on GitHub, when less than the project allows does. */
  github?: G.GithubAccess;
}

const at = (every: Every, hour: number, weekdays: number[] = [1]): RepeatChoice => ({
  ...DEFAULT_REPEAT,
  every,
  hour,
  minute: 0,
  weekdays,
});

const IDEAS: Idea[] = [
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
    key: "triage",
    name: "Triage new issues",
    line: "Labels, questions and links for each issue opened since the last run.",
    icon: <Inbox />,
    prompt:
      "Triage the issues opened since the previous run. For each, add the labels that fit, say what is missing to act on it, link duplicates and point at the code it concerns. Change no code and close nothing.",
    repeat: at("weekday", 8),
    github: "comment",
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
  model?: string;
  effort?: string;
  github?: G.GithubAccess;
}

/** Tasks handed over from the Sessions tab's task box, by project id, for the form to start with. */
const handed = new Map<string, Draft>();

/** Hands a task to the project's Schedules tab, which opens its form with it. */
export function handOver(project: string, draft: Draft) {
  handed.set(project, draft);
}

const fromIdea = (i: Idea): Draft => ({ name: i.name, prompt: i.prompt, repeat: i.repeat, github: i.github });

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
    key: "skips",
    icon: <SkipForward />,
    text: "Runs don't pile up: one is skipped while the last is still at work, or while the plan it runs on is at its limit.",
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
  return (
    <div className="flex animate-rise flex-col gap-12 py-2">
      <div className="grid items-center gap-x-12 gap-y-10 md:grid-cols-[minmax(0,1fr)_300px]">
        <div>
          <h2 className="font-display text-[28px] leading-[1.15] font-medium tracking-[-0.018em] text-ink">
            Sessions that start on their own
          </h2>
          <p className="mt-3 max-w-xl text-[15px] leading-relaxed text-ink-3">
            Give an agent a task and a time: once, or again and again. Each run is a session of its own, on
            your Claude subscription, that you follow like any other.
          </p>
          <ul className="mt-6 flex max-w-xl flex-col gap-3">
            {POINTS.map((p) => (
              <li key={p.key} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
                <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-3 [&_svg]:size-4">
                  {p.icon}
                </span>
                {p.text}
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
          <ul className="border-t border-line">
            {IDEAS.map((i) => (
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
                    <span className="block text-[14px] text-ink">{i.name}</span>
                    <span className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">{i.line}</span>
                    <span className="mt-1 block text-[12px] text-ink-4 sm:hidden">
                      {sentence(describeCronText(repeatCron(i.repeat)))}
                    </span>
                  </span>
                  <span className="shrink-0 text-[12.5px] text-ink-4 max-sm:hidden">
                    {sentence(describeCronText(repeatCron(i.repeat)))}
                  </span>
                  <ArrowRight className="size-3.5 shrink-0 text-ink-4 transition-transform duration-200 group-hover:translate-x-0.5" />
                </button>
              </li>
            ))}
          </ul>
        </section>
      ) : (
        <p className="text-[13px] text-ink-4">People who start sessions in this project set up schedules.</p>
      )}
    </div>
  );
}

/** A cron expression in words, in the browser's way of writing times. */
function describeCronText(text: string): string {
  const parsed = parseCron(text);
  return "error" in parsed ? text : describeCron(parsed.cron, clockOf);
}

/** What a schedule is: when it runs, through the week, and the sessions it started. */
function Preview() {
  const days = ["Mon", "Tue", "Wed", "Thu", "Fri", "Sat", "Sun"];
  return (
    <div aria-hidden="true" className="relative select-none max-md:max-w-sm">
      <div className="overflow-hidden rounded-xl border border-line bg-surface shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]">
        <div className="flex items-center gap-2.5 border-b border-line px-3.5 py-3">
          <span className="flex size-7 items-center justify-center rounded-lg bg-raised text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line)]">
            <Repeat className="size-3.5" />
          </span>
          <div className="min-w-0">
            <p className="text-[13px] font-medium text-ink">Update dependencies</p>
            <p className="text-[11.5px] text-ink-3">Every Monday at {clockOf(9, 0)}</p>
          </div>
        </div>
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
      <div className="mt-3 flex flex-col gap-1.5 rounded-xl border border-line bg-surface px-3.5 py-2.5 text-[12px] shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]">
        {[
          ["#41", "Update 12 dependencies", "idle", "6d"],
          ["#37", "Update 3 dependencies", "stopped", "13d"],
        ].map(([n, title, status, when]) => (
          <div key={n} className="flex items-center gap-2">
            <StatusDot status={status as G.SessionStatus} />
            <span className="font-mono text-ink-4">{n}</span>
            <span className="min-w-0 flex-1 truncate text-ink-2">{title}</span>
            <span className="text-ink-4">{when}</span>
          </div>
        ))}
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

/** The facts under a schedule's name: when it runs, when next, and what its last run did. */
function Facts({ schedule: s }: { schedule: G.Schedule }) {
  const here = useMemo(browserZone, []);
  const last = s.last_run;
  return (
    <Meta
      parts={[
        { key: "when", node: <span className="text-ink-3">{scheduleWhen(s)}</span> },
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
  const [dialog, setDialog] = useState<"delete" | "resume" | "take">();
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
          <span className="flex items-center gap-2">
            {/* On a phone a long name wraps: beside the buttons, little of it would show. */}
            <span className="min-w-0 text-[14.5px] text-ink [overflow-wrap:anywhere] sm:truncate">
              {s.name}
            </span>
            <ChevronDown
              className={cn(
                "size-3.5 shrink-0 text-ink-4 transition-transform duration-200",
                open && "rotate-180",
              )}
            />
          </span>
          <Facts schedule={s} />
          {(s.paused_reason || s.problem) && (
            <span className="mt-1 block text-[12.5px] leading-relaxed text-amber [overflow-wrap:anywhere]">
              {s.paused_reason ? `fctl paused it. ${s.paused_reason}` : s.problem}
            </span>
          )}
        </button>
        {s.owner && (
          <span
            className="relative mt-1 max-sm:hidden"
            title={
              s.cron || s.at !== undefined
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
            onClick={() => run.mutate()}
            aria-label={`Run ${s.name} now, on your Claude subscription`}
            title="Run now, on your Claude subscription"
            className="relative max-sm:px-2 [&>span]:max-sm:sr-only"
          >
            <span>Run now</span>
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
            className="relative max-sm:px-2 [&>span]:max-sm:sr-only"
          >
            <span>Resume</span>
          </Button>
        )}
        {(rights.edit || rights.manage) && (
          <Menu
            className="relative"
            trigger={(toggle, menuOpen) => (
              <IconButton
                label="Schedule actions"
                aria-haspopup="menu"
                aria-expanded={menuOpen}
                onClick={toggle}
                className="-my-0.5"
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
    </li>
  );
}

/** Runs a schedule now, in a session on your subscription, and opens it. */
function useRunNow(s: G.Schedule) {
  const navigate = useNavigate();
  return useMutation({
    mutationFn: () => api.runSchedule(s.id),
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
          {s.base_branch && (
            <Fact k="From">
              <span className="font-mono">{s.base_branch}</span>
            </Fact>
          )}
          {!s.cron && s.at === undefined ? (
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
  const timed = !!s.cron || s.at !== undefined;
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
        <span className="mt-0.5 block text-[12px] text-ink-4">{runTrigger(r)}</span>
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
      toast(`Deleted ${s.name}`);
      onClose();
    },
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Delete ${s.name}?`}
      description="It starts no more sessions. The sessions it started stay, with their transcripts."
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

type Mode = "repeat" | "once" | "cron" | "asked";

const MODES: { value: Mode; label: string }[] = [
  { value: "repeat", label: "Repeats" },
  { value: "once", label: "Once" },
  { value: "cron", label: "Cron" },
  { value: "asked", label: "When asked" },
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
  const [zone, setZone] = useState(schedule?.timezone ?? here);
  const [pickZone, setPickZone] = useState(false);
  const initialRepeat = (schedule?.cron && cronRepeat(schedule.cron)) || draft?.repeat || DEFAULT_REPEAT;
  const [mode, setMode] = useState<Mode>(() => {
    if (!schedule) return "repeat";
    if (schedule.cron) return cronRepeat(schedule.cron) ? "repeat" : "cron";
    return schedule.at !== undefined ? "once" : "asked";
  });
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
      : mode === "once"
        ? Number.isNaN(atTime)
          ? "Choose a day and a time"
          : atTime < Date.now() - 60_000
            ? "That time has passed: choose one ahead"
            : undefined
        : cronProblem(expression);
  const upcoming = useMemo(() => {
    if (whenProblem || mode === "asked") return [];
    if (mode === "once") return [atTime];
    const parsed = parseCron(expression);
    return "error" in parsed ? [] : nextRuns(parsed.cron, zone, Date.now(), 3);
  }, [mode, atTime, expression, zone, whenProblem]);

  const save = useMutation({
    mutationFn: async () => {
      const asks = asked(project, choice);
      // Without a time, it runs when asked: a new one gets neither, a change clears the cron.
      const timing =
        mode === "once"
          ? { at: atTime }
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
      };
      return api.updateSchedule(schedule.id, body);
    },
    onSuccess: (s) => {
      saved(project, s);
      toast(
        schedule
          ? "Saved: its next run goes by it"
          : s.next_run_at
            ? `Set up: it runs ${soon(s.next_run_at, s.timezone)}`
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
          {IDEAS.map((i) => (
            <button
              key={i.key}
              type="button"
              onClick={() => {
                setName(i.name);
                setPrompt(i.prompt);
                setMode("repeat");
                setRepeat(i.repeat);
                setCron(repeatCron(i.repeat));
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
          <div className="inline-flex rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)]">
            {MODES.map((m) => (
              <button
                key={m.value}
                type="button"
                aria-pressed={mode === m.value}
                onClick={() => {
                  if (m.value === "cron" && mode === "repeat") setCron(repeatCron(repeat));
                  if (m.value === "repeat" && mode === "cron") {
                    const r = cronRepeat(cron);
                    if (r) setRepeat(r);
                  }
                  setMode(m.value);
                }}
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
            mode === "asked" && "hidden",
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

        <Upcoming
          times={upcoming}
          zone={zone}
          here={here}
          problem={fieldError("cron", "at") ?? whenProblem}
          once={mode === "once"}
        />
      </fieldset>

      {save.error && !failure?.field && <Banner tone="error">{save.error.message}</Banner>}
      {failure?.field === "claude" && (
        <Banner tone="error">
          {failure.message}{" "}
          <Link
            to="/settings"
            search={{ linked: undefined, error: undefined }}
            className="underline underline-offset-2"
          >
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

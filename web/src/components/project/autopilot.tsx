// A project's Autopilot tab (experimental): an agent that runs the project, or its whole workspace,
// on its own. Before it's on, the tab says what it does and offers directions to start from; once
// on, it's where people watch it work and steer it: where it stands, what it asks them (secrets
// too, typed in here and never into a transcript), their notes, its plan, the sessions it started
// and its log, all live while the tab is open.

import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  ArrowRight,
  BookOpenCheck,
  Check,
  ChevronDown,
  CircleAlert,
  Clock,
  Ellipsis,
  FlaskConical,
  FolderPlus,
  GitFork,
  KeyRound,
  ListChecks,
  MessageCircleQuestion,
  NotebookPen,
  Pause,
  Pencil,
  Play,
  Plus,
  Power,
  RefreshCw,
  Repeat,
  Send,
  Settings2,
  ShieldCheck,
  Sparkles,
  SquareTerminal,
  Undo2,
  UserRoundCheck,
  UsersRound,
  Waypoints,
  X,
  Zap,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { Markdown } from "@/components/markdown";
import { ProjectMark } from "@/components/project/project-icon";
import { ScheduleNamed } from "@/components/session/autopilot";
import {
  AutopilotIcon,
  Avatar,
  Banner,
  Button,
  Dialog,
  IconButton,
  Input,
  Menu,
  MenuItem,
  MenuSeparator,
  SecretInput,
  Select,
  type SelectOption,
  Skeleton,
  Spinner,
  StatusDot,
  statusLabel,
  Textarea,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import {
  atWork,
  CHECK_INS,
  CREW_MAX,
  checkInWords,
  DEFAULT_CHECK_IN,
  DEFAULT_CREW,
  DIRECTIONS,
  type DirectionIdea,
  defaultPowers,
  HANDOFF_CHOICES,
  type HandoffChoice,
  handoffChoice,
  handoffWords,
  hasPlaceholders,
  openAsks,
  POWERS,
  phase,
  planDone,
  wakeWords,
} from "@/lib/autopilot";
import { cn } from "@/lib/cn";
import { MAX_TOKENS, MIN_TOKENS, parseTokens, tokensText, validWindow } from "@/lib/context";
import { ago, agoShort, agoShortWords, clock, day, num, stamp } from "@/lib/format";
import { EFFORTS, effortLabel, modelLabel, modelOffers } from "@/lib/models";
import { keys, queryClient, useMe, useProjects, useWorkspace } from "@/lib/queries";

// ───────────────────────────── the tab ─────────────────────────────

/** What the tab shows: the autopilot once it is on or paused, else what putting it on does. */
export function AutopilotTab({ project }: { project: G.Project }) {
  const q = useAutopilot(project.id);
  const [engaging, setEngaging] = useState<Draft | null>(null);
  const a = q.data;
  if (q.isPending)
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-28 w-full rounded-2xl" />
        <Skeleton className="h-40 w-full rounded-2xl" />
      </div>
    );
  if (q.isError) return <Banner tone="error">Its autopilot didn't load: {q.error.message}</Banner>;
  if (!a || a.state === "off") {
    if (engaging)
      return (
        <Engage
          project={project}
          draft={engaging}
          before={a ?? undefined}
          onCancel={() => setEngaging(null)}
          onDone={() => setEngaging(null)}
        />
      );
    return <Intro project={project} before={a ?? undefined} onStart={setEngaging} />;
  }
  return <Cockpit project={project} a={a} />;
}

/** The autopilot of `project` (id or slug), or null while it has none; polled while the tab shows it. */
export function useAutopilot(project: string) {
  return useQuery({
    queryKey: keys.autopilot(project),
    queryFn: () =>
      api.autopilot(project).catch((e) => {
        if (e instanceof ApiFailure && e.status === 404) return null;
        throw e;
      }),
    // While it works, what it does shows within seconds; resting, within a quarter minute.
    refetchInterval: (q) => {
      const a = q.state.data;
      if (!a || a.state === "off") return false;
      return atWork(a) ? 4_000 : 15_000;
    },
  });
}

function patched(a: G.Autopilot) {
  queryClient.setQueryData(keys.autopilot(a.project_id), a);
  queryClient.setQueryData(keys.autopilot(a.project_slug), a);
  void queryClient.invalidateQueries({ queryKey: keys.autopilotLog(a.project_id) });
  // Its tab's mark and count, in the project and the lists.
  void queryClient.invalidateQueries({ queryKey: ["project"] });
  void queryClient.invalidateQueries({ queryKey: keys.projects });
  void queryClient.invalidateQueries({ queryKey: keys.autopilotAsks });
}

function failed(e: unknown, what: string) {
  toast(`${what}: ${e instanceof Error ? e.message : "something went wrong"}`, "error");
}

/** "Experimental", beside the feature's name wherever it is introduced. */
export function ExperimentalPill({ className }: { className?: string }) {
  return (
    <span
      title="An experimental feature of fctl: it may change, and it acts on its own"
      className={cn(
        "inline-flex h-5 items-center gap-1 rounded-full border border-pink/30 bg-pink-soft px-2 text-[11px] font-medium text-pink",
        className,
      )}
    >
      <FlaskConical className="size-3" aria-hidden="true" />
      Experimental
    </span>
  );
}

/** Its mark in a tile, lit while it works. */
export function AutopilotTile({
  live,
  size = 40,
  className,
}: {
  live?: boolean;
  size?: number;
  className?: string;
}) {
  return (
    <span
      aria-hidden="true"
      style={{ width: size, height: size }}
      className={cn(
        "relative flex shrink-0 items-center justify-center rounded-xl bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]",
        live && "shadow-[inset_0_0_0_1px_var(--c-line),0_0_0_4px_var(--c-accent-soft)]",
        className,
      )}
    >
      <AutopilotIcon live={live} className={size >= 36 ? "size-5" : "size-4"} />
    </span>
  );
}

// ───────────────────────────── before it's on ─────────────────────────────

/** What a form to put a project on autopilot starts from. */
interface Draft {
  direction?: string;
}

const POINTS: { key: string; icon: ReactNode; text: (crew: number, every: string) => ReactNode }[] = [
  {
    key: "plans",
    icon: <ListChecks />,
    text: (crew) => (
      <>
        <span className="text-ink">Plans and delegates.</span> It keeps a plan, and starts up to {crew}{" "}
        sessions at a time to work through it, each with a pull request it merges once checks pass, so the
        next builds on it.
      </>
    ),
  },
  {
    key: "sets-up",
    icon: <Settings2 />,
    text: () => (
      <>
        <span className="text-ink">Sets up what the work needs.</span> The project's settings, repositories,
        schedules and secrets, as far as you let it.
      </>
    ),
  },
  {
    key: "keeps-going",
    icon: <Repeat />,
    text: (_, every) => (
      <>
        <span className="text-ink">Keeps going.</span> It wakes when one of its sessions finishes or you steer
        it, and {every} at least. If its own session stops, it starts a fresh one.
      </>
    ),
  },
  {
    key: "asks",
    icon: <MessageCircleQuestion />,
    text: () => (
      <>
        <span className="text-ink">Asks without waiting.</span> Decisions and credentials come to you here
        while it works on what it can. Secrets go into the project's secrets, never into a transcript.
      </>
    ),
  },
];

function Intro({
  project,
  before,
  onStart,
}: {
  project: G.Project;
  before?: G.Autopilot;
  onStart: (d: Draft) => void;
}) {
  const can = project.role === "owner" || project.role === "maintainer";
  const allowed = project.autopilot?.allowed ?? false;
  const workspace = project.workspace?.name ?? "its workspace";
  return (
    <div className="flex animate-rise flex-col gap-12 py-2">
      <div className="grid items-center gap-x-12 gap-y-10 md:grid-cols-[minmax(0,1fr)_300px]">
        <div>
          <ExperimentalPill />
          <h2 className="mt-4 font-display text-[28px] leading-[1.15] font-medium tracking-[-0.018em] text-ink">
            A project that runs itself
          </h2>
          <p className="mt-3 max-w-xl text-[15px] leading-relaxed text-ink-3">
            Give {project.name} a direction, and an agent takes it from there: it plans the work, starts
            sessions to do it, and sets up what they need. It acts as you, on your Claude subscription, and
            asks you when only a person can decide.
          </p>
          <ul className="mt-6 flex max-w-xl flex-col gap-3">
            {POINTS.map((p) => (
              <li key={p.key} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
                <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-3 [&_svg]:size-4">
                  {p.icon}
                </span>
                <span className="min-w-0">{p.text(DEFAULT_CREW, checkInWords(DEFAULT_CHECK_IN))}</span>
              </li>
            ))}
          </ul>
        </div>
        {/* On a phone the directions below say it, and come sooner. */}
        <div className="max-sm:hidden">
          <Preview />
        </div>
      </div>
      {!allowed ? (
        <Banner tone="info">
          {workspace}'s admins let projects have autopilots in the workspace's settings, where it's an
          experimental feature.
          {project.workspace && (
            <>
              {" "}
              <Link
                to="/w/$slug"
                params={{ slug: project.workspace.slug }}
                search={{ tab: "settings" }}
                className="underline underline-offset-2"
              >
                Its settings
              </Link>
            </>
          )}
        </Banner>
      ) : !can ? (
        <p className="text-[13px] text-ink-4">
          The project's maintainers and owners put it on autopilot, on their own Claude subscription.
        </p>
      ) : (
        <section aria-labelledby="directions-heading">
          <div className="mb-3 flex flex-wrap items-end justify-between gap-3">
            <div>
              <h3 id="directions-heading" className="text-[14px] font-semibold text-ink">
                {before ? "Put it on autopilot again" : "Start from a direction"}
              </h3>
              {before && (
                <p className="mt-1 text-[12.5px] text-ink-3">
                  It keeps the direction, notes, plan and log it had, and what it still asks: change the
                  direction as you put it on.
                </p>
              )}
            </div>
            <Button
              size="sm"
              variant={before ? "primary" : "secondary"}
              icon={before ? <Play className="size-3.5" /> : <Plus className="size-3.5" />}
              onClick={() => onStart({ direction: before?.direction })}
            >
              {before ? "Put it on again…" : "Write your own"}
            </Button>
          </div>
          <ul className="border-t border-line">
            {DIRECTIONS.map((d) => (
              <li key={d.key}>
                <button
                  type="button"
                  onClick={() => onStart({ direction: d.direction })}
                  className="group flex w-full items-center gap-3.5 border-b border-line px-3 py-3 text-left transition-colors hover:bg-hover/40 focus-visible:bg-hover/40 focus-visible:outline-none"
                >
                  <span
                    aria-hidden="true"
                    className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-raised text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line)] [&_svg]:size-4"
                  >
                    {IDEA_ICONS[d.key]}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block text-[14px] text-ink">{d.name}</span>
                    <span className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">{d.line}</span>
                  </span>
                  <ArrowRight className="size-3.5 shrink-0 text-ink-4 transition-transform duration-200 group-hover:translate-x-0.5" />
                </button>
              </li>
            ))}
          </ul>
          <p className="mt-4 max-w-2xl text-[12.5px] leading-relaxed text-ink-4">
            Experimental: it acts on its own with a maintainer's powers in {project.name}, on your
            subscription. Watch what it does here, and pause it or take it off autopilot at any time.
          </p>
        </section>
      )}
    </div>
  );
}

const IDEA_ICONS: Record<string, ReactNode> = {
  build: <Sparkles />,
  healthy: <ShieldCheck />,
  issues: <BookOpenCheck />,
  polish: <Waypoints />,
};

/** What an autopilot looks like at work: its state, its plan and the sessions it runs, small and made up. */
function Preview() {
  return (
    <div aria-hidden="true" className="relative select-none max-md:max-w-sm">
      <div className="overflow-hidden rounded-xl border border-line bg-surface shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]">
        <div className="flex items-center gap-2.5 border-b border-line px-3.5 py-3">
          <AutopilotTile live size={28} className="rounded-lg" />
          <div className="min-w-0">
            <p className="text-[13px] font-medium text-ink">At work</p>
            <p className="text-[11.5px] text-ink-3">Checks in every hour</p>
          </div>
        </div>
        <ul className="flex flex-col gap-1.5 px-3.5 py-3 text-[12px]">
          {[
            ["Contacts and companies", true],
            ["Pipeline board", false],
            ["Import from CSV", false],
          ].map(([item, done]) => (
            <li key={String(item)} className="flex items-center gap-2">
              <span
                className={cn(
                  "flex size-3.5 items-center justify-center rounded-[4px] border",
                  done ? "border-accent bg-accent text-highlight-ink" : "border-line-strong",
                )}
              >
                {done && <Check className="size-2.5" strokeWidth={3} />}
              </span>
              <span className={done ? "text-ink-4 line-through" : "text-ink-2"}>{item}</span>
            </li>
          ))}
        </ul>
      </div>
      <div className="mt-3 flex flex-col gap-1.5 rounded-xl border border-line bg-surface px-3.5 py-2.5 text-[12px] shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]">
        {(
          [
            ["#12", "Pipeline board with drag and drop", "running"],
            ["#11", "Contacts and companies", "stopped"],
          ] as const
        ).map(([n, title, status]) => (
          <div key={n} className="flex items-center gap-2">
            <StatusDot status={status} />
            <span className="font-mono text-ink-4">{n}</span>
            <span className="min-w-0 flex-1 truncate text-ink-2">{title}</span>
          </div>
        ))}
      </div>
      <div className="mt-3 rounded-xl border border-blue/30 bg-surface bg-[linear-gradient(to_bottom,var(--c-blue-soft),transparent_80%)] px-3.5 py-2.5 text-[12px] shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]">
        <p className="flex items-center gap-1.5 font-medium text-blue">
          <KeyRound className="size-3" />
          It needs a secret
        </p>
        <p className="mt-1 text-ink-3">
          <span className="font-mono text-ink-2">CLOUDFLARE_API_TOKEN</span> to deploy from CI
        </p>
      </div>
    </div>
  );
}

// ───────────────────────────── putting it on ─────────────────────────────

/** The form that puts the project on autopilot. */
function Engage({
  project,
  draft,
  before,
  onCancel,
  onDone,
}: {
  project: G.Project;
  draft: Draft;
  before?: G.Autopilot;
  onCancel: () => void;
  onDone: () => void;
}) {
  const workspace = useWorkspace(project.workspace?.id);
  const admin = workspace.data?.role === "owner" || workspace.data?.role === "admin";
  const taken = workspace.data?.autopilot && workspace.data.autopilot.project_id !== project.id;
  const [direction, setDirection] = useState(draft.direction ?? "");
  const [scope, setScope] = useState<G.AutopilotScope>(before?.scope ?? "project");
  const [powers, setPowers] = useState<G.AutopilotPower[]>(before?.powers ?? defaultPowers("project"));
  const [checkIn, setCheckIn] = useState(before?.check_in_minutes ?? DEFAULT_CHECK_IN);
  const [crew, setCrew] = useState(before?.crew_max ?? DEFAULT_CREW);
  const [model, setModel] = useState(before?.model ?? "");
  const [effort, setEffort] = useState(before?.effort ?? "");
  const text = useRef<HTMLTextAreaElement>(null);
  const form = useRef<HTMLFormElement>(null);
  useEffect(() => {
    form.current?.scrollIntoView({ block: "nearest", behavior: "smooth" });
    text.current?.focus();
  }, []);
  // A workspace's autopilot may create projects; a project's may not.
  useEffect(() => {
    setPowers((p) =>
      scope === "workspace"
        ? p.includes("projects")
          ? p
          : [...p, "projects"]
        : p.filter((x) => x !== "projects"),
    );
  }, [scope]);
  const put = useMutation({
    mutationFn: () =>
      api.putAutopilot(project.id, {
        direction,
        scope,
        powers,
        check_in_minutes: checkIn,
        crew_max: crew,
        model: model || "default",
        effort: effort || "default",
      }),
    onSuccess: (a) => {
      patched(a);
      toast(`${project.name} is on autopilot: it wakes now`);
      onDone();
    },
    onError: (e) => failed(e, "It isn't on"),
  });
  const placeholders = hasPlaceholders(direction);
  const empty = !direction.trim();
  return (
    <form
      ref={form}
      onSubmit={(e) => {
        e.preventDefault();
        if (!empty) put.mutate();
      }}
      className="animate-rise scroll-mt-6 rounded-2xl border border-line-strong bg-surface shadow-[0_24px_60px_-30px_rgb(0_0_0/0.4)]"
    >
      <div className="flex items-start gap-3.5 border-b border-line px-5 py-4">
        <AutopilotTile size={36} />
        <div className="min-w-0 flex-1">
          <h2 className="flex flex-wrap items-center gap-2 text-[16px] font-semibold text-ink">
            Put {project.name} on autopilot
            <ExperimentalPill />
          </h2>
          <p className="mt-0.5 text-[13px] text-ink-3">
            Say where to take it. You can change the direction, and steer it with notes, at any time.
          </p>
        </div>
        <IconButton type="button" label="Cancel" onClick={onCancel}>
          <X className="size-4" />
        </IconButton>
      </div>
      <div className="flex flex-col gap-7 px-5 py-5">
        <div>
          <div className="mb-2 flex flex-wrap items-center gap-1.5">
            <span className="mr-1 text-[12.5px] text-ink-4">Start from</span>
            {DIRECTIONS.map((d: DirectionIdea) => (
              <button
                key={d.key}
                type="button"
                onClick={() => {
                  setDirection(d.direction);
                  text.current?.focus();
                }}
                className={cn(
                  "inline-flex h-7 items-center gap-1.5 rounded-full border px-2.5 text-[12.5px] transition-colors [&_svg]:size-3.5",
                  direction === d.direction
                    ? "border-ink-4 bg-raised text-ink"
                    : "border-line text-ink-3 hover:border-line-strong hover:text-ink",
                )}
              >
                {IDEA_ICONS[d.key]}
                {d.name}
              </button>
            ))}
          </div>
          <label htmlFor="autopilot-direction" className="sr-only">
            Where it should take {project.name}
          </label>
          <Textarea
            id="autopilot-direction"
            ref={text}
            value={direction}
            onChange={(e) => setDirection(e.target.value)}
            minRows={7}
            maxRows={22}
            placeholder={`Where should it take ${project.name}? What to build or reach, for whom, and what to leave alone.`}
            className="font-[inherit] text-[14px]"
            onKeyDown={(e) => {
              if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && !empty) {
                e.preventDefault();
                put.mutate();
              }
            }}
          />
          <p className={cn("mt-1.5 text-[12.5px]", placeholders ? "text-amber" : "text-ink-4")}>
            {placeholders
              ? "Fill in the parts in brackets: they're what makes the direction this project's."
              : "Markdown. The more it knows of why, the better it decides what you didn't say."}
          </p>
        </div>
        {before?.scope === "workspace" && (
          <p className="text-[12.5px] text-ink-3">
            It ran all of {workspace.data?.name ?? "the workspace"} before, and what it kept reaches every
            project: it stays the workspace's.
          </p>
        )}
        {admin && !taken && before?.scope !== "workspace" && (
          <fieldset>
            <legend className="mb-2 text-[12.5px] font-medium text-ink-2">What it runs</legend>
            <div className="grid gap-2 sm:grid-cols-2">
              {(
                [
                  [
                    "project",
                    `${project.name}`,
                    "It works in this project's sessions, settings and repositories.",
                  ],
                  [
                    "workspace",
                    `All of ${workspace.data?.name ?? "the workspace"}`,
                    "Every project of the workspace, from this one: it starts sessions in any and creates new ones.",
                  ],
                ] as const
              ).map(([value, label, line]) => (
                <label
                  key={value}
                  className={cn(
                    "flex cursor-pointer items-start gap-3 rounded-xl border px-3.5 py-3 transition-colors",
                    scope === value ? "border-ink-4 bg-raised" : "border-line hover:border-line-strong",
                  )}
                >
                  <input
                    type="radio"
                    name="autopilot-scope"
                    value={value}
                    checked={scope === value}
                    onChange={() => setScope(value)}
                    className="mt-1 accent-[var(--c-ink)]"
                  />
                  <span className="min-w-0">
                    <span className="block text-[13.5px] text-ink">{label}</span>
                    <span className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">{line}</span>
                  </span>
                </label>
              ))}
            </div>
          </fieldset>
        )}
        <PowersField scope={scope} powers={powers} onChange={setPowers} />
        <div className="grid gap-5 sm:grid-cols-2">
          <RhythmFields checkIn={checkIn} setCheckIn={setCheckIn} crew={crew} setCrew={setCrew} />
        </div>
        <ModelFields
          project={project}
          model={model}
          setModel={setModel}
          effort={effort}
          setEffort={setEffort}
        />
      </div>
      <div className="flex flex-wrap items-center justify-end gap-x-4 gap-y-3 border-t border-line px-5 py-3.5">
        <p className="mr-auto max-w-md text-[12.5px] leading-snug text-ink-3">
          It runs on your Claude subscription and acts as you
          {scope === "workspace" ? " in every project" : ""}: its own sessions and those it starts.
        </p>
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          type="submit"
          variant="primary"
          loading={put.isPending}
          disabled={empty}
          icon={<AutopilotIcon className="size-4" />}
        >
          Put on autopilot
        </Button>
      </div>
    </form>
  );
}

/** What it may do besides starting and steering sessions, which it always does. */
function PowersField({
  scope,
  powers,
  onChange,
}: {
  scope: G.AutopilotScope;
  powers: G.AutopilotPower[];
  onChange: (p: G.AutopilotPower[]) => void;
}) {
  const shown = POWERS.filter((p) => scope === "workspace" || !p.workspace);
  return (
    <fieldset>
      <legend className="mb-2 text-[12.5px] font-medium text-ink-2">What it may do</legend>
      <ul className="overflow-hidden rounded-xl border border-line">
        <li className="flex items-start gap-3 bg-sunken/40 px-3.5 py-2.5">
          <input type="checkbox" checked disabled className="mt-1" aria-describedby="power-sessions" />
          <span className="min-w-0">
            <span className="block text-[13.5px] text-ink">Start and steer sessions</span>
            <span id="power-sessions" className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">
              Always: it works through them, up to as many at once as you allow, and messages, stops and
              resumes those it started.
            </span>
          </span>
        </li>
        {shown.map((p) => {
          const on = powers.includes(p.power);
          const id = `power-${p.power}`;
          return (
            <li key={p.power} className="border-t border-line">
              <label
                htmlFor={id}
                className="flex cursor-pointer items-start gap-3 px-3.5 py-2.5 hover:bg-hover/30"
              >
                <input
                  id={id}
                  type="checkbox"
                  checked={on}
                  onChange={() => onChange(on ? powers.filter((x) => x !== p.power) : [...powers, p.power])}
                  className="mt-1 accent-[var(--c-ink)]"
                />
                <span className="min-w-0">
                  <span className="block text-[13.5px] text-ink">{p.label}</span>
                  <span className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">{p.line}</span>
                </span>
              </label>
            </li>
          );
        })}
      </ul>
      <p className="mt-1.5 text-[12.5px] text-ink-4">
        Never: people and their roles, access tokens, runners, GitHub accounts, deleting projects.
      </p>
    </fieldset>
  );
}

function RhythmFields({
  checkIn,
  setCheckIn,
  crew,
  setCrew,
}: {
  checkIn: number;
  setCheckIn: (n: number) => void;
  crew: number;
  setCrew: (n: number) => void;
}) {
  const checkIns: SelectOption[] = CHECK_INS.map((c) => ({ value: String(c.minutes), label: c.label }));
  const crews: SelectOption[] = Array.from({ length: CREW_MAX }, (_, i) => ({
    value: String(i + 1),
    label: i === 0 ? "1 session at a time" : `${i + 1} sessions at once`,
  }));
  return (
    <>
      <div className="flex flex-col gap-1.5">
        <span className="text-[12.5px] font-medium text-ink-2">It checks in</span>
        <Select
          aria-label="How often it checks in"
          value={String(checkIn)}
          options={checkIns}
          onChange={(v) => setCheckIn(Number(v))}
        />
        <span className="text-[12.5px] text-ink-4">At least: it wakes sooner when something needs it.</span>
      </div>
      <div className="flex flex-col gap-1.5">
        <span className="text-[12.5px] font-medium text-ink-2">It runs</span>
        <Select
          aria-label="How many of its sessions work at once"
          value={String(crew)}
          options={crews}
          onChange={(v) => setCrew(Number(v))}
        />
        <span className="text-[12.5px] text-ink-4">Each on your subscription and a runner's slot.</span>
      </div>
    </>
  );
}

function ModelFields({
  project,
  model,
  setModel,
  effort,
  setEffort,
}: {
  project: G.Project;
  model: string;
  setModel: (m: string) => void;
  effort: string;
  setEffort: (e: string) => void;
}) {
  const own = project.settings.model ?? "";
  const models: SelectOption[] = [
    { value: "", label: `The project's (${modelLabel(own)})` },
    ...modelOffers(project, model)
      .filter((o) => o.value)
      .map((o) => ({ value: o.value, label: o.label })),
  ];
  const ownEffort = project.settings.effort ?? "";
  const efforts: SelectOption[] = [
    { value: "", label: `The project's (${effortLabel(ownEffort)})` },
    ...EFFORTS.map((e) => ({ value: e.value, label: e.label })),
  ];
  return (
    <div>
      <span className="mb-1.5 block text-[12.5px] font-medium text-ink-2">Its own sessions think with</span>
      <div className="grid gap-2 sm:grid-cols-2">
        <Select aria-label="Its model" value={model} options={models} onChange={setModel} />
        <Select aria-label="Its effort level" value={effort} options={efforts} onChange={setEffort} />
      </div>
      <p className="mt-1.5 text-[12.5px] text-ink-4">
        Planning and steering take judgment: the most capable model at high effort serves it well. The
        sessions it starts choose their own.
      </p>
    </div>
  );
}

// ───────────────────────────── on ─────────────────────────────

function Cockpit({ project, a }: { project: G.Project; a: G.Autopilot }) {
  const asks = openAsks(a);
  const can = a.can.steer;
  // A workspace's autopilot works across the workspace: what it does is its admins' to see.
  if (!a.can.read)
    return (
      <div className="flex animate-rise flex-col gap-6">
        <Header project={project} a={a} />
        <div className="grid gap-x-8 gap-y-6 md:grid-cols-[minmax(0,1fr)_288px]">
          <Card title="What it does" icon={<ShieldCheck />}>
            <p className="text-[13.5px] leading-relaxed text-ink-3">
              It works across {project.workspace?.name ?? "the workspace"}, in projects you may not see, so
              its plan, sessions, asks and log are for the workspace's admins, who steer it. Its sessions in
              your projects show there, with its mark.
            </p>
          </Card>
          <Rules a={a} project={project} />
        </div>
      </div>
    );
  return (
    <div className="flex animate-rise flex-col gap-6">
      <Header project={project} a={a} />
      {asks.length > 0 && (
        <section aria-labelledby="asks-heading" className="flex flex-col gap-3">
          <h2 id="asks-heading" className="flex items-center gap-2 text-[14px] font-semibold text-ink">
            It asks you
            <span className="inline-flex h-[18px] min-w-[18px] items-center justify-center rounded-full bg-blue px-1.5 text-[10.5px] font-semibold text-white tabular-nums">
              {asks.length}
            </span>
          </h2>
          {asks.map((ask) => (
            <AskCard key={ask.id} a={a} ask={ask} />
          ))}
        </section>
      )}
      {can && a.state !== "off" && <Steer a={a} />}
      <div className="grid gap-x-8 gap-y-6 md:grid-cols-[minmax(0,1fr)_288px]">
        <div className="flex min-w-0 flex-col gap-6 max-md:order-2">
          <Log a={a} />
        </div>
        <aside
          className="flex min-w-0 flex-col gap-4 max-md:order-1"
          aria-label="Its direction, plan and sessions"
        >
          <Direction a={a} />
          <Plan a={a} />
          <Crew a={a} />
          <Rules a={a} project={project} />
        </aside>
      </div>
    </div>
  );
}

/** Each phase's tone: its word, its dot, the wash from the card's corner and the card's border. */
export const TONES: Record<
  "accent" | "blue" | "pink" | "ink",
  { word: string; dot: string; glow: string; border: string }
> = {
  accent: {
    word: "text-accent",
    dot: "bg-accent",
    glow: "bg-[radial-gradient(120%_140%_at_0%_0%,var(--c-accent-soft),transparent_60%)]",
    border: "border-line-strong",
  },
  blue: {
    word: "text-blue",
    dot: "bg-blue",
    glow: "bg-[radial-gradient(120%_140%_at_0%_0%,var(--c-blue-soft),transparent_60%)]",
    border: "border-line",
  },
  pink: {
    word: "text-pink",
    dot: "bg-pink",
    glow: "bg-[radial-gradient(120%_140%_at_0%_0%,var(--c-pink-soft),transparent_60%)]",
    border: "border-pink/25",
  },
  ink: { word: "text-ink-3", dot: "bg-ink-4", glow: "", border: "border-line" },
};

/** Where it stands, and what people do with it from here. */
function Header({ project, a }: { project: G.Project; a: G.Autopilot }) {
  const now = useNow(15_000);
  const p = phase(a, now);
  const tone = TONES[p.tone];
  const live = atWork(a);
  const [off, setOff] = useState(false);
  const [settings, setSettings] = useState(false);
  const [taking, setTaking] = useState(false);
  const me = useMe().data;
  const mine = !!me && a.owner?.id === me.user.id;
  const { fresh } = useMoves(a);
  return (
    <section
      aria-label="Where it stands"
      // Nothing clips the card, so that its ⋯ menu reaches past its foot; its glow and problem strip round
      // their own corners (its radius less its border).
      className={cn("relative rounded-2xl border bg-surface", tone.border)}
    >
      <div
        aria-hidden="true"
        className={cn("pointer-events-none absolute inset-0 rounded-[15px]", tone.glow)}
      />
      <div className="relative flex flex-wrap items-start gap-x-4 gap-y-4 px-5 py-4.5">
        {/* On a phone its mark sits beside its name, rather than in a row of its own. */}
        <AutopilotTile live={live} size={44} className="max-sm:hidden" />
        <div className="min-w-[min(100%,16rem)] flex-1">
          <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
            <AutopilotTile live={live} size={28} className="rounded-lg sm:hidden" />
            <h2 className="text-[15px] font-semibold text-ink">
              {a.scope === "workspace"
                ? `${project.workspace?.name ?? "The workspace"}'s autopilot`
                : "Autopilot"}
            </h2>
            <ExperimentalPill />
          </div>
          <p aria-live="polite" className="mt-1.5 flex items-start gap-2 text-[14px] leading-snug">
            <span className="relative mt-[6px] inline-flex size-2 shrink-0">
              {live && (
                <span
                  className={cn(
                    "absolute inset-0 animate-ping rounded-full opacity-60 [animation-duration:2s]",
                    tone.dot,
                  )}
                />
              )}
              <span className={cn("relative inline-flex size-2 rounded-full", tone.dot)} />
            </span>
            <span className="min-w-0">
              <span className={cn("font-medium", tone.word)}>{p.word}</span>
              <span className="text-ink-3"> · {p.line}</span>
            </span>
          </p>
          <p className="mt-2 text-[12.5px] text-ink-4">
            {a.owner
              ? `On ${a.owner.id === me?.user.id ? "your" : `${a.owner.name}'s`} Claude subscription`
              : "Its owner is gone"}
            {" · "}
            {a.check_ins === 1 ? "1 check-in" : `${a.check_ins} check-ins`}
            {a.woken_at ? `, the last ${ago(a.woken_at, now)}` : ""}
            {/* A workspace's autopilot's own session is its admins', who steer it. */}
            {a.session && (a.scope === "project" || a.can.steer) && (
              <>
                {" · "}
                <Link
                  to="/s/$id"
                  params={{ id: a.session.id }}
                  className="text-ink-3 underline-offset-2 hover:text-ink hover:underline"
                >
                  Its session #{a.session.number}
                </Link>
              </>
            )}
          </p>
        </div>
        {a.can.steer && (
          <div className="flex shrink-0 items-center gap-1.5">
            <MoveButtons a={a} />
            <Menu
              trigger={(toggle, open) => (
                <IconButton label="More" aria-expanded={open} onClick={toggle}>
                  <Ellipsis className="size-4" />
                </IconButton>
              )}
            >
              {(close) => (
                <>
                  <MenuItem
                    icon={<Settings2 />}
                    onClick={() => {
                      close();
                      setSettings(true);
                    }}
                  >
                    Change how it works…
                  </MenuItem>
                  {!mine && me && (
                    <MenuItem
                      icon={<UserRoundCheck />}
                      onClick={() => {
                        close();
                        setTaking(true);
                      }}
                    >
                      Run it on your subscription…
                    </MenuItem>
                  )}
                  {a.session && (
                    <MenuItem icon={<SquareTerminal />} link={{ href: `/s/${a.session.id}` }} onClick={close}>
                      Watch its session
                    </MenuItem>
                  )}
                  {a.state === "on" && a.session && (
                    <MenuItem
                      icon={<RefreshCw />}
                      onClick={() => {
                        close();
                        fresh.mutate();
                      }}
                    >
                      Hand off to a fresh session
                    </MenuItem>
                  )}
                  <MenuSeparator />
                  <MenuItem
                    danger
                    icon={<Power />}
                    onClick={() => {
                      close();
                      setOff(true);
                    }}
                  >
                    Take off autopilot…
                  </MenuItem>
                </>
              )}
            </Menu>
          </div>
        )}
      </div>
      {a.problem && (
        <div className="relative flex items-start gap-2 rounded-b-[15px] border-t border-amber/25 bg-amber-soft px-5 py-2.5 text-[12.5px] text-ink-2">
          <CircleAlert className="mt-0.5 size-3.5 shrink-0 text-amber" aria-hidden="true" />
          {a.problem}
        </div>
      )}
      <OffDialog a={a} project={project} open={off} onClose={() => setOff(false)} />
      <SettingsDialog a={a} project={project} open={settings} onClose={() => setSettings(false)} />
      <TakeOverDialog a={a} project={project} open={taking} onClose={() => setTaking(false)} />
    </section>
  );
}

/**
 * What people do with it from where it stands: wake it (in a fresh session too) or pause it while it's on,
 * resume it once paused.
 */
export function useMoves(a: G.Autopilot) {
  const me = useMe().data;
  const mine = !!me && a.owner?.id === me.user.id;
  const wake = useMutation({
    mutationFn: () => api.wakeAutopilot(a.project_id),
    onSuccess: (x) => {
      patched(x);
      toast("It wakes now");
    },
    onError: (e) => failed(e, "It didn't wake"),
  });
  // Its session at work finishes its turn first.
  const fresh = useMutation({
    mutationFn: () => api.wakeAutopilot(a.project_id, { fresh: true }),
    onSuccess: (x) => {
      patched(x);
      toast(atWork(x) ? "A fresh session takes over once this turn ends" : "A fresh session takes over now");
    },
    onError: (e) => failed(e, "No fresh session took over"),
  });
  const pause = useMutation({
    mutationFn: () => api.pauseAutopilot(a.project_id),
    onSuccess: (x) => {
      patched(x);
      toast("Paused: it wakes no more until you resume it");
    },
    onError: (e) => failed(e, "It isn't paused"),
  });
  const resume = useMutation({
    mutationFn: () => api.resumeAutopilot(a.project_id),
    onSuccess: (x) => {
      patched(x);
      toast(
        !mine && takenBy(x, me) ? "Resumed, on your subscription: it wakes now" : "Resumed: it wakes now",
      );
    },
    onError: (e) => failed(e, "It isn't resumed"),
  });
  return { wake, fresh, pause, resume, mine };
}

/** *Wake now* and *Pause* while it's on, *Resume* once paused: for those who steer it. */
export function MoveButtons({ a }: { a: G.Autopilot }) {
  const { wake, pause, resume, mine } = useMoves(a);
  if (a.state === "off") return null;
  return a.state === "on" ? (
    <>
      <Button
        size="sm"
        icon={<Zap className="size-3.5" />}
        loading={wake.isPending}
        onClick={() => wake.mutate()}
        title="Wake it now, with what's new"
      >
        Wake now
      </Button>
      <Button
        size="sm"
        icon={<Pause className="size-3.5" />}
        loading={pause.isPending}
        onClick={() => pause.mutate()}
        title="It wakes no more until resumed; its session stops"
      >
        Pause
      </Button>
    </>
  ) : (
    <Button
      size="sm"
      variant="primary"
      icon={<Play className="size-3.5" />}
      loading={resume.isPending}
      onClick={() => resume.mutate()}
      title={
        mine
          ? "It wakes at once"
          : a.keep_owner && a.owner
            ? `It wakes at once, on ${a.owner.name}'s Claude subscription, which they keep it on`
            : "It wakes at once, on your Claude subscription from now on"
      }
    >
      Resume
    </Button>
  );
}

/** Now, every `every` milliseconds, for what says how long ago or until. */
export function useNow(every: number): number {
  const [now, setNow] = useState(() => Date.now());
  useEffect(() => {
    const t = window.setInterval(() => setNow(Date.now()), every);
    return () => window.clearInterval(t);
  }, [every]);
  return now;
}

function OffDialog({
  a,
  project,
  open,
  onClose,
}: {
  a: G.Autopilot;
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const working = a.crew.filter((c) =>
    ["queued", "provisioning", "preparing", "running", "idle", "paused"].includes(c.status),
  );
  const [stop, setStop] = useState(false);
  const off = useMutation({
    mutationFn: () => api.autopilotOff(a.project_id, { stop_sessions: stop }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.autopilot(a.project_id) });
      void queryClient.invalidateQueries({ queryKey: keys.autopilot(a.project_slug) });
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      toast(`${project.name} is off autopilot`);
      onClose();
    },
    onError: (e) => failed(e, "It's still on"),
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Take ${project.name} off autopilot?`}
      description="Its own session stops, and it wakes no more. It keeps its direction, notes and log, for when you put it on again."
    >
      {working.length > 0 && (
        <label className="mt-1 flex cursor-pointer items-start gap-3 rounded-xl border border-line px-3.5 py-3">
          <input
            type="checkbox"
            checked={stop}
            onChange={(e) => setStop(e.target.checked)}
            className="mt-1"
          />
          <span className="min-w-0 text-[13px] leading-snug text-ink-2">
            Also stop the {working.length === 1 ? "session" : `${working.length} sessions`} it started that{" "}
            {working.length === 1 ? "still works" : "still work"}
            <span className="mt-0.5 block text-[12.5px] text-ink-4">
              Otherwise they carry on, and you follow them like any other.
            </span>
          </span>
        </label>
      )}
      <div className="mt-6 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Keep it on
        </Button>
        <Button variant="danger" loading={off.isPending} onClick={() => off.mutate()}>
          Take it off
        </Button>
      </div>
    </Dialog>
  );
}

/** Someone who steers it, not its owner, runs it on their own subscription from now on. */
function TakeOverDialog({
  a,
  project,
  open,
  onClose,
}: {
  a: G.Autopilot;
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const take = useMutation({
    mutationFn: () => api.putAutopilot(a.project_id, { take_over: true }),
    onSuccess: (x) => {
      patched(x);
      toast("It runs on your subscription now");
      onClose();
    },
    onError: (e) => failed(e, "It isn't yours"),
  });
  const where =
    a.scope === "workspace" ? ` in every project of ${project.workspace?.name ?? "the workspace"}` : "";
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Run it on your subscription?"
      description={`From now on it runs on your Claude subscription and acts as you${where}: its own sessions and those it starts.${a.owner ? ` ${a.owner.name} no longer runs it${a.keep_owner ? ", and whether it stays on yours when others change it is then yours to choose" : ""}.` : ""}`}
    >
      <div className="mt-6 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" loading={take.isPending} onClick={() => take.mutate()}>
          Run it on mine
        </Button>
      </div>
    </Dialog>
  );
}

function SettingsDialog({
  a,
  project,
  open,
  onClose,
}: {
  a: G.Autopilot;
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const me = useMe().data;
  const [powers, setPowers] = useState(a.powers);
  const [checkIn, setCheckIn] = useState(a.check_in_minutes);
  const [crew, setCrew] = useState(a.crew_max);
  const [model, setModel] = useState(a.model ?? "");
  const [effort, setEffort] = useState(a.effort ?? "");
  const [handoff, setHandoff] = useState(handoffChoice(a.handoff_at));
  const [handoffTyped, setHandoffTyped] = useState(typedSize(a.handoff_at));
  const [keep, setKeep] = useState(a.keep_owner);
  // The fields start from the autopilot as the dialog opens, and only then: the tab refetches it
  // every few seconds, and its session's context changes with each step, which mustn't undo what
  // someone is choosing or typing.
  const latest = useRef(a);
  latest.current = a;
  useEffect(() => {
    if (!open) return;
    const a = latest.current;
    setPowers(a.powers);
    setCheckIn(a.check_in_minutes);
    setCrew(a.crew_max);
    setModel(a.model ?? "");
    setEffort(a.effort ?? "");
    setHandoff(handoffChoice(a.handoff_at));
    setHandoffTyped(typedSize(a.handoff_at));
    setKeep(a.keep_owner);
  }, [open]);
  const mine = !!me && a.owner?.id === me.user.id;
  /** Where a fresh session takes over, as chosen: `null` for only after 12 hours, `undefined` for a size not read. */
  const handoffAt =
    handoff === ""
      ? null
      : handoff === "other"
        ? (() => {
            const t = parseTokens(handoffTyped);
            return t != null && validWindow(t) ? t : undefined;
          })()
        : Number(handoff);
  const costs =
    checkIn !== a.check_in_minutes ||
    crew !== a.crew_max ||
    model !== (a.model ?? "") ||
    effort !== (a.effort ?? "") ||
    (handoffAt !== undefined && handoffAt !== (a.handoff_at ?? null));
  const save = useMutation({
    mutationFn: () => {
      if (handoffAt === undefined)
        throw new ApiFailure(400, {
          code: "validation",
          field: "handoff_at",
          message: `type a size from ${tokensText(MIN_TOKENS)} to ${tokensText(MAX_TOKENS)} tokens, like 200K`,
        });
      return api.putAutopilot(a.project_id, {
        powers,
        check_in_minutes: checkIn,
        crew_max: crew,
        model: model || "default",
        effort: effort || "default",
        // 0: only after 12 hours.
        handoff_at: handoffAt ?? 0,
        // Its owner's choice alone.
        ...(mine ? { keep_owner: keep } : {}),
      });
    },
    onSuccess: (x) => {
      patched(x);
      // Whose it is now, as the control plane decided: its owner may keep it, while they can run it.
      toast(!mine && takenBy(x, me) ? "Changed: it runs on your subscription from now on" : "Changed");
      onClose();
    },
    onError: (e) => failed(e, "Not changed"),
  });
  return (
    <Dialog open={open} onClose={onClose} title="How it works" wide>
      <div className="flex flex-col gap-6">
        <PowersField scope={a.scope} powers={powers} onChange={setPowers} />
        <div className="grid gap-5 sm:grid-cols-2">
          <RhythmFields checkIn={checkIn} setCheckIn={setCheckIn} crew={crew} setCrew={setCrew} />
        </div>
        <ModelFields
          project={project}
          model={model}
          setModel={setModel}
          effort={effort}
          setEffort={setEffort}
        />
        <HandoffField
          choice={handoff}
          setChoice={setHandoff}
          typed={handoffTyped}
          setTyped={setHandoffTyped}
          context={a.session?.context_tokens}
        />
        {mine && <KeepField a={a} keep={keep} setKeep={setKeep} />}
        {!mine && costs && a.owner && (
          <Banner tone="info">
            {a.keep_owner
              ? `It runs on ${a.owner.name}'s Claude subscription, which they keep it on when others change it: your changes run on theirs.`
              : `It runs on ${a.owner.name}'s Claude subscription. Changing how much it works runs it on yours from now on.`}
          </Banner>
        )}
      </div>
      <div className="mt-6 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" loading={save.isPending} onClick={() => save.mutate()}>
          Save
        </Button>
      </div>
    </Dialog>
  );
}

/** A size as it is typed, for a size of people's own: "" for none or one offered. */
function typedSize(tokens?: number | null): string {
  return tokens != null && handoffChoice(tokens) === "other" ? tokensText(tokens) : "";
}

/** Where a fresh session of its own takes over from the one it works in. */
function HandoffField({
  choice,
  setChoice,
  typed,
  setTyped,
  context,
}: {
  choice: HandoffChoice;
  setChoice: (c: HandoffChoice) => void;
  typed: string;
  setTyped: (t: string) => void;
  /** Its session's context now, in tokens. */
  context?: number | null;
}) {
  const options: SelectOption<HandoffChoice>[] = [
    { value: "", label: "After 12 hours", description: "Whatever its context holds" },
    ...HANDOFF_CHOICES.map((t) => ({
      value: `${t}` as HandoffChoice,
      label: `At ${tokensText(t)} tokens of context`,
      short: `At ${tokensText(t)} tokens`,
      description: "Or after 12 hours",
    })),
    { value: "other", label: "At another size", description: "Or after 12 hours" },
  ];
  return (
    <div>
      <span className="mb-1.5 block text-[12.5px] font-medium text-ink-2">A fresh session takes over</span>
      <div className="grid gap-2 sm:grid-cols-2">
        <Select
          aria-label="When a fresh session takes over"
          value={choice}
          options={options}
          onChange={setChoice}
        />
        {choice === "other" && (
          <Input
            aria-label="Tokens of context"
            value={typed}
            onChange={(e) => setTyped(e.target.value)}
            placeholder="200K"
            autoComplete="off"
            autoCapitalize="off"
            autoCorrect="off"
            spellCheck={false}
          />
        )}
      </div>
      <p className="mt-1.5 text-[12.5px] text-ink-4">
        Each wake reads its whole conversation again, uncached after an hour&rsquo;s rest. A fresh session
        starts from its plan and log, and costs far less at each wake.
        {context != null && ` Its session holds ${num(context)} tokens now.`}
      </p>
    </div>
  );
}

/** Its owner's choice: whether it stays on their subscription when others change or resume it. */
function KeepField({ a, keep, setKeep }: { a: G.Autopilot; keep: boolean; setKeep: (k: boolean) => void }) {
  const who = a.scope === "workspace" ? "the workspace's admins" : "the project's maintainers and owners";
  return (
    <fieldset>
      <legend className="mb-2 text-[12.5px] font-medium text-ink-2">When others change it</legend>
      <label
        htmlFor="autopilot-keep"
        className="flex cursor-pointer items-start gap-3 rounded-xl border border-line px-3.5 py-2.5 hover:bg-hover/30"
      >
        <input
          id="autopilot-keep"
          type="checkbox"
          checked={keep}
          onChange={(e) => setKeep(e.target.checked)}
          className="mt-1 accent-[var(--c-ink)]"
          aria-describedby="autopilot-keep-line"
        />
        <span className="min-w-0">
          <span className="block text-[13.5px] text-ink">Keep it on my subscription</span>
          <span id="autopilot-keep-line" className="mt-0.5 block text-[12.5px] leading-snug text-ink-3">
            Those who steer it, {who}, then change how much it works and resume it on your Claude plan, rather
            than taking it over, and it goes on acting as you. Only for a project whose people you all trust.
          </span>
        </span>
      </label>
      {keep && !a.keep_owner && (
        <Banner tone="warn" className="mt-2">
          Any of them can then run it as hard as they choose on your plan: up to {CREW_MAX} sessions at once,
          checking in every 15 minutes.
        </Banner>
      )}
    </fieldset>
  );
}

/** Whether `a` now runs on the subscription of `me`. */
function takenBy(a: G.Autopilot, me: G.Me | null | undefined): boolean {
  return !!me && a.owner?.id === me.user.id;
}

// ───────────────────────────── what it asks ─────────────────────────────

function AskCard({ a, ask }: { a: G.Autopilot; ask: G.AutopilotAsk }) {
  const secret = ask.kind === "secret" && ask.secret;
  return (
    <article
      aria-label={secret ? `It needs the secret ${ask.secret?.name}` : "It asks you"}
      className="animate-rise rounded-2xl border border-blue/30 bg-surface bg-[linear-gradient(to_bottom,var(--c-blue-soft),transparent_70%)] p-4"
    >
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full bg-blue-soft text-blue">
          {secret ? <KeyRound className="size-3.5" /> : <MessageCircleQuestion className="size-4" />}
        </span>
        <div className="min-w-0 flex-1">
          <p className="flex flex-wrap items-baseline gap-x-2 text-[12px] font-medium text-blue">
            {secret ? "It needs a secret" : "It asks you"}
            <span className="font-normal text-ink-4" title={stamp(ask.created_at)}>
              {agoShort(ask.created_at)}
            </span>
          </p>
          {secret && ask.secret && (
            <p className="mt-1.5 flex flex-wrap items-center gap-2">
              <code className="rounded-md bg-sunken px-2 py-0.5 font-mono text-[13px] text-ink shadow-[inset_0_0_0_1px_var(--c-line)]">
                {ask.secret.name}
              </code>
              {ask.secret.project_slug !== a.project_slug && (
                <span className="text-[12.5px] text-ink-3">for {ask.secret.project_slug}</span>
              )}
            </p>
          )}
          <p className="mt-1.5 text-[15px] leading-relaxed whitespace-pre-line text-ink [overflow-wrap:anywhere]">
            {ask.text}
          </p>
          {ask.detail && <AskDetail text={ask.detail} />}
          {a.can.steer ? (
            secret && ask.secret ? (
              <SecretAnswer a={a} ask={ask} secret={ask.secret} />
            ) : (
              <QuestionAnswer a={a} ask={ask} />
            )
          ) : (
            <p className="mt-2.5 text-[12px] text-ink-4">
              {a.scope === "workspace"
                ? "The workspace's admins answer it."
                : "The project's maintainers and owners answer it."}
            </p>
          )}
        </div>
      </div>
    </article>
  );
}

/** What helps people answer: Markdown, with screenshots of the options. Long ones fold. */
function AskDetail({ text }: { text: string }) {
  const long = text.length > 600 || text.split("\n").length > 12;
  const [open, setOpen] = useState(!long);
  return (
    <div className="mt-3 rounded-xl border border-line bg-canvas/40 px-3.5 py-3">
      <div className={cn("relative text-[13.5px]", !open && "max-h-48 overflow-hidden")}>
        <Markdown text={text} className="prose-note" />
        {!open && (
          <div className="pointer-events-none absolute inset-x-0 bottom-0 h-16 bg-gradient-to-t from-surface to-transparent" />
        )}
      </div>
      {long && (
        <button
          type="button"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="mt-2 inline-flex items-center gap-1 text-[12.5px] text-ink-3 hover:text-ink"
        >
          <ChevronDown className={cn("size-3.5 transition-transform duration-200", open && "rotate-180")} />
          {open ? "Show less" : "Show all"}
        </button>
      )}
    </div>
  );
}

function QuestionAnswer({ a, ask }: { a: G.Autopilot; ask: G.AutopilotAsk }) {
  const [text, setText] = useState("");
  const [sending, setSending] = useState<string>();
  const [declining, setDeclining] = useState(false);
  const answer = useMutation({
    mutationFn: (value: string) => api.answerAutopilot(a.project_id, ask.id, { answer: value }),
    onSuccess: (x) => {
      patched(x);
      toast("Answered: it hears now");
    },
    onError: (e) => failed(e, "Not sent"),
    onSettled: () => setSending(undefined),
  });
  const send = (value: string) => {
    if (sending || !value.trim()) return;
    setSending(value);
    answer.mutate(value.trim());
  };
  const options = ask.options ?? [];
  return (
    <div className="mt-3">
      {options.length > 0 && (
        <div className="flex flex-wrap gap-2">
          {options.map((o) => (
            <button
              key={o}
              type="button"
              disabled={!!sending}
              onClick={() => send(o)}
              className={cn(
                "inline-flex min-h-8 max-w-full items-center gap-2 rounded-full border px-3.5 py-1 text-left text-[13px] transition-[background-color,border-color,color,transform] duration-150 active:scale-[0.97]",
                sending === o
                  ? "border-blue/50 bg-blue-soft text-ink"
                  : "border-line-strong bg-raised text-ink-2 hover:border-blue/45 hover:bg-blue-soft hover:text-ink disabled:opacity-60",
              )}
            >
              {sending === o && <Spinner className="size-3" />}
              <span className="[overflow-wrap:anywhere]">{o}</span>
            </button>
          ))}
        </div>
      )}
      <form
        className="mt-2.5 flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          send(text);
        }}
      >
        <Textarea
          value={text}
          onChange={(e) => setText(e.target.value)}
          minRows={1}
          maxRows={8}
          aria-label="Your answer"
          placeholder={options.length ? "Or answer in your own words" : "Your answer"}
          className="text-[13.5px]"
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              send(text);
            }
          }}
        />
        <Button
          type="submit"
          size="field"
          disabled={!text.trim() || !!sending}
          loading={sending === text.trim() && !!text.trim()}
        >
          <Send className="size-3.5" />
          <span className="sr-only">Send</span>
        </Button>
      </form>
      <div className="mt-2 flex items-center gap-3 text-[12px] text-ink-4">
        <span>It goes on with what it can meanwhile, and hears your answer at once.</span>
        <button
          type="button"
          onClick={() => setDeclining(true)}
          className="ml-auto shrink-0 text-ink-3 hover:text-ink"
        >
          Decline…
        </button>
      </div>
      <DeclineDialog a={a} ask={ask} open={declining} onClose={() => setDeclining(false)} />
    </div>
  );
}

function SecretAnswer({ a, ask, secret }: { a: G.Autopilot; ask: G.AutopilotAsk; secret: G.AskedSecret }) {
  const [value, setValue] = useState("");
  const [declining, setDeclining] = useState(false);
  const give = useMutation({
    mutationFn: () => api.answerAutopilot(a.project_id, ask.id, { secret: value }),
    onSuccess: (x) => {
      setValue("");
      patched(x);
      void queryClient.invalidateQueries({ queryKey: keys.secrets(secret.project_id) });
      toast(`${secret.name} is set: it hears it's there, never what it is`);
    },
    onError: (e) => failed(e, "Not set"),
  });
  if (!a.can.secrets)
    return (
      <p className="mt-2.5 text-[12px] text-ink-4">
        Those who may set {secret.project_slug}'s secrets give it: its maintainers and owners.
      </p>
    );
  return (
    <div className="mt-3">
      <form
        className="flex flex-col gap-2 sm:flex-row sm:items-start"
        onSubmit={(e) => {
          e.preventDefault();
          if (value) give.mutate();
        }}
      >
        <SecretInput
          value={value}
          onChange={(e) => setValue(e.target.value)}
          aria-label={`The value of ${secret.name}`}
          placeholder={`The value of ${secret.name}`}
          className="font-mono text-[13px]"
        />
        <Button
          type="submit"
          size="field"
          variant="primary"
          disabled={!value}
          loading={give.isPending}
          icon={<ShieldCheck className="size-3.5" />}
        >
          Save secret
        </Button>
      </form>
      <p className="mt-2 text-[12px] leading-snug text-ink-4">
        It goes into {secret.project_slug}'s secrets, encrypted
        {secret.exists ? `, in place of the ${secret.name} it has` : ""}. The autopilot hears only that it's
        set; sessions that start from now on have it as ${secret.name}.
        <button type="button" onClick={() => setDeclining(true)} className="ml-2 text-ink-3 hover:text-ink">
          Decline…
        </button>
      </p>
      <DeclineDialog a={a} ask={ask} open={declining} onClose={() => setDeclining(false)} />
    </div>
  );
}

function DeclineDialog({
  a,
  ask,
  open,
  onClose,
}: {
  a: G.Autopilot;
  ask: G.AutopilotAsk;
  open: boolean;
  onClose: () => void;
}) {
  const [reason, setReason] = useState("");
  const decline = useMutation({
    mutationFn: () => api.declineAutopilot(a.project_id, ask.id, { reason: reason.trim() || undefined }),
    onSuccess: (x) => {
      patched(x);
      toast("Declined: it hears now");
      onClose();
    },
    onError: (e) => failed(e, "Not declined"),
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Decline what it asks?"
      description="It hears at once, with why if you say, and finds another way or leaves that part."
    >
      <Textarea
        value={reason}
        onChange={(e) => setReason(e.target.value)}
        minRows={2}
        maxRows={6}
        placeholder="Why, for the autopilot (optional)"
        aria-label="Why"
      />
      <div className="mt-5 flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="primary" loading={decline.isPending} onClick={() => decline.mutate()}>
          Decline
        </Button>
      </div>
    </Dialog>
  );
}

// ───────────────────────────── steering ─────────────────────────────

/** Notes people give it: it hears each at once, and keeps it until someone removes it. */
function Steer({ a }: { a: G.Autopilot }) {
  const [text, setText] = useState("");
  const add = useMutation({
    mutationFn: () => api.addAutopilotNote(a.project_id, text.trim()),
    onSuccess: (x) => {
      setText("");
      patched(x);
      toast(x.state === "on" ? "It hears your note now" : "Noted: it hears it once resumed");
    },
    onError: (e) => failed(e, "Not added"),
  });
  const notes = a.notes;
  return (
    <section aria-labelledby="steer-heading" className="rounded-2xl border border-line bg-surface">
      <form
        className="px-4 pt-3.5 pb-3"
        onSubmit={(e) => {
          e.preventDefault();
          if (text.trim()) add.mutate();
        }}
      >
        <h2 id="steer-heading" className="mb-2 flex items-center gap-2 text-[13px] font-medium text-ink-2">
          <NotebookPen className="size-3.5 text-ink-4" aria-hidden="true" />
          Steer it
        </h2>
        <Textarea
          bare
          value={text}
          onChange={(e) => setText(e.target.value)}
          minRows={2}
          maxRows={10}
          aria-label="A note for the autopilot"
          placeholder="A priority, a constraint, context it should know: “Focus on the pipeline board this week”"
          className="text-[14px] leading-relaxed"
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && text.trim()) {
              e.preventDefault();
              add.mutate();
            }
          }}
        />
        <div className="mt-2 flex items-center justify-between gap-3">
          <p className="text-[12px] text-ink-4">
            {a.state === "on"
              ? "It hears it at once, and keeps it until you remove it. A newer note wins."
              : "Paused, it hears it once resumed."}
          </p>
          <Button type="submit" size="sm" variant="primary" disabled={!text.trim()} loading={add.isPending}>
            Add note
          </Button>
        </div>
      </form>
      {notes.length > 0 && (
        <ul className="border-t border-line">
          {notes.map((n) => (
            <NoteRow key={n.id} a={a} note={n} />
          ))}
        </ul>
      )}
    </section>
  );
}

function NoteRow({ a, note }: { a: G.Autopilot; note: G.AutopilotNote }) {
  const remove = useMutation({
    mutationFn: () => api.removeAutopilotNote(a.project_id, note.id),
    onSuccess: (x) => {
      patched(x);
      toast("Removed: it no longer goes by it");
    },
    onError: (e) => failed(e, "Not removed"),
  });
  return (
    <li className="group flex items-start gap-3 border-b border-line px-4 py-3 last:border-b-0">
      {note.author ? (
        <Avatar user={note.author} size={22} />
      ) : (
        <span className="size-[22px] rounded-full bg-raised" />
      )}
      <div className="min-w-0 flex-1">
        <p className="flex flex-wrap items-center gap-x-2 text-[12px] text-ink-4">
          <span className="font-medium text-ink-2">{note.author?.name ?? "Someone"}</span>
          <span title={stamp(note.created_at)}>{agoShort(note.created_at)}</span>
          {note.seen_at ? (
            <span className="inline-flex items-center gap-1" title={`It got it ${stamp(note.seen_at)}`}>
              <Check className="size-3" aria-hidden="true" />
              Seen
            </span>
          ) : (
            <span className="text-blue">Not seen yet</span>
          )}
        </p>
        <div className="mt-0.5 text-[13.5px] leading-relaxed text-ink">
          <Markdown text={note.text} className="prose-note" />
        </div>
      </div>
      <IconButton
        label="Remove the note"
        onClick={() => remove.mutate()}
        disabled={remove.isPending}
        className="size-7 opacity-0 group-hover:opacity-100 focus-visible:opacity-100 [@media(hover:none)]:opacity-100"
      >
        {remove.isPending ? <Spinner className="size-3" /> : <X className="size-3.5" />}
      </IconButton>
    </li>
  );
}

// ───────────────────────────── its direction, plan, sessions ─────────────────────────────

function Card({
  title,
  icon,
  action,
  children,
  footer,
}: {
  title: string;
  icon: ReactNode;
  action?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
}) {
  return (
    <section className="rounded-2xl border border-line bg-surface">
      <div className="flex items-center gap-2 px-4 pt-3.5">
        <span aria-hidden="true" className="text-ink-4 [&_svg]:size-3.5">
          {icon}
        </span>
        <h2 className="flex-1 text-[13px] font-medium text-ink-2">{title}</h2>
        {action}
      </div>
      <div className="px-4 pt-2 pb-3.5">{children}</div>
      {footer && <div className="border-t border-line px-4 py-2.5 text-[12px] text-ink-4">{footer}</div>}
    </section>
  );
}

/**
 * Long text, clamped to `lines` with the rest a click away. Where it's cut it fades out, whatever
 * it's on: a card's surface, or a session's details rail.
 */
export function Clamped({
  text,
  lines = 8,
  className,
}: {
  text: string;
  lines?: number;
  className?: string;
}) {
  const long = text.split("\n").length > lines || text.length > lines * 70;
  const [open, setOpen] = useState(false);
  return (
    <div>
      <div
        className={cn(
          "relative text-[13px] leading-relaxed text-ink-2",
          className,
          long &&
            !open &&
            "max-h-44 overflow-hidden [mask-image:linear-gradient(to_bottom,#000_calc(100%-3rem),transparent)]",
        )}
      >
        <Markdown text={text} className="prose-note" />
      </div>
      {long && (
        <button
          type="button"
          onClick={() => setOpen(!open)}
          aria-expanded={open}
          className="mt-1.5 inline-flex items-center gap-1 text-[12px] text-ink-3 hover:text-ink"
        >
          <ChevronDown className={cn("size-3.5 transition-transform duration-200", open && "rotate-180")} />
          {open ? "Show less" : "Show all"}
        </button>
      )}
    </div>
  );
}

function Direction({ a }: { a: G.Autopilot }) {
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(a.direction);
  const save = useMutation({
    mutationFn: () => api.putAutopilot(a.project_id, { direction: text }),
    onSuccess: (x) => {
      patched(x);
      setEditing(false);
      toast(x.state === "on" ? "It heads the new way now" : "Changed: it goes by it once resumed");
    },
    onError: (e) => failed(e, "Not changed"),
  });
  return (
    <Card
      title="Direction"
      icon={<Waypoints />}
      action={
        a.can.steer &&
        !editing && (
          <IconButton
            label="Change its direction"
            className="size-7"
            onClick={() => {
              setText(a.direction);
              setEditing(true);
            }}
          >
            <Pencil className="size-3.5" />
          </IconButton>
        )
      }
      footer={
        !editing && (
          <>
            {a.direction_by ? `Set by ${a.direction_by.name}, ` : ""}
            <span title={stamp(a.direction_at)}>{agoShortWords(a.direction_at)}</span>
          </>
        )
      }
    >
      {editing ? (
        <form
          onSubmit={(e) => {
            e.preventDefault();
            if (text.trim()) save.mutate();
          }}
        >
          <Textarea
            value={text}
            onChange={(e) => setText(e.target.value)}
            minRows={6}
            maxRows={18}
            autoFocus
            aria-label="Its direction"
            className="text-[13px]"
          />
          <div className="mt-2 flex justify-end gap-1.5">
            <Button type="button" size="sm" variant="ghost" onClick={() => setEditing(false)}>
              Cancel
            </Button>
            <Button
              type="submit"
              size="sm"
              variant="primary"
              disabled={!text.trim()}
              loading={save.isPending}
            >
              Save
            </Button>
          </div>
        </form>
      ) : (
        <Clamped text={a.direction} />
      )}
    </Card>
  );
}

function Plan({ a }: { a: G.Autopilot }) {
  const done = useMemo(() => planDone(a.plan), [a.plan]);
  return (
    <Card
      title="Its plan"
      icon={<ListChecks />}
      action={
        done && <span className="text-[12px] text-ink-4 tabular-nums">{`${done.checked} of ${done.of}`}</span>
      }
      footer={a.plan_at && <span title={stamp(a.plan_at)}>Updated {agoShortWords(a.plan_at)}</span>}
    >
      {done && (
        <div className="mb-2.5 h-1 overflow-hidden rounded-full bg-sunken">
          <div
            className="h-full rounded-full bg-accent transition-[width] duration-500 ease-[var(--ease-out)]"
            style={{ width: `${(done.checked / done.of) * 100}%` }}
          />
        </div>
      )}
      {a.plan ? (
        <Clamped text={a.plan} lines={12} />
      ) : (
        <p className="text-[13px] leading-relaxed text-ink-4">
          It writes one as it starts: what it will do, in order. It keeps it current as the work goes.
        </p>
      )}
    </Card>
  );
}

const WORKING = new Set<G.SessionStatus>([
  "queued",
  "provisioning",
  "preparing",
  "running",
  "paused",
  "stopping",
]);

function Crew({ a }: { a: G.Autopilot }) {
  const count = crewCount(a);
  return (
    <Card
      title="Its sessions"
      icon={<GitFork />}
      action={count && <span className="text-[12px] text-ink-4 tabular-nums">{count}</span>}
    >
      <CrewList a={a} />
    </Card>
  );
}

/** "3 working · 39 in all", once it started any. */
export function crewCount(a: G.Autopilot): string | undefined {
  if (a.crew_total === 0) return;
  return `${a.crew.filter((c) => WORKING.has(c.status)).length} working · ${a.crew_total} in all`;
}

/**
 * The sessions it started: five rows in all, those at work or waiting for a message first (all of
 * them when there are more), then the rest of the 20 newest a click away.
 */
export function CrewList({ a, className }: { a: G.Autopilot; className?: string }) {
  const [all, setAll] = useState(false);
  const working = a.crew.filter((c) => WORKING.has(c.status) || c.status === "idle");
  const rest = a.crew.filter((c) => !working.includes(c));
  const shown = all ? a.crew : [...working, ...rest.slice(0, Math.max(0, 5 - working.length))];
  const others = a.crew.length - shown.length;
  return (
    <>
      {a.crew.length === 0 ? (
        <p className={cn("text-[13px] leading-relaxed text-ink-4", className)}>
          The sessions it starts show here, live: up to {a.crew_max} at once.
        </p>
      ) : (
        <ul className="-mx-2 flex flex-col">
          {shown.map((c) => (
            <li key={c.id}>
              <CrewRow c={c} home={a.project_slug} />
            </li>
          ))}
        </ul>
      )}
      {others > 0 && (
        <button
          type="button"
          onClick={() => setAll(true)}
          className="mt-1.5 inline-flex items-center gap-1 text-[12px] text-ink-3 hover:text-ink"
        >
          <ChevronDown className="size-3.5" />
          Show {others} more
        </button>
      )}
    </>
  );
}

/** One of its sessions: its status, number and title, what it asks, and its pull requests' states. */
export function CrewRow({ c, home }: { c: G.AutopilotCrew; home?: string }) {
  return (
    <Link
      to="/s/$id"
      params={{ id: c.id }}
      className="group flex items-start gap-2.5 rounded-lg px-2 py-1.5 transition-colors hover:bg-hover"
      title={`${statusLabel(c.status)}: ${c.title}`}
    >
      <StatusDot status={c.status} className="mt-[6px]" />
      <span className="min-w-0 flex-1">
        <span className="flex items-baseline gap-1.5">
          <span className="shrink-0 font-mono text-[11.5px] text-ink-4">
            {c.project_slug !== home ? c.project_slug : ""}#{c.number}
          </span>
          <span className="min-w-0 flex-1 truncate text-[13px] text-ink-2 group-hover:text-ink">
            {c.title}
          </span>
        </span>
        {c.question ? (
          <span className="mt-0.5 flex items-center gap-1 text-[12px] text-blue">
            <MessageCircleQuestion className="size-3 shrink-0" aria-hidden="true" />
            <span className="truncate">{c.question}</span>
          </span>
        ) : (
          (c.pull_requests ?? []).length > 0 && (
            <span className="mt-0.5 block truncate text-[11.5px] text-ink-4">
              {(c.pull_requests ?? []).map((p) => `#${p.number} ${p.state ?? "open"}`).join(" · ")}
            </span>
          )
        )}
      </span>
    </Link>
  );
}

function Rules({ a, project }: { a: G.Autopilot; project: G.Project }) {
  return (
    <Card title="How it works" icon={<Settings2 />}>
      <ul className="flex flex-col gap-1.5 text-[12.5px] leading-snug text-ink-3">
        <li className="flex gap-2">
          <Clock className="mt-[2px] size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
          Checks in {checkInWords(a.check_in_minutes)}, and runs up to{" "}
          {a.crew_max === 1 ? "1 session" : `${a.crew_max} sessions`} at once
        </li>
        <li className="flex gap-2">
          <UserRoundCheck className="mt-[2px] size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
          Starts and steers sessions,{" "}
          {a.powers.includes("merge") ? "which merge their pull requests" : "which leave merging to people"}
          {a.powers.some((p) => p !== "merge") &&
            `; ${a.powers
              .filter((p) => p !== "merge")
              .map((p) => POWERS.find((x) => x.power === p)?.label.toLowerCase())
              .filter(Boolean)
              .join(", ")}`}
        </li>
        {a.scope === "workspace" && (
          <li className="flex gap-2">
            <FolderPlus className="mt-[2px] size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
            Works in every project of {project.workspace?.name ?? "its workspace"}
          </li>
        )}
        {a.keep_owner && <KeptLine a={a} />}
        <li className="flex gap-2">
          <Sparkles className="mt-[2px] size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
          Thinks with {modelLabel(a.model ?? project.settings.model ?? "")}
          {(a.effort ?? project.settings.effort)
            ? ` at ${effortLabel(a.effort ?? project.settings.effort ?? "").toLowerCase()} effort`
            : ""}
        </li>
        <li className="flex gap-2">
          <Repeat className="mt-[2px] size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
          <span>
            A fresh session takes over {handoffWords(a.handoff_at)}
            {a.session?.context_tokens != null && (
              <span className="text-ink-4"> · its session holds {num(a.session.context_tokens)}</span>
            )}
          </span>
        </li>
      </ul>
    </Card>
  );
}

/** That its owner keeps it on their subscription when others change it. */
function KeptLine({ a }: { a: G.Autopilot }) {
  const me = useMe().data;
  const whose = !a.owner ? "its owner's" : me && a.owner.id === me.user.id ? "your" : `${a.owner.name}'s`;
  return (
    <li className="flex gap-2">
      <UsersRound className="mt-[2px] size-3.5 shrink-0 text-ink-4" aria-hidden="true" />
      Stays on {whose} subscription when others change or resume it
    </li>
  );
}

// ───────────────────────────── its log ─────────────────────────────

const ENTRY_ICONS: Record<G.AutopilotEntryKind, ReactNode> = {
  note: <NotebookPen />,
  woke: <AutopilotIcon />,
  session: <GitFork />,
  steered: <Send />,
  settings: <Settings2 />,
  secret: <KeyRound />,
  repository: <BookOpenCheck />,
  schedule: <Repeat />,
  project: <FolderPlus />,
  plan: <ListChecks />,
  asked: <MessageCircleQuestion />,
  answered: <Check />,
  direction: <Waypoints />,
  state: <Power />,
  problem: <CircleAlert />,
  other: <Undo2 />,
};

function Log({ a }: { a: G.Autopilot }) {
  const working = atWork(a);
  const q = useInfiniteQuery({
    queryKey: keys.autopilotLog(a.project_id),
    queryFn: ({ pageParam }) => api.autopilotLog(a.project_id, pageParam, 40),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => (last.has_more ? last.entries[last.entries.length - 1]?.id : undefined),
    refetchInterval: a.state === "on" ? (working ? 5_000 : 20_000) : false,
  });
  const entries = useMemo(() => q.data?.pages.flatMap((p) => p.entries) ?? [], [q.data]);
  const days = useMemo(() => byDay(entries), [entries]);
  const crew = useMemo(() => new Map(a.crew.map((c) => [c.id, c])), [a.crew]);
  return (
    <section aria-labelledby="log-heading">
      <h2 id="log-heading" className="mb-3 flex items-center gap-2 text-[14px] font-semibold text-ink">
        Log
        <span className="text-[12px] font-normal text-ink-4">What it did, and what happened to it</span>
      </h2>
      {q.isPending ? (
        <div className="flex flex-col gap-2">
          <Skeleton className="h-10 w-full" />
          <Skeleton className="h-10 w-4/5" />
        </div>
      ) : entries.length === 0 ? (
        <p className="rounded-2xl border border-dashed border-line px-4 py-8 text-center text-[13px] text-ink-4">
          What it does shows here as it does it: the sessions it starts, what it sets up, and what it writes
          down.
        </p>
      ) : (
        <div className="flex flex-col gap-5">
          {days.map(([label, list]) => (
            <div key={label}>
              <h3 className="mb-1.5 text-[11.5px] font-medium text-ink-4">{label}</h3>
              <ol className="relative flex flex-col">
                {list.map((e) => (
                  <EntryRow key={e.id} e={e} crew={crew} home={a.project_slug} />
                ))}
              </ol>
            </div>
          ))}
          {q.hasNextPage && (
            <Button
              size="sm"
              variant="ghost"
              loading={q.isFetchingNextPage}
              onClick={() => void q.fetchNextPage()}
              className="self-start"
            >
              Show earlier
            </Button>
          )}
        </div>
      )}
    </section>
  );
}

/** Entries grouped by the day they happened, newest first: "Today", "Yesterday", "Nov 3". */
function byDay(entries: G.AutopilotEntry[]): [string, G.AutopilotEntry[]][] {
  const out: [string, G.AutopilotEntry[]][] = [];
  const today = new Date().setHours(0, 0, 0, 0);
  for (const e of entries) {
    const d = new Date(e.at).setHours(0, 0, 0, 0);
    const label = d === today ? "Today" : d === today - 86_400_000 ? "Yesterday" : day(e.at);
    const last = out[out.length - 1];
    if (last && last[0] === label) last[1].push(e);
    else out.push([label, [e]]);
  }
  return out;
}

function EntryRow({
  e,
  crew,
  home,
}: {
  e: G.AutopilotEntry;
  crew: Map<string, G.AutopilotCrew>;
  home: string;
}) {
  // Its wakes divide what it did from one check-in to the next.
  if (e.kind === "woke")
    return (
      <li className="flex items-center gap-2.5 py-1.5 pl-[3px] text-[12px] text-ink-4">
        <span className="flex size-[18px] shrink-0 items-center justify-center [&_svg]:size-3.5">
          <AutopilotIcon />
        </span>
        <span className="h-px w-3 bg-line" aria-hidden="true" />
        <span className="min-w-0 truncate">{e.text}</span>
        {e.session && (
          <Link to="/s/$id" params={{ id: e.session.id }} className="shrink-0 text-ink-3 hover:text-ink">
            #{e.session.number}
          </Link>
        )}
        <span className="h-px flex-1 bg-line" aria-hidden="true" />
        <time title={stamp(e.at)} className="shrink-0 tabular-nums">
          {clock(e.at)}
        </time>
      </li>
    );
  const person = !!e.by;
  const note = e.kind === "note";
  const problem = e.kind === "problem";
  const live = e.session ? crew.get(e.session.id) : undefined;
  return (
    <li className="group flex gap-3 py-2">
      <span
        aria-hidden="true"
        className={cn(
          "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full [&_svg]:size-3.5",
          problem
            ? "bg-danger-soft text-danger"
            : person
              ? "bg-raised text-ink-3 shadow-[inset_0_0_0_1px_var(--c-line)]"
              : note
                ? "bg-accent-soft text-ink"
                : "bg-raised text-ink-2 shadow-[inset_0_0_0_1px_var(--c-line)]",
        )}
      >
        {person && e.by ? <Avatar user={e.by} size={24} /> : ENTRY_ICONS[e.kind]}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex items-start gap-3">
          <div
            className={cn("min-w-0 flex-1 text-[13.5px] leading-relaxed", note ? "text-ink" : "text-ink-2")}
          >
            <div className="[overflow-wrap:anywhere]">
              {e.schedule ? (
                // fctl's own line, with the schedule's name a link to it.
                <ScheduleNamed
                  text={e.text}
                  schedule={e.schedule}
                  slug={e.project_slug ?? home}
                  className="decoration-line-strong hover:text-ink hover:decoration-ink-3"
                />
              ) : (
                <Markdown text={e.text} className={cn("prose-note", !note && "[&>*:last-child]:inline")} />
              )}
            </div>
          </div>
          <time title={stamp(e.at)} className="mt-0.5 shrink-0 text-[11.5px] text-ink-4 tabular-nums">
            {clock(e.at)}
          </time>
        </div>
        {e.session && (
          <Link
            to="/s/$id"
            params={{ id: e.session.id }}
            className="mt-1.5 inline-flex max-w-full items-center gap-2 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-[12.5px] transition-colors hover:border-line-strong hover:bg-hover/40"
          >
            {live ? (
              <StatusDot status={live.status} />
            ) : (
              <GitFork className="size-3 text-ink-4" aria-hidden="true" />
            )}
            <span className="shrink-0 font-mono text-[11.5px] text-ink-4">
              {e.project_slug && e.project_slug !== home ? e.project_slug : ""}#{e.session.number}
            </span>
            <span className="min-w-0 truncate text-ink-2">{e.session.title}</span>
            {live && <span className="shrink-0 text-[11.5px] text-ink-4">{statusLabel(live.status)}</span>}
          </Link>
        )}
        {!note && !e.session && e.project_slug && e.project_slug !== home && (
          <ProjectCard slug={e.project_slug} />
        )}
      </div>
    </li>
  );
}

/** Another project an entry concerns (one it created, or changed), as the sidebar shows it. */
function ProjectCard({ slug }: { slug: string }) {
  const p = useProjects().data?.projects.find((x) => x.slug === slug);
  return (
    <Link
      to="/p/$slug"
      params={{ slug }}
      className="mt-1.5 inline-flex max-w-full items-center gap-2 rounded-lg border border-line bg-surface px-2.5 py-1.5 text-[12.5px] transition-colors hover:border-line-strong hover:bg-hover/40"
    >
      {p ? (
        <ProjectMark project={p} size={14} />
      ) : (
        <FolderPlus className="size-3 text-ink-4" aria-hidden="true" />
      )}
      <span className="min-w-0 truncate text-ink-2">{p?.name ?? slug}</span>
      {p && <span className="shrink-0 font-mono text-[11.5px] text-ink-4">{slug}</span>}
    </Link>
  );
}

export { wakeWords };

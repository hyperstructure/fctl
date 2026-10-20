// Home: every agent at work as a thread of light, and what to do next.

import { type FoundryHandle, type FoundryLane, foundry, LANES } from "@hyperstructure/design/gl/foundry";
import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Check, Cpu, FolderPlus, KeyRound, Plus, ShieldCheck, Users } from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { PlanUsageStrip } from "@/components/plan-usage";
import {
  AttachButton,
  DraftTray,
  DropHint,
  useAttachments,
  useFileDrop,
} from "@/components/session/attachments";
import { SessionRow } from "@/components/session/row";
import { StartButton, useStartSession } from "@/components/session/start";
import {
  Button,
  firstNames,
  GithubIcon,
  GLCanvas,
  Select,
  Skeleton,
  Textarea,
  TextareaControls,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { NoRunnerNote } from "@/components/workspace/no-runners";
import type * as G from "@/gen";
import { workspaces as wsApi } from "@/lib/api";
import { filesFromPaste } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { forgetDraft, loadText, saveText } from "@/lib/draft";
import { feed, useFeed } from "@/lib/feed";
import { ago, lowOnDisk } from "@/lib/format";
import { keys, queryClient, useMe, useProjects, useRunners, useSessions, useWorkspaces } from "@/lib/queries";
import { repoSummary, repoTitle } from "@/lib/repos";
import { waitsForYou } from "@/lib/status";
import { useTaskBoxFocus } from "@/lib/task-box";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

const ACTIVE: G.SessionStatus[] = [
  "queued",
  "provisioning",
  "preparing",
  "running",
  "idle",
  "paused",
  "stopping",
];

function greeting(user: G.User) {
  const h = new Date().getHours();
  const part = h < 5 ? "Working late" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
  return `${part}, ${firstNames(user)}.`;
}

/** Palette index in the scene: 0 neutral, 1 working, 2 waiting, 3 paused, 4 failed. */
function laneColor(s: G.SessionStatus): number {
  if (s === "running") return 1;
  if (s === "idle") return 2;
  if (s === "paused") return 3;
  if (s === "failed") return 4;
  return 0;
}

/** Spreads n sessions over the lanes, evenly. */
function lanesFor(n: number): number[] {
  return Array.from({ length: n }, (_, i) => Math.round(((i + 0.5) * LANES) / Math.max(n, 1) - 0.5));
}

/** The sessions at work as threads of light, in the foundry scene. */
function Threads({ active, handle }: { active: G.Session[]; handle: RefObject<FoundryHandle | null> }) {
  const energy = useRef(new Map<string, number>());
  // Keyed on ids and statuses: the list itself is a new array on every render.
  const key = [...active]
    .sort((a, b) => a.created_at - b.created_at)
    .slice(0, LANES)
    .map((s) => `${s.id}:${s.status}`)
    .join(",");
  const lanes = useMemo(() => {
    const entries = key ? key.split(",").map((e) => e.split(":") as [string, G.SessionStatus]) : [];
    const ls = lanesFor(entries.length);
    return new Map(entries.map(([id, status], i) => [id, { lane: ls[i] ?? 0, status }]));
  }, [key]);

  useEffect(() => {
    const apply = () => {
      const m = new Map<number, FoundryLane>();
      for (const [id, l] of lanes) {
        const e = energy.current.get(id) ?? 0;
        m.set(l.lane, {
          lit: ["running", "idle", "paused", "failed"].includes(l.status) ? 1 : 0.55,
          color: laneColor(l.status),
          energy: Math.min(1, e),
        });
      }
      handle.current?.setLanes(m);
    };
    apply();
    const off = feed.onPulse((p) => {
      const l = lanes.get(p.sessionId);
      if (!l) return;
      if (p.kind !== "delta") handle.current?.pulse(l.lane, p.weight);
      energy.current.set(
        p.sessionId,
        Math.min(1.5, (energy.current.get(p.sessionId) ?? 0) + Math.min(0.5, p.weight / 400)),
      );
    });
    const decay = setInterval(() => {
      for (const [k, v] of energy.current) energy.current.set(k, v * 0.82);
      apply();
    }, 500);
    return () => {
      off();
      clearInterval(decay);
    };
  }, [lanes, handle]);

  return (
    <div className="absolute inset-0">
      <GLCanvas
        scene={() => {
          const h = foundry({ planet: [0.95, -0.85, 1.4], density: 0.5 });
          handle.current = h;
          return h.scene;
        }}
        options={{ pixel: 3 }}
      />
    </div>
  );
}

/**
 * A project's repositories in a line. Where space is short the name gives way and the count
 * stays: the grid's narrowest is the count alone, its widest the whole line.
 */
function RepoText({ project }: { project: G.Project }) {
  const s = repoSummary(project);
  if (!s) return "No repository";
  return (
    <span className="grid grid-cols-[minmax(0,max-content)_auto] font-mono text-[11.5px]">
      <span className="truncate">{s.name}</span>
      <span className="whitespace-pre">{s.name && s.more ? ` ${s.more}` : s.more}</span>
    </span>
  );
}

/** The project a new session goes to: picked here last time, while it is still there. */
const LAST_TARGET = "fctl:last-target";

function QuickStart({
  user,
  projects,
  workspaces,
}: {
  user: string;
  projects: G.Project[];
  workspaces: G.Workspace[];
}) {
  const index = useMemo(() => new Map(workspaces.map((w) => [w.id, w])), [workspaces]);
  // Projects where you may start sessions, under their workspaces, in your order (the list's).
  const editable = useMemo(
    () =>
      projects.filter((p) => p.role !== "viewer" && (!p.workspace || !index.get(p.workspace.id)?.blocked)),
    [projects, index],
  );
  // What was being written here before, kept in this browser until a session starts with it.
  const [kept] = useState(() => loadText(user));
  const remembered = (() => {
    try {
      return localStorage.getItem(LAST_TARGET) ?? undefined;
    } catch {
      return undefined;
    }
  })();
  const here = (slug?: string) => editable.find((p) => p.slug === slug)?.slug;
  const first = here(kept?.project) ?? here(remembered) ?? editable[0]?.slug;
  const [project, setProject] = useState(first ?? "");
  const [prompt, setPrompt] = useState(kept?.prompt ?? "");
  const ref = useRef<HTMLTextAreaElement>(null);
  useTaskBoxFocus(ref, editable.length > 0);
  useEffect(() => {
    if ((!project || !editable.some((p) => p.slug === project)) && first) setProject(first);
  }, [first, project, editable]);
  const pick = (slug: string) => {
    setProject(slug);
    try {
      localStorage.setItem(LAST_TARGET, slug);
    } catch {}
  };
  const sent = useRef(false);
  const { start, starting } = useStartSession(() => {
    // The draft went out with the session.
    sent.current = true;
    return forgetDraft(user);
  });
  const files = useAttachments(editable.length && project ? { project } : undefined, { keep: user });
  const attach = (f: File[], pasted?: boolean) => {
    files.add(f, { pasted });
    ref.current?.focus();
  };
  // The draft is kept as it changes, and on the way out with the caret where it is then.
  const caret = useRef(kept?.selection);
  const save = useRef(() => {});
  save.current = () => {
    if (sent.current) return;
    if (ref.current) caret.current = [ref.current.selectionStart, ref.current.selectionEnd];
    const empty = !prompt.trim() && !files.drafts.length;
    saveText(user, empty ? undefined : { prompt, project, selection: caret.current });
  };
  // biome-ignore lint/correctness/useExhaustiveDependencies: saves when the draft changes
  useEffect(() => save.current(), [prompt, project, files.drafts.length]);
  useEffect(() => () => save.current(), []);
  useLayoutEffect(() => {
    const s = kept?.selection;
    if (s) ref.current?.setSelectionRange(s[0], s[1]);
  }, [kept]);
  const dragging = useFileDrop(attach, editable.length > 0 && !starting);
  if (!editable.length) return null;
  const target = editable.find((p) => p.slug === project);
  const ws = target?.workspace ? freshest(index, target.workspace) : undefined;
  const several = workspaces.length > 1;
  return (
    <>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          start(project, prompt, files);
        }}
        className={cn(
          "group relative rounded-2xl border border-line-strong bg-surface/85 shadow-[0_24px_80px_rgb(0_0_0/0.35)] backdrop-blur-2xl transition-[border-color,box-shadow] duration-200 focus-within:border-accent/45 focus-within:shadow-[0_24px_80px_rgb(0_0_0/0.35),0_0_0_4px_var(--c-accent-soft)]",
          dragging && "border-accent/50",
        )}
      >
        <DraftTray
          drafts={files.drafts}
          onRemove={files.remove}
          onRetry={files.retry}
          className="px-4 pt-4"
        />
        <Textarea
          ref={ref}
          bare
          minRows={2}
          maxRows={8}
          value={prompt}
          readOnly={starting}
          onChange={(e) => setPrompt(e.target.value)}
          onBlur={() => save.current()}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              start(project, prompt, files);
            }
          }}
          onPaste={(e) => {
            const pasted = filesFromPaste(e.clipboardData);
            if (!pasted.length || starting) return;
            e.preventDefault();
            attach(pasted, true);
          }}
          placeholder="What should an agent build next?"
          className="block px-5 pt-4 font-display text-[18px] leading-[1.5] text-ink placeholder:text-ink-4"
          aria-label="Task for a new session"
        />
        <TextareaControls textarea={ref} className="gap-2 pt-4 pr-3 pb-3 pl-2.5">
          <AttachButton onFiles={(f) => attach(f)} />
          <div className="flex min-w-0 items-center gap-2 text-[12.5px]">
            <span className="hidden text-ink-4 sm:inline">in</span>
            <Select
              size="sm"
              aria-label="Workspace and project"
              value={project}
              disabled={starting}
              onChange={pick}
              options={editable.map((p) => {
                const w = p.workspace ? freshest(index, p.workspace) : undefined;
                return {
                  value: p.slug,
                  label: several && w ? `${w.name} / ${p.name}` : p.name,
                  item: p.name,
                  icon: w ? <WorkspaceAvatar workspace={w} size={14} title={false} /> : undefined,
                  description: <RepoText project={p} />,
                  section:
                    several && w
                      ? {
                          key: w.id,
                          label: (
                            <>
                              <WorkspaceAvatar workspace={w} size={14} title={false} />
                              {w.name}
                            </>
                          ),
                        }
                      : undefined,
                };
              })}
            />
            {target && repoSummary(target) && (
              <span
                className="hidden shrink-[100] items-center gap-1.5 font-mono text-[11.5px] text-ink-4 md:inline-flex"
                title={repoTitle(target, ws?.name)}
              >
                <GithubIcon className="size-3 shrink-0" />
                <RepoText project={target} />
              </span>
            )}
          </div>
          <span className="flex-1" />
          <span className="hidden text-[11.5px] text-ink-4 sm:inline">⌘↵ to start</span>
          <StartButton
            starting={starting}
            disabled={!prompt.trim() && !files.drafts.length}
            className="enabled:hover:scale-[1.03]"
          >
            Start
          </StartButton>
        </TextareaControls>
        <DropHint show={dragging} />
      </form>
      <NoRunnerNote workspace={target?.workspace && index.get(target.workspace.id)} className="mt-3" />
    </>
  );
}

function Step({
  done,
  title,
  children,
  action,
}: {
  done: boolean;
  title: string;
  children: ReactNode;
  action?: ReactNode;
}) {
  return (
    <li className="flex gap-4 py-4">
      <span
        className={cn(
          "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border transition-colors",
          done ? "border-highlight bg-highlight text-highlight-ink" : "border-line-strong text-ink-4",
        )}
      >
        {done ? <Check className="size-3.5" /> : <span className="size-1.5 rounded-full bg-ink-4" />}
      </span>
      <div className="min-w-0 flex-1">
        <div
          className={cn(
            "text-[14.5px] font-medium",
            done ? "text-ink-3 line-through decoration-ink-4" : "text-ink",
          )}
        >
          {title}
        </div>
        {!done && <div className="mt-1 text-[13.5px] leading-relaxed text-ink-3">{children}</div>}
      </div>
      {!done && action}
    </li>
  );
}

function GettingStarted({
  me,
  runners,
  projects,
  workspaces,
}: {
  me: G.Me;
  runners: G.Runner[];
  projects: G.Project[];
  workspaces: G.Workspace[];
}) {
  const workspace = workspaces.length > 0;
  const claude = me.claude.some((c) => c.status === "active");
  const runner = runners.some((r) => r.status === "online");
  const project = projects.length > 0;
  if (workspace && claude && runner && project) return null;
  // The first workspace you run, for the steps that happen in one.
  const yours = workspaces.find((w) => w.role === "owner" || w.role === "admin");
  return (
    <section className="animate-rise">
      <h2 className="font-display text-[22px] tracking-[-0.01em] text-ink">Set up your factory</h2>
      <p className="mt-1 text-[14px] text-ink-3">A few steps, and agents start working for you.</p>
      <ol className="mt-3 divide-y divide-line border-y border-line">
        <Step
          done={workspace}
          title="Join or create a workspace"
          action={
            <Link to="/w/new">
              <Button size="sm" icon={<Plus className="size-3.5" />}>
                Create
              </Button>
            </Link>
          }
        >
          The people you work with, your projects, the machines that run them and the GitHub accounts they
          work in. Or join one you are invited to.
        </Step>
        <Step
          done={claude}
          title="Connect your Claude subscription"
          action={
            <Link to="/settings" search={{ linked: undefined, error: undefined }}>
              <Button size="sm" icon={<KeyRound className="size-3.5" />}>
                Connect
              </Button>
            </Link>
          }
        >
          Sessions run on your own Claude Pro or Max plan — never on API billing.
        </Step>
        <Step
          done={runner}
          title="Bring a machine online"
          action={
            yours ? (
              <Link to="/w/$slug" params={{ slug: yours.slug }} search={{ tab: "runners" }}>
                <Button size="sm" icon={<Cpu className="size-3.5" />}>
                  Add a runner
                </Button>
              </Link>
            ) : undefined
          }
        >
          {yours
            ? "Install fctl on the machine that runs agents (your spare Mac is perfect) and register it for your workspace."
            : "A workspace admin connects a machine that runs its agents."}
        </Step>
        <Step
          done={project}
          title="Create a project"
          action={
            workspace ? (
              <Link to="/new">
                <Button size="sm" icon={<FolderPlus className="size-3.5" />}>
                  Create
                </Button>
              </Link>
            ) : undefined
          }
        >
          Link a GitHub repository — or start a brand new one — and tell agents how to set it up.
        </Step>
      </ol>
    </section>
  );
}

/** Invitations to workspaces that wait for you: join or decline them here. */
function Invitations({ invitations }: { invitations: G.WorkspaceInvitation[] }) {
  const done = () => {
    for (const k of [keys.workspaces, keys.projects, ["sessions"], keys.runners, keys.me])
      void queryClient.invalidateQueries({ queryKey: k });
  };
  const accept = useMutation({
    mutationFn: (i: G.WorkspaceInvitation) => wsApi.accept(i.id),
    onSuccess: (w) => {
      done();
      toast(`You joined ${w.name}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const decline = useMutation({
    mutationFn: (i: G.WorkspaceInvitation) => wsApi.decline(i.id),
    onSuccess: done,
    onError: (e) => toast(e.message, "error"),
  });
  if (!invitations.length) return null;
  return (
    <section className="flex animate-rise flex-col gap-2">
      {invitations.map((i) => (
        <div
          key={i.id}
          className="flex flex-wrap items-center gap-4 rounded-xl border border-accent/25 bg-accent-soft px-4 py-3.5"
        >
          <WorkspaceAvatar workspace={i.workspace} size={36} />
          <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
            <div className="text-[14.5px] text-ink">
              {i.invited_by ? `${i.invited_by.name} invited you to ` : "You're invited to "}
              <span className="font-medium">{i.workspace.name}</span>
            </div>
            <div className="mt-0.5 text-[12.5px] text-ink-3">
              As {i.email}, {i.role === "admin" ? "an admin" : "a member"} · {ago(i.created_at)}
              {i.needs_two_factor && (
                <span className="text-amber">
                  {" "}
                  · it requires two-factor sign-in:{" "}
                  <Link
                    to="/settings"
                    search={{ linked: undefined, error: undefined }}
                    className="underline underline-offset-2"
                  >
                    set it up
                  </Link>{" "}
                  to join
                </span>
              )}
            </div>
          </div>
          {/* On a phone they take a line of their own, where a long name gives way. */}
          <div className="flex max-w-full min-w-0 gap-2">
            <Button
              size="sm"
              variant="ghost"
              loading={decline.isPending && decline.variables?.id === i.id}
              onClick={() => decline.mutate(i)}
            >
              Decline
            </Button>
            <Button
              size="sm"
              variant="primary"
              disabled={i.needs_two_factor}
              loading={accept.isPending && accept.variables?.id === i.id}
              onClick={() => accept.mutate(i)}
              className="min-w-0"
            >
              <span className="truncate">Join {i.workspace.name}</span>
            </Button>
          </div>
        </div>
      ))}
    </section>
  );
}

/** Narrows the home page's lists to one workspace, with each one's work in progress. */
function WorkspaceFilter({
  workspaces,
  active,
  value,
  onChange,
  className,
}: {
  workspaces: G.Workspace[];
  active: G.Session[];
  value: string | undefined;
  onChange: (id: string | undefined) => void;
  className?: string;
}) {
  if (workspaces.length < 2) return null;
  const count = (id?: string) => active.filter((s) => !id || s.workspace?.id === id).length;
  const chip = (id: string | undefined, label: ReactNode, n: number) => (
    <button
      key={id ?? "all"}
      type="button"
      aria-pressed={value === id}
      onClick={() => onChange(id)}
      className={cn(
        "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-[12.5px] transition-colors duration-150",
        value === id
          ? "border-line-strong bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "border-transparent text-ink-3 hover:bg-hover hover:text-ink",
      )}
    >
      {label}
      {n > 0 && <span className="text-[11px] text-ink-4 tabular-nums">{n}</span>}
    </button>
  );
  return (
    <fieldset
      className={cn("-mx-6 flex gap-1 overflow-x-auto px-6 [scrollbar-width:none]", className)}
      aria-label="Show the work of"
    >
      {chip(undefined, "All workspaces", count())}
      {workspaces
        .filter((w) => !w.blocked)
        .map((w) =>
          chip(
            w.id,
            <>
              <WorkspaceAvatar workspace={w} size={14} title={false} />
              {w.name}
            </>,
            count(w.id),
          ),
        )}
    </fieldset>
  );
}

/** Someone who is in no workspace yet: what to do first. */
function NoWorkspace({ invited }: { invited: boolean }) {
  return (
    <section className="flex animate-rise flex-col items-start gap-4 rounded-2xl border border-line bg-surface/70 p-6">
      <span className="flex size-10 items-center justify-center rounded-xl bg-raised text-ink-3 shadow-[inset_0_0_0_1px_var(--c-line)]">
        <Users className="size-5" />
      </span>
      <div>
        <h2 className="font-display text-[22px] tracking-[-0.01em] text-ink">
          {invited ? "Join a workspace to start" : "You aren't in a workspace yet"}
        </h2>
        <p className="mt-1 max-w-xl text-[14px] leading-relaxed text-ink-3">
          Projects, the machines that run their agents and the GitHub accounts they work in all belong to
          workspaces.{" "}
          {invited
            ? "Join the one you are invited to above, or create your own."
            : "Create one for your own projects, or ask someone to invite you to theirs."}
        </p>
      </div>
      <Link to="/w/new">
        <Button variant="primary" icon={<Plus className="size-3.5" />}>
          Create a workspace
        </Button>
      </Link>
    </section>
  );
}

export function Home() {
  const { data: me } = useMe();
  const projects = useProjects();
  const runners = useRunners();
  const workspaces = useWorkspaces();
  const index = useWorkspaceIndex();
  const active = useSessions({ status: "active", limit: 50 });
  const recent = useSessions({ status: "ended", limit: 12 });
  const f = useFeed();
  const [only, setOnly] = useState<string>();
  const mine = workspaces.data?.workspaces ?? [];
  const several = mine.length > 1;
  const activeAll = (active.data?.sessions ?? []).filter((s) => ACTIVE.includes(s.status));
  const inFilter = (s: G.Session) => !only || s.workspace?.id === only;
  const activeList = activeAll.filter(inFilter);
  const recentList = (recent.data?.sessions ?? []).filter(inFilter);
  // A session that waits for its agent's background work is at work too.
  const working = activeAll.filter(
    (s) => s.status === "running" || (s.status === "idle" && !waitsForYou(s)),
  ).length;
  const waiting = activeAll.filter(waitsForYou).length;
  const onlineRunners = (f.runners.length ? f.runners : (runners.data?.runners ?? [])).filter(
    (r) => r.status === "online",
  );
  const online = onlineRunners.length;
  const draining = onlineRunners.filter((r) => r.drain).length;
  const lowDisk = onlineRunners.filter((r) => !r.drain && lowOnDisk(r)).length;
  const scene = useRef<FoundryHandle | null>(null);
  // The plan that sessions you start run on.
  const usage = me?.claude.find((c) => c.is_default)?.usage;
  const blocked = mine.filter((w) => w.blocked);
  const row = (s: G.Session) => (
    <SessionRow key={s.id} s={s} workspace={several && !only ? freshest(index, s.workspace) : undefined} />
  );

  return (
    <div>
      <div
        // Shorter on a phone, so that the composer and the sessions start higher up.
        className="relative h-[360px] overflow-hidden border-b border-line max-sm:h-[272px]"
        onPointerMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          scene.current?.pointer((e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height);
        }}
        onPointerLeave={() => scene.current?.pointerLeave()}
      >
        <Threads active={activeAll} handle={scene} />
        <div className="pointer-events-none absolute inset-0 bg-gradient-to-b from-transparent via-transparent to-canvas" />
        <div className="relative z-[1] mx-auto flex h-full max-w-[920px] flex-col justify-end px-6 pb-10">
          <h1 className="animate-rise font-display text-[40px] leading-[1.08] font-medium tracking-[-0.022em] text-ink">
            {me ? greeting(me.user) : " "}
          </h1>
          <p className="mt-3 animate-rise text-[15px] text-ink-2 [animation-delay:80ms]">
            {working > 0 ? (
              <>
                <span className="text-accent">
                  {working} {working === 1 ? "agent is" : "agents are"} working
                </span>
                {waiting > 0 && <> · {waiting} waiting for you</>}
              </>
            ) : waiting > 0 ? (
              <>
                {waiting} {waiting === 1 ? "session is" : "sessions are"} waiting for you
              </>
            ) : (
              "All quiet. Start something."
            )}
            <span className="text-ink-4">
              {several && ` · in ${mine.length} workspaces`} · {online} {online === 1 ? "runner" : "runners"}{" "}
              online
              {draining > 0 && `, ${draining} draining`}
              {lowDisk > 0 && `, ${lowDisk} low on disk`}
            </span>
          </p>
        </div>
      </div>
      <div className="mx-auto max-w-[920px] px-6">
        <div className="-mt-6 relative z-[2]">
          {me && projects.data && workspaces.data && (
            <QuickStart
              user={me.user.id}
              projects={projects.data.projects}
              workspaces={workspaces.data.workspaces}
            />
          )}
          {usage?.length ? <PlanUsageStrip usage={usage} className="mt-3" /> : null}
        </div>
        <div className="mt-12 flex flex-col gap-12 pb-24">
          {workspaces.data && <Invitations invitations={workspaces.data.invitations} />}
          {blocked.map((w) => (
            <div
              key={w.id}
              className="flex items-center gap-3 rounded-xl border border-amber/25 bg-amber-soft px-4 py-3 text-[13.5px] text-ink-2"
            >
              <WorkspaceAvatar workspace={w} size={24} />
              <span className="min-w-0 flex-1 [overflow-wrap:anywhere]">
                {w.name} requires two-factor sign-in. Until you set it up, its projects and sessions are
                closed to you.
              </span>
              <Link to="/settings" search={{ linked: undefined, error: undefined }}>
                <Button size="sm" icon={<ShieldCheck className="size-3.5" />}>
                  Set it up
                </Button>
              </Link>
            </div>
          ))}
          {workspaces.data && mine.length === 0 && (
            <NoWorkspace invited={workspaces.data.invitations.length > 0} />
          )}
          {me && projects.data && runners.data && workspaces.data && mine.length > 0 && (
            <GettingStarted
              me={me}
              runners={f.runners.length ? f.runners : runners.data.runners}
              projects={projects.data.projects}
              workspaces={mine}
            />
          )}
          {mine.length > 0 && (
            <>
              <section>
                {/* Up into the gap above, clear of the list below: it narrows Recently too. */}
                <WorkspaceFilter
                  workspaces={mine}
                  active={activeAll}
                  value={only}
                  onChange={setOnly}
                  className="-mt-6 mb-9"
                />
                <div className="mb-2 flex items-baseline justify-between">
                  <h2 className="text-[13px] font-semibold text-ink">Working now</h2>
                  <span className="text-[12px] text-ink-4">{activeList.length} active</span>
                </div>
                {active.isPending ? (
                  <div className="space-y-3 py-3">
                    <Skeleton className="h-10" />
                    <Skeleton className="h-10" />
                  </div>
                ) : activeList.length === 0 ? (
                  <p className="border-y border-line py-6 text-[13.5px] text-ink-4">
                    {only
                      ? `No agents are working in ${index.get(only)?.name ?? "this workspace"}.`
                      : "No agents are working. Give one a task above."}
                  </p>
                ) : (
                  <div className="border-t border-line">{activeList.map(row)}</div>
                )}
              </section>
              {recentList.length > 0 && (
                <section>
                  <h2 className="mb-2 text-[13px] font-semibold text-ink">Recently</h2>
                  <div className="border-t border-line">{recentList.map(row)}</div>
                </section>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

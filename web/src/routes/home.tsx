// The Agent Floor, the home page: every agent at work as a thread of light, and
// what to do next.

import { type FoundryHandle, type FoundryLane, foundry, LANES } from "@hyperstructure/design/gl/foundry";
import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Check, Cpu, FolderPlus, KeyRound, Plus, ShieldCheck, Users } from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { PlanUsageStrip } from "@/components/plan-usage";
import { RunnerApprovals } from "@/components/runner-approvals";
import { RunnerUpdate } from "@/components/runner-update";
import { SessionRow } from "@/components/session/row";
import { TaskBox } from "@/components/session/task-box";
import { Button, calledName, GLCanvas, Skeleton, toast } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { workspaces as wsApi } from "@/lib/api";
import { cn } from "@/lib/cn";
import { feed, useFeed } from "@/lib/feed";
import { ago, lowOnDisk } from "@/lib/format";
import { keys, queryClient, useMe, useProjects, useRunners, useSessions, useWorkspaces } from "@/lib/queries";
import { ACTIVE } from "@/lib/sessions";
import { waitsForYou } from "@/lib/status";
import { isUltra } from "@/lib/usage";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

function greeting(user: G.User) {
  const h = new Date().getHours();
  const part = h < 5 ? "Working late" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
  return `${part}, ${calledName(user)}.`;
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
        name="Agent Floor scene"
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
            <Link to="/settings" search={{ tab: "claude" }}>
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
            ? "A runner is a machine that runs your agents' sessions (your spare Mac is perfect): install fctl on it and register it for your workspace."
            : "A workspace admin adds a runner: a machine that runs its agents' sessions."}
        </Step>
        <Step
          done={project}
          title="Create a project"
          action={
            yours ? (
              <Link to="/new">
                <Button size="sm" icon={<FolderPlus className="size-3.5" />}>
                  Create
                </Button>
              </Link>
            ) : undefined
          }
        >
          {yours
            ? "Link a GitHub repository — or start a brand new one — and tell agents how to set it up."
            : "A workspace admin creates its projects, and adds you to the ones you work in."}
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
          // Narrower than 36rem inside, as on a phone or beside the sidebar on a tablet, the
          // buttons take a line of their own under the text.
          className="@container rounded-xl border border-accent/25 bg-accent-soft px-4 py-3.5"
        >
          <div className="flex flex-col gap-3.5 @xl:flex-row @xl:items-center @xl:gap-4">
            <div className="flex min-w-0 flex-1 items-start gap-3 @xl:items-center @xl:gap-4">
              <WorkspaceAvatar workspace={i.workspace} size={36} />
              <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
                <div className="text-[14.5px] text-ink">
                  {i.invited_by ? `${i.invited_by.name} invited you to ` : "You're invited to "}
                  <span className="font-medium">{i.workspace.name}</span>
                </div>
                <div className="mt-0.5 text-[12.5px] text-ink-3">
                  As {i.email}, {i.role === "admin" ? "an admin" : "a member"} ·{" "}
                  <span className="whitespace-nowrap">{ago(i.created_at)}</span>
                  {i.needs_two_factor && (
                    <span className="text-amber">
                      {" "}
                      · it requires two-factor sign-in:{" "}
                      <Link
                        to="/settings"
                        search={{ tab: "security" }}
                        className="underline underline-offset-2"
                      >
                        set it up
                      </Link>{" "}
                      to join
                    </span>
                  )}
                </div>
              </div>
            </div>
            {/* Under the text, Join takes the room Decline leaves; beside it, a long name gives
                way before the text does. */}
            <div className="flex min-w-0 gap-2 @xl:max-w-1/2">
              <Button
                size="sm"
                variant="ghost"
                loading={decline.isPending && decline.variables?.id === i.id}
                onClick={() => decline.mutate(i)}
                className="@max-xl:h-9"
              >
                Decline
              </Button>
              <Button
                size="sm"
                variant="primary"
                disabled={i.needs_two_factor}
                loading={accept.isPending && accept.variables?.id === i.id}
                onClick={() => accept.mutate(i)}
                className="min-w-0 @max-xl:h-9 @max-xl:flex-1"
              >
                <span className="truncate">Join {i.workspace.name}</span>
              </Button>
            </div>
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
  const plan = me?.claude.find((c) => c.is_default);
  const usage = plan?.usage;
  const blocked = mine.filter((w) => w.blocked);
  const row = (s: G.Session) => (
    <SessionRow key={s.id} s={s} workspace={several && !only ? freshest(index, s.workspace) : undefined} />
  );

  return (
    <div>
      <div
        // Shorter on a phone, so that the composer and the sessions start higher up.
        className="relative h-[360px] overflow-hidden max-sm:h-[272px]"
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
            <TaskBox
              user={me.user.id}
              projects={projects.data.projects}
              workspaces={workspaces.data.workspaces}
              workspace={only}
            />
          )}
          {usage?.length ? (
            <PlanUsageStrip usage={usage} ultra={plan && isUltra(plan)} className="mt-3" />
          ) : null}
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
              <Link to="/settings" search={{ tab: "security" }}>
                <Button size="sm" icon={<ShieldCheck className="size-3.5" />}>
                  Set it up
                </Button>
              </Link>
            </div>
          ))}
          <RunnerApprovals />
          {me && <RunnerUpdate user={me.user.id} />}
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
                  <div>{activeList.map(row)}</div>
                )}
              </section>
              {recentList.length > 0 && (
                <section>
                  <h2 className="mb-2 text-[13px] font-semibold text-ink">Recently</h2>
                  <div>{recentList.map(row)}</div>
                </section>
              )}
            </>
          )}
        </div>
      </div>
    </div>
  );
}

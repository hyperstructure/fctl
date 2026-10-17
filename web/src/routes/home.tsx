// The floor: every agent at work as a thread of light, and what to do next.

import { useMutation } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import { ArrowRight, ArrowUp, Check, Cpu, FolderPlus, KeyRound } from "lucide-react";
import { type ReactNode, type RefObject, useEffect, useMemo, useRef, useState } from "react";
import { Avatar, Button, GLCanvas, Skeleton, StatusDot, Textarea, toast } from "@/components/ui";
import type * as G from "@/gen";
import { LANES, type ThreadLane, type ThreadsHandle, threads } from "@/gl/threads";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { feed, useFeed } from "@/lib/feed";
import { agoShort, stamp } from "@/lib/format";
import { useMe, useProjects, useRunners, useSessions } from "@/lib/queries";

const ACTIVE: G.SessionStatus[] = [
  "queued",
  "provisioning",
  "preparing",
  "running",
  "idle",
  "paused",
  "stopping",
];

function greeting(name: string) {
  const h = new Date().getHours();
  const part = h < 5 ? "Working late" : h < 12 ? "Good morning" : h < 18 ? "Good afternoon" : "Good evening";
  return `${part}, ${name.split(" ")[0]}.`;
}

function laneColor(s: G.SessionStatus): number {
  if (s === "running") return 1;
  if (s === "idle") return 2;
  if (s === "failed") return 4;
  return 3;
}

/** Spreads n sessions over the lanes, evenly. */
function lanesFor(n: number): number[] {
  return Array.from({ length: n }, (_, i) => Math.round(((i + 0.5) * LANES) / Math.max(n, 1) - 0.5));
}

function Floor({ active, handle }: { active: G.Session[]; handle: RefObject<ThreadsHandle | null> }) {
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
      const m = new Map<number, ThreadLane>();
      for (const [id, l] of lanes) {
        const e = energy.current.get(id) ?? 0;
        m.set(l.lane, {
          lit: l.status === "stopping" ? 0.4 : 1,
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
          const h = threads();
          handle.current = h;
          return h.scene;
        }}
        options={{ scale: 1, maxDpr: 2 }}
      />
    </div>
  );
}

function QuickStart({ projects }: { projects: G.Project[] }) {
  const navigate = useNavigate();
  const [project, setProject] = useState(projects[0]?.slug ?? "");
  const [prompt, setPrompt] = useState("");
  useEffect(() => {
    if (!project && projects[0]) setProject(projects[0].slug);
  }, [projects, project]);
  const start = useMutation({
    mutationFn: () => api.createSession(project, { prompt }),
    onSuccess: (s) => navigate({ to: "/s/$id", params: { id: s.id } }),
    onError: (e) => toast(e.message, "error"),
  });
  const editable = projects.filter((p) => p.role !== "viewer");
  if (!editable.length) return null;
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (prompt.trim()) start.mutate();
      }}
      className="group relative rounded-2xl border border-line-strong bg-surface/85 shadow-[0_24px_80px_rgb(0_0_0/0.35)] backdrop-blur-2xl transition-[border-color,box-shadow] duration-200 focus-within:border-accent/45 focus-within:shadow-[0_24px_80px_rgb(0_0_0/0.35),0_0_0_4px_var(--c-accent-soft)]"
    >
      <Textarea
        bare
        minRows={2}
        maxRows={10}
        value={prompt}
        onChange={(e) => setPrompt(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
            e.preventDefault();
            if (prompt.trim()) start.mutate();
          }
        }}
        placeholder="What should an agent build next?"
        className="block px-5 pt-4 pb-14 font-serif text-[18px] leading-[1.5] text-ink placeholder:text-ink-4"
        aria-label="Task for a new session"
      />
      <div className="absolute right-3 bottom-3 left-4 flex items-center gap-3">
        <label className="flex min-w-0 items-center gap-2 text-[12.5px] text-ink-3">
          <span className="text-ink-4">in</span>
          <select
            value={project}
            onChange={(e) => setProject(e.target.value)}
            className="min-w-0 truncate rounded-md border border-line bg-raised px-2 py-1 text-[12.5px] text-ink outline-none"
          >
            {editable.map((p) => (
              <option key={p.id} value={p.slug}>
                {p.name}
              </option>
            ))}
          </select>
        </label>
        <span className="flex-1" />
        <span className="hidden text-[11.5px] text-ink-4 sm:inline">⌘↵ to start</span>
        <button
          type="submit"
          disabled={!prompt.trim() || start.isPending}
          className={cn(
            "flex h-9 items-center gap-2 rounded-full pr-3 pl-4 text-[13.5px] font-medium transition-all duration-200 ease-[var(--ease-spring)]",
            prompt.trim() ? "bg-ink text-canvas hover:scale-[1.03]" : "bg-raised text-ink-4",
          )}
        >
          Start <ArrowUp className="size-4" />
        </button>
      </div>
    </form>
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
          done ? "border-accent bg-accent text-accent-ink" : "border-line-strong text-ink-4",
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

function GettingStarted({ me, runners, projects }: { me: G.Me; runners: G.Runner[]; projects: G.Project[] }) {
  const admin = me.user.role !== "member";
  const claude = me.claude.some((c) => c.status === "active");
  const runner = runners.some((r) => r.status === "online");
  const project = projects.length > 0;
  if (claude && runner && project) return null;
  return (
    <section className="animate-rise">
      <h2 className="font-serif text-[22px] tracking-[-0.01em] text-ink">Set up your factory</h2>
      <p className="mt-1 text-[14px] text-ink-3">Three steps, and agents start working for you.</p>
      <ol className="mt-3 divide-y divide-line border-y border-line">
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
            admin ? (
              <Link to="/admin" search={{ tab: "runners" }}>
                <Button size="sm" icon={<Cpu className="size-3.5" />}>
                  Add a runner
                </Button>
              </Link>
            ) : undefined
          }
        >
          {admin
            ? "Install fctl on the machine that runs agents (your spare Mac is perfect) and register it."
            : "An admin needs to connect a machine that runs the agents."}
        </Step>
        <Step
          done={project}
          title="Create a project"
          action={
            <Link to="/new">
              <Button size="sm" icon={<FolderPlus className="size-3.5" />}>
                Create
              </Button>
            </Link>
          }
        >
          Link a GitHub repository — or start a brand new one — and tell agents how to set it up.
        </Step>
      </ol>
    </section>
  );
}

function SessionRow({ s }: { s: G.Session }) {
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      className="group flex items-center gap-3.5 border-b border-line py-3 transition-colors last:border-b-0 hover:bg-raised/30"
    >
      <StatusDot status={s.status} className="ml-1" />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[14.5px] text-ink transition-colors group-hover:text-ink">
          {s.title}
        </div>
        <div className="mt-0.5 flex items-center gap-2 text-[12px] text-ink-4">
          <span className="font-mono">
            {s.project_slug}#{s.number}
          </span>
          {s.status_reason && s.status !== "running" && s.status !== "idle" && (
            <span className="truncate">· {s.status_reason}</span>
          )}
          {s.stats.files_changed > 0 && (
            <span>
              · <span className="text-accent/80">+{s.stats.additions}</span>{" "}
              <span className="text-danger/80">−{s.stats.deletions}</span>
            </span>
          )}
          {s.pull_request && <span>· PR #{s.pull_request.number}</span>}
        </div>
      </div>
      <Avatar user={s.created_by} size={20} />
      <time
        dateTime={new Date(s.last_activity_at ?? s.created_at).toISOString()}
        title={stamp(s.last_activity_at ?? s.created_at)}
        className="w-10 shrink-0 text-right text-[12px] whitespace-nowrap text-ink-4 tabular-nums"
      >
        {agoShort(s.last_activity_at ?? s.created_at)}
      </time>
      <ArrowRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-hover:translate-x-0.5 group-hover:opacity-100" />
    </Link>
  );
}

export function Home() {
  const { data: me } = useMe();
  const projects = useProjects();
  const runners = useRunners();
  const active = useSessions({ status: "active", limit: 50 });
  const recent = useSessions({ status: "ended", limit: 12 });
  const f = useFeed();
  const activeList = (active.data?.sessions ?? []).filter((s) => ACTIVE.includes(s.status));
  const working = activeList.filter((s) => s.status === "running").length;
  const waiting = activeList.filter((s) => s.status === "idle").length;
  const online = (f.runners.length ? f.runners : (runners.data?.runners ?? [])).filter(
    (r) => r.status === "online",
  ).length;
  const floor = useRef<ThreadsHandle | null>(null);

  return (
    <div>
      <div
        className="relative h-[360px] overflow-hidden border-b border-line"
        onPointerMove={(e) => {
          const r = e.currentTarget.getBoundingClientRect();
          floor.current?.pointer((e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height);
        }}
        onPointerLeave={() => floor.current?.pointerLeave()}
      >
        <Floor active={activeList} handle={floor} />
        <div className="pointer-events-none absolute inset-0 bg-gradient-to-b from-transparent via-transparent to-canvas" />
        <div className="relative z-[1] mx-auto flex h-full max-w-[920px] flex-col justify-end px-6 pb-10">
          <h1 className="animate-rise font-serif text-[40px] leading-[1.08] font-medium tracking-[-0.022em] text-ink">
            {me ? greeting(me.user.name) : " "}
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
              "The floor is quiet. Start something."
            )}
            <span className="text-ink-4">
              {" "}
              · {online} {online === 1 ? "runner" : "runners"} online
            </span>
          </p>
        </div>
      </div>
      <div className="mx-auto max-w-[920px] px-6">
        <div className="-mt-6 relative z-[2]">
          {projects.data && <QuickStart projects={projects.data.projects} />}
        </div>
        <div className="mt-12 flex flex-col gap-12 pb-24">
          {me && projects.data && runners.data && (
            <GettingStarted
              me={me}
              runners={f.runners.length ? f.runners : runners.data.runners}
              projects={projects.data.projects}
            />
          )}
          <section>
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
                No agents are working. Give one a task above.
              </p>
            ) : (
              <div className="border-t border-line">
                {activeList.map((s) => (
                  <SessionRow key={s.id} s={s} />
                ))}
              </div>
            )}
          </section>
          {(recent.data?.sessions.length ?? 0) > 0 && (
            <section>
              <h2 className="mb-2 text-[13px] font-semibold text-ink">Recently</h2>
              <div className="border-t border-line">
                {recent.data?.sessions.map((s) => (
                  <SessionRow key={s.id} s={s} />
                ))}
              </div>
            </section>
          )}
        </div>
      </div>
    </div>
  );
}

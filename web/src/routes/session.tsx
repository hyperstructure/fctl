// A session: the agent at work, live.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useParams } from "@tanstack/react-router";
import {
  ArrowUpRight,
  Bell,
  BellOff,
  BellRing,
  Ellipsis,
  ExternalLink,
  GitBranch,
  GitPullRequest,
  Globe,
  PanelRight,
  Play,
  Share2,
  Square,
  Terminal,
  Users,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { Composer } from "@/components/session/composer";
import { AgentOrb } from "@/components/session/orb";
import { Transcript } from "@/components/session/transcript";
import {
  Avatar,
  AvatarStack,
  Banner,
  Button,
  CopyButton,
  Dialog,
  Field,
  IconButton,
  Input,
  Menu,
  MenuItem,
  Select,
  type SelectOption,
  Skeleton,
  StatusPill,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago, duration, num, until } from "@/lib/format";
import { hasChanges, type RepoChanges, repoChanges } from "@/lib/git";
import { useLiveSession } from "@/lib/live";
import { prFullRef, prRef, prTitle, pullRequests } from "@/lib/pulls";
import { keys, queryClient, useMe, useProject } from "@/lib/queries";
import { claudeAppLink } from "@/lib/remote-control";
import { type Stall, stallLabel, stallText, useStall } from "@/lib/stall";
import { statusReason } from "@/lib/status";
import { useStickToBottom } from "@/lib/stick-to-bottom";
import { agentTime, turnsTaken } from "@/lib/work";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

export function SessionPage() {
  const { id } = useParams({ from: "/app/s/$id" });
  const { data: me } = useMe();
  const initial = useQuery({ queryKey: keys.session(id), queryFn: () => api.session(id) });
  const [s, store] = useLiveSession(id, initial.data);
  const session = s.session ?? initial.data;
  const prs = session ? pullRequests(session) : [];
  const workspaceIndex = useWorkspaceIndex();
  const stall = useStall(session, s);
  const [rail, setRail] = useState(true);
  const [share, setShare] = useState(false);
  const transcript = useRef<HTMLDivElement>(null);
  // The newest work stays in view while the reader is at the bottom.
  useStickToBottom(transcript, id);

  useEffect(() => {
    if (session) document.title = `${session.title} · ${session.project_slug}#${session.number}`;
    return () => {
      document.title = "fctl";
    };
  }, [session]);

  const resume = useMutation({
    mutationFn: (b?: { prompt?: string; attachments?: G.AttachmentId[] }) =>
      api.resume(id, b?.prompt, b?.attachments),
  });
  const stop = useMutation({
    mutationFn: () => api.stop(id),
    onSuccess: () => toast("Stopping the session"),
  });
  // Live updates go to everyone watching and leave out whether *you* follow it.
  const following = initial.data?.following ?? false;
  const follow = useMutation({
    mutationFn: () => (following ? api.unfollow(id) : api.follow(id)),
    onSuccess: async (updated) => {
      queryClient.setQueryData<G.Session>(keys.session(id), (old) =>
        old ? { ...old, ...updated } : updated,
      );
      if (!updated.following) {
        toast("You no longer get notifications about this session");
        return;
      }
      const n = await queryClient
        .fetchQuery({ queryKey: keys.notifications, queryFn: api.notifications, staleTime: 60_000 })
        .catch(() => undefined);
      toast(
        n && n.devices.length === 0
          ? "Following. Turn on notifications in Settings to get them in a browser."
          : "Following: you get a notification when it waits for you, pauses or fails",
      );
    },
    onError: (e) => toast(e.message, "error"),
  });

  if (initial.isError && !session) {
    return (
      <div className="mx-auto max-w-xl px-6 py-24">
        <Banner tone="error">This session does not exist, or you cannot see it.</Banner>
      </div>
    );
  }

  const status = session?.status;
  // The session's workspace as your list has it now (renames and avatars).
  const workspace = freshest(workspaceIndex, session?.workspace ?? initial.data?.workspace);
  const rc = session?.remote_control;
  const role = s.role ?? session?.role;
  const isOwner = role === "owner";

  return (
    <div className="flex min-h-dvh">
      <div className="flex min-w-0 flex-1 flex-col">
        {/* Header */}
        <header className="sticky top-0 z-10 border-b border-line bg-canvas/80 backdrop-blur-xl">
          <div className="flex items-center gap-4 px-6 py-3.5">
            <AgentOrb status={status} activity={s.activity} />
            <div className="min-w-0 flex-1">
              {session ? (
                <>
                  <h1 className="truncate font-display text-[21px] leading-tight font-medium tracking-[-0.012em] text-ink">
                    {session.title}
                  </h1>
                  {session.description && (
                    <p className="mt-0.5 truncate text-[13.5px] text-ink-2" title={session.description}>
                      {session.description}
                    </p>
                  )}
                  <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12.5px] text-ink-3">
                    <span className="inline-flex min-w-0 items-center gap-1.5">
                      {workspace && (
                        <>
                          <Link
                            to="/w/$slug"
                            params={{ slug: workspace.slug }}
                            className="inline-flex min-w-0 items-center gap-1.5 text-ink-2 hover:text-ink"
                          >
                            <WorkspaceAvatar workspace={workspace} size={15} />
                            <span className="max-w-[12rem] truncate">{workspace.name}</span>
                          </Link>
                          <span className="text-ink-4">/</span>
                        </>
                      )}
                      <Link
                        to="/p/$slug"
                        params={{ slug: session.project_slug }}
                        search={{ tab: undefined }}
                        className="font-mono text-ink-2 hover:text-ink"
                      >
                        {session.project_slug}#{session.number}
                      </Link>
                    </span>
                    <StatusPill status={session.status} />
                    {statusReason(session) && (
                      <span className="truncate text-ink-4">{statusReason(session)}</span>
                    )}
                    {session.resume_at && (
                      <span className="text-amber">resumes in {until(session.resume_at)}</span>
                    )}
                    {session.branch && (
                      <span className="inline-flex min-w-0 items-center gap-1 font-mono text-[12px] text-ink-3">
                        <GitBranch className="size-3 shrink-0" />
                        <span className="[overflow-wrap:anywhere]">{session.branch}</span>
                      </span>
                    )}
                    {prs.length > 0 && (
                      // Together, so that they wrap to the next line as one list.
                      <span className="inline-flex flex-wrap items-center gap-x-3 gap-y-1">
                        {prs.map((pr) => (
                          <a
                            key={pr.url}
                            href={pr.url}
                            target="_blank"
                            rel="noreferrer"
                            title={prTitle(pr)}
                            className="inline-flex items-center gap-1 text-ink-2 hover:text-ink"
                          >
                            <GitPullRequest className="size-3" />
                            {prRef(pr)}
                          </a>
                        ))}
                      </span>
                    )}
                    {s.connection === "reconnecting" && <span className="text-amber">Reconnecting…</span>}
                    {stall && <span className="text-amber">{stallLabel(stall)}</span>}
                  </div>
                </>
              ) : (
                <>
                  <Skeleton className="h-6 w-80" />
                  <Skeleton className="mt-2 h-3.5 w-56" />
                </>
              )}
            </div>
            <div className="flex shrink-0 items-center gap-1.5">
              {s.viewers.length > 1 && <AvatarStack users={s.viewers.map((v) => v.user)} />}
              {rc?.session_url && (
                <a
                  {...claudeAppLink(rc.session_url)}
                  className="hidden h-8 items-center gap-1.5 rounded-md border border-accent/30 bg-accent-soft px-3 text-[12.5px] font-medium text-accent transition-colors hover:border-accent/60 sm:inline-flex"
                >
                  Open in Claude <ArrowUpRight className="size-3.5" />
                </a>
              )}
              {session && (
                // On phones it is in the menu, where the title keeps its room.
                <span className="hidden sm:contents">
                  <IconButton
                    label={
                      following
                        ? "Following: you get its notifications. Click to stop."
                        : "Follow: get a notification when it waits for you, pauses or fails"
                    }
                    aria-pressed={following}
                    onClick={() => follow.mutate()}
                    disabled={follow.isPending}
                    className={cn(following && "bg-accent-soft")}
                  >
                    {following ? <BellRing className="size-4 text-accent" /> : <Bell className="size-4" />}
                  </IconButton>
                </span>
              )}
              {isOwner && (
                <IconButton label="Share" onClick={() => setShare(true)}>
                  <Share2 className="size-4" />
                </IconButton>
              )}
              <IconButton
                label="Details"
                onClick={() => setRail(!rail)}
                className={cn("hidden lg:inline-flex", rail && "text-ink")}
              >
                <PanelRight className="size-4" />
              </IconButton>
              <Menu
                trigger={(toggle, open) => (
                  <IconButton label="More" aria-haspopup="menu" aria-expanded={open} onClick={toggle}>
                    <Ellipsis className="size-4" />
                  </IconButton>
                )}
              >
                {(close) => (
                  <>
                    {rc?.session_url && (
                      <>
                        {/* The header has no room for the button on phones. */}
                        <MenuItem
                          icon={<ArrowUpRight />}
                          link={claudeAppLink(rc.session_url)}
                          onClick={close}
                          className="sm:hidden"
                        >
                          Open in Claude
                        </MenuItem>
                        <MenuItem
                          icon={<Globe />}
                          link={{ href: rc.session_url, target: "_blank", rel: "noreferrer" }}
                          onClick={close}
                        >
                          Open on claude.ai
                        </MenuItem>
                      </>
                    )}
                    {isOwner && status && !["stopped", "failed"].includes(status) && (
                      <MenuItem
                        icon={<Square />}
                        onClick={() => {
                          close();
                          stop.mutate();
                        }}
                      >
                        Stop the session
                      </MenuItem>
                    )}
                    {isOwner && (status === "stopped" || status === "failed") && (
                      <MenuItem
                        icon={<Play />}
                        onClick={() => {
                          close();
                          resume.mutate(undefined);
                        }}
                      >
                        Resume
                      </MenuItem>
                    )}
                    {session && (
                      <MenuItem
                        icon={following ? <BellOff /> : <Bell />}
                        onClick={() => {
                          close();
                          follow.mutate();
                        }}
                      >
                        {following ? "Stop following" : "Follow"}
                      </MenuItem>
                    )}
                    <MenuItem
                      icon={<Terminal />}
                      onClick={() => {
                        close();
                        void navigator.clipboard.writeText(`fctl attach ${id}`);
                        toast("Copied `fctl attach` command");
                      }}
                    >
                      Copy `fctl attach`
                    </MenuItem>
                    <MenuItem
                      icon={<ExternalLink />}
                      onClick={() => {
                        close();
                        void navigator.clipboard.writeText(location.href);
                        toast("Link copied");
                      }}
                    >
                      Copy link
                    </MenuItem>
                  </>
                )}
              </Menu>
            </div>
          </div>
        </header>

        {/* Transcript */}
        <div ref={transcript} className="mx-auto w-full max-w-[760px] flex-1 px-6 pt-6 pb-4">
          {s.error && (
            <Banner tone="error" className="mb-4">
              {s.error}
            </Banner>
          )}
          {!session || (s.connection === "connecting" && !s.items.length) ? (
            <div className="space-y-4 pt-4">
              <Skeleton className="h-16 w-3/4" />
              <Skeleton className="h-4 w-1/2" />
              <Skeleton className="h-24 w-full" />
            </div>
          ) : (
            <Transcript
              sessionId={id}
              items={s.items}
              live={s.live}
              status={status}
              stall={stall}
              canManage={isOwner}
              onLoadOlder={() => void store.loadOlder()}
              hasMoreBefore={s.hasMoreBefore}
              loadingOlder={s.loadingOlder}
            />
          )}
        </div>

        {/* Composer */}
        <div className="sticky bottom-0 z-10 bg-gradient-to-t from-canvas via-canvas/95 to-transparent pt-6 pb-[max(1.25rem,env(safe-area-inset-bottom))]">
          <div className="mx-auto w-full max-w-[760px] px-6">
            <Composer
              status={status}
              role={role}
              viewers={s.viewers}
              meId={me?.user.id}
              sessionId={id}
              onSend={(t, files) =>
                store.send(
                  t,
                  me?.user
                    ? { id: me.user.id, name: me.user.name, avatar_url: me.user.avatar_url }
                    : undefined,
                  files,
                )
              }
              onInterrupt={() => store.interrupt()}
              onTyping={() => store.typing()}
              onResume={async (t, files) => {
                await resume.mutateAsync({
                  prompt: t,
                  attachments: files.attachments.map((a) => a.id),
                });
                for (const url of Object.values(files.previews)) URL.revokeObjectURL(url);
              }}
            />
          </div>
        </div>
      </div>

      {rail && session && (
        <Rail
          session={session}
          git={s.git}
          viewers={s.viewers}
          model={s.model}
          runnerConnected={s.runnerConnected}
          stall={stall}
        />
      )}
      {session && <ShareDialog open={share} onClose={() => setShare(false)} session={session} />}
    </div>
  );
}

function RailSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="border-b border-line px-5 py-4 last:border-b-0">
      <h3 className="mb-2.5 text-[12.5px] font-medium text-ink-3">{title}</h3>
      {children}
    </section>
  );
}

function Row({ k, v }: { k: string; v: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-0.5 text-[12.5px]">
      <span className="text-ink-4">{k}</span>
      <span className="min-w-0 truncate text-right text-ink-2">{v}</span>
    </div>
  );
}

/**
 * The agent's time so far, counting up each second while a turn is under
 * way, up to `until` when its work stopped coming through.
 */
function AgentTime({ stats, until }: { stats: G.SessionStats; until?: number }) {
  const ticking = stats.turn_started_at != null && until == null;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [ticking]);
  return <span className="tabular-nums">{duration(agentTime(stats, now, until))}</span>;
}

/** A repository's changed files and totals. */
function Changes({
  repo,
  title,
  limit,
  showTotals,
}: {
  repo: RepoChanges;
  /** The repository's name, when the session has more than one. */
  title?: string;
  limit: number;
  showTotals: boolean;
}) {
  const { files, ahead } = repo;
  const maxChange = Math.max(1, ...files.map((f) => (f.additions ?? 0) + (f.deletions ?? 0)));
  return (
    <div>
      {title && (
        <div className="mb-1.5 flex items-baseline justify-between gap-2">
          <span className="min-w-0 truncate font-mono text-[12px] text-ink-2" title={title}>
            {title}
          </span>
          {repo.readOnly && <span className="shrink-0 text-[11.5px] text-ink-4">read only</span>}
        </div>
      )}
      {files.length === 0 ? (
        <p className="text-[12.5px] text-ink-4">No changes yet.</p>
      ) : (
        <ul className="flex flex-col gap-1">
          {files.slice(0, limit).map((f) => {
            const a = f.additions ?? 0;
            const d = f.deletions ?? 0;
            return (
              <li key={f.path} className="flex items-center gap-2 text-[12px]">
                <span
                  className={cn(
                    "w-3 shrink-0 text-center font-mono text-[11px]",
                    f.status === "added" || f.status === "untracked"
                      ? "text-accent"
                      : f.status === "deleted"
                        ? "text-danger"
                        : "text-blue",
                  )}
                >
                  {f.status === "added" || f.status === "untracked"
                    ? "A"
                    : f.status === "deleted"
                      ? "D"
                      : f.status === "renamed"
                        ? "R"
                        : "M"}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-ink-2" title={f.path}>
                  {f.path}
                </span>
                <span className="flex h-1.5 w-12 shrink-0 overflow-hidden rounded-full bg-raised">
                  <span className="bg-accent/80" style={{ width: `${(a / maxChange) * 100}%` }} />
                  <span className="bg-danger/70" style={{ width: `${(d / maxChange) * 100}%` }} />
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {showTotals && (
        <div className="mt-3 flex flex-wrap gap-x-3 text-[12px] text-ink-4">
          <span>
            <span className="text-accent">+{repo.additions}</span>{" "}
            <span className="text-danger">−{repo.deletions}</span>
          </span>
          {ahead > 0 && (
            <span>
              {ahead} {ahead === 1 ? "commit" : "commits"}
            </span>
          )}
          <span>
            {repo.pushed
              ? "pushed"
              : ahead > 0
                ? repo.readOnly
                  ? "kept in the sandbox"
                  : "not pushed yet"
                : ""}
          </span>
        </div>
      )}
    </div>
  );
}

/** Every pull request of the session, with the repository of each. */
function PullRequests({ prs }: { prs: G.PullRequestRef[] }) {
  return (
    <ul className="flex flex-col gap-1.5">
      {prs.map((pr) => (
        <li key={pr.url}>
          <a
            href={pr.url}
            target="_blank"
            rel="noreferrer"
            title={prTitle(pr)}
            className="flex items-center gap-2 rounded-md border border-line px-3 py-2 text-[12.5px] text-ink-2 transition-colors hover:border-line-strong hover:text-ink"
          >
            <GitPullRequest className="size-3.5 shrink-0 text-accent" />
            <span className="min-w-0 flex-1">
              <span className="block truncate">{pr.title ?? `Pull request #${pr.number}`}</span>
              <span className="block truncate font-mono text-[11.5px] text-ink-4">{prFullRef(pr)}</span>
            </span>
            <ArrowUpRight className="size-3.5 shrink-0 text-ink-4" />
          </a>
        </li>
      ))}
    </ul>
  );
}

function Rail({
  session,
  git,
  viewers,
  model,
  runnerConnected,
  stall,
}: {
  session: G.Session;
  git?: G.GitState;
  viewers: G.Presence[];
  model?: string;
  runnerConnected: boolean;
  stall?: Stall;
}) {
  const st = session.stats;
  const project = useProject(session.project_slug).data;
  const [primary, ...others] = repoChanges(
    git ?? { ahead: 0, behind: 0, files: [], uncommitted: 0, pushed: false },
  );
  // Several repositories: each under its name, those without changes left out.
  const changed = others.filter(hasChanges);
  const prs = pullRequests(session);
  return (
    <aside className="sticky top-0 hidden h-dvh w-[300px] shrink-0 overflow-y-auto border-l border-line bg-surface/40 lg:block">
      <RailSection title="Changes">
        {primary && others.length === 0 ? (
          <Changes repo={primary} limit={60} showTotals={!!git} />
        ) : (
          <div className="flex flex-col gap-5">
            {primary && (hasChanges(primary) || changed.length === 0) && (
              <Changes
                repo={primary}
                title={project?.repo ? `${project.repo.owner}/${project.repo.name}` : "~/workspace"}
                limit={30}
                showTotals={hasChanges(primary)}
              />
            )}
            {changed.map((r) => (
              <Changes key={r.name} repo={r} title={r.name} limit={30} showTotals />
            ))}
          </div>
        )}
      </RailSection>
      {prs.length > 0 && (
        <RailSection title="Pull requests">
          <PullRequests prs={prs} />
        </RailSection>
      )}
      <RailSection title="Work">
        <Row k="Turns" v={turnsTaken(st)} />
        <Row k="Tool calls" v={st.tool_calls} />
        <Row k="Agent time" v={<AgentTime stats={st} until={stall?.since} />} />
        <Row k="Tokens in / out" v={`${num(st.input_tokens)} / ${num(st.output_tokens)}`} />
        <Row k="Cache reads" v={num(st.cache_read_tokens)} />
      </RailSection>
      <RailSection title="Session">
        <Row
          k="Started by"
          v={
            <span className="inline-flex items-center gap-1.5">
              <Avatar user={session.created_by} size={16} />
              {session.created_by.name}
            </span>
          }
        />
        <Row k="Created" v={ago(session.created_at)} />
        {session.base_branch && <Row k="From" v={<span className="font-mono">{session.base_branch}</span>} />}
        {(model ?? session.model) && <Row k="Model" v={model ?? session.model} />}
        {session.effort && <Row k="Effort" v={session.effort} />}
        {session.runner && (
          <Row
            k="Runner"
            v={
              <span className="inline-flex items-center gap-1.5" title={stall && stallText(stall).title}>
                <span
                  className={cn(
                    "size-1.5 rounded-full",
                    stall?.kind === "unreachable" ? "bg-amber" : runnerConnected ? "bg-accent" : "bg-ink-4",
                  )}
                />
                {session.runner.name}
              </span>
            }
          />
        )}
        <Row k="Billing" v="Claude subscription" />
      </RailSection>
      <RailSection title="Follow along">
        {session.remote_control?.session_url ? (
          <>
            <a
              {...claudeAppLink(session.remote_control.session_url)}
              className="flex items-center justify-between rounded-md border border-accent/25 bg-accent-soft px-3 py-2 text-[12.5px] text-accent"
            >
              Open in the Claude app <ArrowUpRight className="size-3.5" />
            </a>
            <div className="mt-3 flex items-center justify-between">
              <span className="text-[12.5px] text-ink-4">Browser</span>
              <a
                href={session.remote_control.session_url}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-ink-3 transition-colors hover:bg-hover hover:text-ink"
              >
                claude.ai <ArrowUpRight className="size-3.5" />
              </a>
            </div>
          </>
        ) : (
          <p className="text-[12.5px] leading-relaxed text-ink-4">
            {session.remote_control?.unavailable_reason ??
              "Remote Control appears here when the session's Claude login supports it."}
          </p>
        )}
        <div className="mt-3 flex items-center justify-between">
          <span className="text-[12.5px] text-ink-4">CLI</span>
          <CopyButton text={`fctl attach ${session.id}`} label="fctl attach" />
        </div>
      </RailSection>
      <RailSection title={`Watching (${viewers.length})`}>
        <ul className="flex flex-col gap-1.5">
          {viewers.map((v) => (
            <li key={v.user.id} className="flex items-center gap-2 text-[12.5px] text-ink-2">
              <Avatar user={v.user} size={18} />
              <span className="min-w-0 flex-1 truncate">{v.user.name}</span>
              {v.connections > 1 && <span className="text-ink-4">×{v.connections}</span>}
            </li>
          ))}
        </ul>
      </RailSection>
    </aside>
  );
}

const SHARE_ROLES: SelectOption<G.SessionRole>[] = [
  { value: "viewer", label: "Can view", description: "Reads the transcript" },
  { value: "editor", label: "Can message", description: "Also messages and interrupts the agent" },
];

function ShareDialog({ open, onClose, session }: { open: boolean; onClose: () => void; session: G.Session }) {
  const shares = useQuery({
    queryKey: keys.shares(session.id),
    queryFn: () => api.shares(session.id),
    enabled: open,
  });
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<G.SessionRole>("editor");
  const add = useMutation({
    mutationFn: () => api.share(session.id, { email, role }),
    onSuccess: (r) => {
      shares.refetch();
      setEmail("");
      toast(`Shared with ${r.shares[r.shares.length - 1]?.user.name ?? email}`);
    },
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Share this session"
      description="Project members already see it. Share it with anyone else on this instance."
    >
      <form
        className="flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          add.mutate();
        }}
      >
        <Field label="Email" className="flex-1" error={add.error?.message}>
          {(id) => (
            <Input
              id={id}
              type="email"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              placeholder="teammate@example.com"
            />
          )}
        </Field>
        <Select aria-label="Access" value={role} onChange={setRole} options={SHARE_ROLES} />
        <Button type="submit" variant="primary" loading={add.isPending}>
          Share
        </Button>
      </form>
      <ul className="mt-5 flex flex-col gap-2">
        {shares.data?.shares.map((sh) => (
          <li key={sh.user.id} className="flex items-center gap-2.5 text-[13px]">
            <Avatar user={sh.user} size={22} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-ink">{sh.user.name}</span>
              <span className="block truncate text-[12px] text-ink-4">{sh.email}</span>
            </span>
            <span className="text-[12px] text-ink-3">
              {sh.role === "viewer" ? "Can view" : sh.role === "editor" ? "Can message" : "Owner"}
            </span>
            <button
              type="button"
              className="text-[12px] text-ink-4 hover:text-danger"
              onClick={() => api.unshare(session.id, sh.user.id).then(() => shares.refetch())}
            >
              Remove
            </button>
          </li>
        ))}
        {shares.data?.shares.length === 0 && (
          <li className="flex items-center gap-2 text-[12.5px] text-ink-4">
            <Users className="size-3.5" /> Not shared with anyone outside the project.
          </li>
        )}
      </ul>
    </Dialog>
  );
}

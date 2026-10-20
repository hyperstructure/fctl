// A workspace: its projects, people, runners, GitHub accounts, settings and
// what happened in it. Admins manage it here; everyone in it sees who and
// what it holds.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  ArrowRight,
  Check,
  Cpu,
  ExternalLink,
  FolderPlus,
  ImageUp,
  LogOut,
  MailPlus,
  Pencil,
  Plus,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { type ReactNode, useRef, useState } from "react";
import { AuditLog } from "@/components/audit-log";
import { GithubAvatar } from "@/components/github-avatar";
import { SessionRow } from "@/components/session/row";
import {
  Avatar,
  Banner,
  Button,
  CodeBox,
  DangerZone,
  Dialog,
  Empty,
  Field,
  GithubIcon,
  Input,
  Section,
  Select,
  type SelectOption,
  Skeleton,
  Spinner,
  StatusDot,
  Switch,
  toast,
} from "@/components/ui";
import { WORKSPACE_COLORS, WorkspaceAvatar } from "@/components/workspace/avatar";
import { avatarImage } from "@/components/workspace/avatar-image";
import type * as G from "@/gen";
import { ApiFailure, api, workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago, agoShort } from "@/lib/format";
import { BREW_INSTALL, installCommand } from "@/lib/install";
import { keys, queryClient, useMe, useProjects, useRunners, useSessions, useWorkspace } from "@/lib/queries";
import { repoSummary } from "@/lib/repos";
import { BASE_ROLES, claudeOptions, roleLabel } from "@/lib/workspaces";

export type WorkspaceTab = "projects" | "people" | "runners" | "github" | "settings" | "audit";

const TABS: { tab: WorkspaceTab; label: string; admin?: boolean }[] = [
  { tab: "projects", label: "Projects" },
  { tab: "people", label: "People" },
  { tab: "runners", label: "Runners" },
  { tab: "github", label: "GitHub" },
  { tab: "settings", label: "Settings" },
  { tab: "audit", label: "Audit log", admin: true },
];

/** Refetches what a change to a workspace touches: the page caches it by its slug, its parts by its id. */
function refresh() {
  for (const k of [keys.workspaces, ["workspace"], keys.projects, keys.runners])
    void queryClient.invalidateQueries({ queryKey: k });
}

export function WorkspacePage() {
  const { slug } = useParams({ from: "/app/w/$slug" });
  const { tab: asked = "projects", github, account, reason } = useSearch({ from: "/app/w/$slug" });
  const ws = useWorkspace(slug);
  const w = ws.data;
  const admin = w?.role === "owner" || w?.role === "admin";
  // Admins' tabs show members the workspace's projects instead.
  const tab = !w || TABS.some((t) => t.tab === asked && (!t.admin || admin)) ? asked : "projects";
  if (ws.isError)
    return (
      <div className="mx-auto max-w-xl px-6 py-24">
        <Banner tone="error">This workspace does not exist, or you aren't a member of it.</Banner>
      </div>
    );
  return (
    <div className="mx-auto max-w-[920px] px-6 pt-12 pb-24">
      {w ? (
        <header className="flex animate-rise items-start gap-4 sm:gap-5">
          <WorkspaceAvatar workspace={w} size={60} className="max-sm:size-12! max-sm:text-[18px]!" />
          <div className="min-w-0 flex-1">
            <h1 className="truncate font-display text-[28px] leading-[1.1] font-medium tracking-[-0.022em] sm:text-[36px]">
              {w.name}
            </h1>
            <div className="mt-2 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12.5px] text-ink-3">
              <span className="text-ink-2">{roleLabel(w.role)}</span>
              <span className="min-w-0 truncate">as {w.email}</span>
              <span className="text-ink-4 max-sm:hidden">·</span>
              <span>
                {w.members} {w.members === 1 ? "person" : "people"}
              </span>
              <span className="inline-flex items-center gap-1.5">
                <span
                  className={cn(
                    "size-1.5 rounded-full",
                    w.runners_online > 0 ? "bg-accent shadow-[0_0_8px_var(--c-accent)]" : "bg-ink-4",
                  )}
                />
                {w.runners === 0
                  ? "no runners yet"
                  : `${w.runners_online} of ${w.runners} ${w.runners === 1 ? "runner" : "runners"} online`}
              </span>
              {w.require_two_factor && (
                <span className="inline-flex items-center gap-1 text-ink-3">
                  <ShieldCheck className="size-3.5 text-accent" /> Two-factor required
                </span>
              )}
            </div>
          </div>
          {admin && !w.blocked && (
            // The People tab has the form; on a phone the name needs the room.
            <Link
              to="/w/$slug"
              params={{ slug }}
              search={{ tab: "people" }}
              className="shrink-0 max-sm:hidden"
            >
              <Button icon={<MailPlus className="size-3.5" />}>Invite</Button>
            </Link>
          )}
        </header>
      ) : (
        <div className="flex items-center gap-5">
          <Skeleton className="size-[60px] rounded-xl" />
          <div className="flex-1">
            <Skeleton className="h-9 w-72" />
            <Skeleton className="mt-3 h-4 w-56" />
          </div>
        </div>
      )}
      {w?.blocked ? (
        <div className="mt-8 flex flex-col items-start gap-4">
          <Banner tone="warn" className="self-stretch">
            {w.name} requires two-factor sign-in of its members, and you haven't set it up. Until you do, its
            projects and sessions are closed to you.{" "}
            <Link
              to="/settings"
              search={{ linked: undefined, error: undefined }}
              className="underline underline-offset-2"
            >
              Set it up in Settings
            </Link>
            .
          </Banner>
          <Leave w={w} />
        </div>
      ) : (
        <>
          <nav className="-mx-6 mt-8 overflow-x-auto px-6 [scrollbar-width:none]">
            <div className="flex w-max min-w-full gap-1 border-b border-line">
              {TABS.filter((t) => !t.admin || admin).map((t) => (
                <Link
                  key={t.tab}
                  to="/w/$slug"
                  params={{ slug }}
                  search={{ tab: t.tab === "projects" ? undefined : t.tab }}
                  className={cn(
                    "relative shrink-0 px-3 py-2.5 text-[13.5px] transition-colors",
                    tab === t.tab ? "text-ink" : "text-ink-3 hover:text-ink",
                  )}
                >
                  {t.label}
                  {tab === t.tab && (
                    <span className="absolute inset-x-2 -bottom-px h-[2px] rounded-full bg-accent" />
                  )}
                </Link>
              ))}
            </div>
          </nav>
          <div className="mt-8">
            {w && tab === "projects" && <Projects w={w} />}
            {w && tab === "people" && <People w={w} />}
            {w && tab === "runners" && <Runners w={w} />}
            {w && tab === "github" && <Github w={w} returned={github} account={account} reason={reason} />}
            {w && tab === "settings" && <WorkspaceSettings w={w} />}
            {w && tab === "audit" && admin && <Audit w={w} />}
          </div>
        </>
      )}
    </div>
  );
}

// ───────────────────────────── projects ─────────────────────────────

/** A project's repositories in its line of the list: "hyperstructure/fctl (+3 more)". */
function RepoLine({ project }: { project: G.Project }) {
  const s = repoSummary(project);
  if (!s) return "No repository";
  return (
    <>
      {s.name && <span className="font-mono">{s.name}</span>}
      {s.name && s.more ? ` ${s.more}` : s.more}
    </>
  );
}

function Projects({ w }: { w: G.Workspace }) {
  const projects = useProjects();
  const sessions = useSessions({ workspace: w.id, limit: 8 });
  const mine = (projects.data?.projects ?? []).filter((p) => p.workspace?.id === w.id);
  // Projects its base role doesn't open to you, which you aren't a member of.
  const hidden = projects.data ? Math.max(0, w.projects - mine.length) : 0;
  const closed = `${hidden === 1 ? "1 project" : `${hidden} projects`} you aren't a member of`;
  return (
    <div className="flex flex-col gap-12">
      <SetUp w={w} projects={mine.length} />
      <section>
        <div className="mb-2 flex items-baseline justify-between">
          <h2 className="text-[13px] font-semibold text-ink">Projects</h2>
          <Link to="/new" search={{ workspace: w.slug }}>
            <Button size="sm" variant="ghost" icon={<Plus className="size-3.5" />}>
              New project
            </Button>
          </Link>
        </div>
        {projects.isPending ? (
          <div className="space-y-3 py-3">
            <Skeleton className="h-12" />
            <Skeleton className="h-12" />
          </div>
        ) : mine.length === 0 && hidden > 0 ? (
          <Empty
            icon={<FolderPlus className="size-6" />}
            title={`${w.name} has ${closed}`}
            action={
              <Link to="/new" search={{ workspace: w.slug }}>
                <Button icon={<Plus className="size-3.5" />}>Create a project</Button>
              </Link>
            }
          >
            Its admins add people to its projects. You see the ones you are added to here, and on the home
            page.
          </Empty>
        ) : mine.length === 0 ? (
          <Empty
            icon={<FolderPlus className="size-6" />}
            title="No projects yet"
            action={
              <Link to="/new" search={{ workspace: w.slug }}>
                <Button variant="primary" icon={<Plus className="size-3.5" />}>
                  Create a project
                </Button>
              </Link>
            }
          >
            A project is a repository plus what agents need to know to work in it.
          </Empty>
        ) : (
          <ul className="border-t border-line">
            {mine.map((p) => (
              <li key={p.id}>
                <Link
                  to="/p/$slug"
                  params={{ slug: p.slug }}
                  search={{ tab: undefined }}
                  className="group flex items-center gap-3.5 border-b border-line px-3 py-3.5 transition-colors hover:bg-hover/30"
                >
                  <span className="flex size-4 items-center justify-center">
                    {p.active_sessions > 0 ? (
                      <StatusDot status="running" />
                    ) : (
                      <span className="size-1.5 rounded-full bg-ink-4/60" />
                    )}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[14.5px] text-ink">{p.name}</div>
                    <div className="mt-0.5 truncate text-[12px] text-ink-4">
                      <RepoLine project={p} />
                      {" · "}
                      {p.total_sessions} {p.total_sessions === 1 ? "session" : "sessions"}
                      {p.active_sessions > 0 && (
                        <span className="text-accent"> · {p.active_sessions} active</span>
                      )}
                    </div>
                  </div>
                  <span className="shrink-0 text-[12px] text-ink-4 tabular-nums">
                    {agoShort(p.last_activity_at ?? p.created_at)}
                  </span>
                  <ArrowRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-hover:translate-x-0.5 group-hover:opacity-100 max-sm:hidden" />
                </Link>
              </li>
            ))}
          </ul>
        )}
        {mine.length > 0 && hidden > 0 && (
          <p className="mt-3 px-3 text-[12.5px] text-ink-4">
            And {closed}: its admins add people to projects.
          </p>
        )}
      </section>
      {(sessions.data?.sessions.length ?? 0) > 0 && (
        <section>
          <h2 className="mb-2 text-[13px] font-semibold text-ink">Recently</h2>
          <div className="border-t border-line">
            {sessions.data?.sessions.map((s) => (
              <SessionRow key={s.id} s={s} />
            ))}
          </div>
        </section>
      )}
    </div>
  );
}

/** What a new workspace still needs, for its admins, until it has it all. */
function SetUp({ w, projects }: { w: G.Workspace; projects: number }) {
  const admin = w.role === "owner" || w.role === "admin";
  const gh = useQuery({ queryKey: keys.workspaceGithub(w.id), queryFn: () => workspaces.github(w.id) });
  // Without a GitHub App on the instance there is nothing to connect.
  const github = w.github.length > 0 || gh.data?.configured === true;
  // A workspace for one has nobody to invite: its admins skip the step.
  const skipInvites = useMutation({
    mutationFn: () => workspaces.update(w.id, { skip_invites: true }),
    onSuccess: (x) => {
      queryClient.setQueryData(keys.workspace(w.slug), x);
      refresh();
      toast("Skipped: invite people from the People tab any time");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const invited = w.members > 1;
  const all: { done: boolean; skip?: boolean; title: string; body: string; action: ReactNode }[] = [
    {
      done: invited,
      skip: w.skip_invites && !invited,
      title: "Invite the people you work with",
      body: "They join with the address you invite, and see its projects as the base role lets them.",
      action: (
        <>
          <Link to="/w/$slug" params={{ slug: w.slug }} search={{ tab: "people" }}>
            <Button size="sm" icon={<MailPlus className="size-3.5" />}>
              Invite
            </Button>
          </Link>
          <Button
            size="sm"
            variant="ghost"
            loading={skipInvites.isPending}
            onClick={() => skipInvites.mutate()}
          >
            Skip for now
          </Button>
        </>
      ),
    },
    {
      done: w.runners > 0,
      title: "Bring a machine online",
      body: "Its sessions wait until one of its runners is online: register a Mac or Linux machine, or add one of yours.",
      action: (
        <Link to="/w/$slug" params={{ slug: w.slug }} search={{ tab: "runners" }}>
          <Button size="sm" icon={<Cpu className="size-3.5" />}>
            Add a runner
          </Button>
        </Link>
      ),
    },
    {
      done: w.github.length > 0,
      skip: !github,
      title: "Connect a GitHub account",
      body: "Its projects use the repositories of the accounts the workspace works in.",
      action: (
        <Link to="/w/$slug" params={{ slug: w.slug }} search={{ tab: "github" }}>
          <Button size="sm" icon={<GithubIcon className="size-3.5" />}>
            Connect
          </Button>
        </Link>
      ),
    },
    {
      done: projects > 0,
      title: "Create a project",
      body: "A repository plus what agents need to know to work in it.",
      action: (
        <Link to="/new" search={{ workspace: w.slug }}>
          <Button size="sm" icon={<FolderPlus className="size-3.5" />}>
            Create
          </Button>
        </Link>
      ),
    },
  ];
  const steps = all.filter((s) => !s.skip);
  if (!admin || steps.every((s) => s.done)) return null;
  const done = steps.filter((s) => s.done).length;
  return (
    <section className="animate-rise">
      <div className="flex items-baseline justify-between gap-4">
        <h2 className="min-w-0 font-display text-[22px] tracking-[-0.01em] text-ink [overflow-wrap:anywhere]">
          Set up {w.name}
        </h2>
        <span className="shrink-0 text-[12px] text-ink-4 tabular-nums">
          {done} of {steps.length} done
        </span>
      </div>
      <ol className="mt-3 divide-y divide-line border-y border-line">
        {steps.map((s) => (
          <li
            key={s.title}
            className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-4 py-4 sm:grid-cols-[auto_minmax(0,1fr)_auto]"
          >
            <span
              className={cn(
                "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border transition-colors",
                s.done ? "border-highlight bg-highlight text-highlight-ink" : "border-line-strong text-ink-4",
              )}
            >
              {s.done ? <Check className="size-3.5" /> : <span className="size-1.5 rounded-full bg-ink-4" />}
            </span>
            <div>
              <div
                className={cn(
                  "text-[14.5px] font-medium",
                  s.done ? "text-ink-3 line-through decoration-ink-4" : "text-ink",
                )}
              >
                {s.title}
              </div>
              {!s.done && <div className="mt-1 text-[13.5px] leading-relaxed text-ink-3">{s.body}</div>}
            </div>
            {/* Under the words on a phone; beside them on wider screens, the step's own action at the edge. */}
            {!s.done && (
              <div className="col-start-2 mt-3 flex items-center gap-1 sm:col-start-3 sm:row-start-1 sm:mt-0 sm:flex-row-reverse sm:items-start">
                {s.action}
              </div>
            )}
          </li>
        ))}
      </ol>
    </section>
  );
}

// ───────────────────────────── people ─────────────────────────────

const inviteRoles: SelectOption<G.WorkspaceRole>[] = [
  { value: "member", label: "Member", description: "Works in its projects, as the base role allows" },
  { value: "admin", label: "Admin", description: "Also manages people, runners, GitHub and settings" },
];

function People({ w }: { w: G.Workspace }) {
  const { data: me } = useMe();
  const admin = w.role === "owner" || w.role === "admin";
  const list = useQuery({ queryKey: keys.workspaceMembers(w.id), queryFn: () => workspaces.members(w.id) });
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<G.WorkspaceRole>("member");
  const [transferTo, setTransferTo] = useState<G.WorkspaceMember | null>(null);
  const [removing, setRemoving] = useState<G.WorkspaceMember | null>(null);
  const done = () => {
    void list.refetch();
    refresh();
  };
  const invite = useMutation({
    mutationFn: () => workspaces.invite(w.id, { email: email.trim(), role }),
    onSuccess: (i) => {
      setEmail("");
      done();
      toast(i.emailed ? `Invited ${i.email}, and emailed them` : `Invited ${i.email}`);
    },
  });
  // Instance admins can invite someone to the instance and here at once.
  const inviteToInstance = useMutation({
    mutationFn: () => api.createInvite({ email: email.trim(), workspaces: [{ workspace: w.id, role }] }),
    onSuccess: (i) => {
      setEmail("");
      invite.reset();
      done();
      setLink(i);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const [link, setLink] = useState<G.Invite | null>(null);
  const setMemberRole = useMutation({
    mutationFn: (v: { user: string; role: G.WorkspaceRole }) => workspaces.setRole(w.id, v.user, v.role),
    onSuccess: done,
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: (m: G.WorkspaceMember) => workspaces.removeMember(w.id, m.user.id),
    onSuccess: (_, m) => {
      setRemoving(null);
      done();
      toast(`${m.user.name} is no longer in ${w.name}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const revoke = useMutation({
    mutationFn: (id: string) => workspaces.revokeInvitation(w.id, id),
    onSuccess: done,
    onError: (e) => toast(e.message, "error"),
  });
  const transfer = useMutation({
    mutationFn: (m: G.WorkspaceMember) => workspaces.transfer(w.id, m.user.id),
    onSuccess: (_, m) => {
      setTransferTo(null);
      done();
      toast(`${m.user.name} is the primary admin of ${w.name}; you stay an admin`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const notOnInstance = invite.error instanceof ApiFailure && invite.error.code === "access_denied";
  const instanceAdmin = me?.user.role === "owner" || me?.user.role === "admin";
  return (
    <div>
      {admin && (
        <form
          className="flex flex-col gap-2 sm:flex-row sm:items-end"
          onSubmit={(e) => {
            e.preventDefault();
            if (email.trim()) invite.mutate();
          }}
        >
          <Field
            label="Invite someone"
            className="flex-1"
            hint="An address they sign in with. They join from their home page; Settings shows which of their addresses they joined as."
            error={invite.error && !notOnInstance ? invite.error.message : undefined}
          >
            {(id) => (
              <Input
                id={id}
                type="email"
                value={email}
                onChange={(e) => {
                  setEmail(e.target.value);
                  invite.reset();
                }}
                placeholder="grace@example.com"
              />
            )}
          </Field>
          <div className="flex gap-2 sm:mb-[26px]">
            <Select aria-label="Role" value={role} onChange={setRole} options={inviteRoles} />
            <Button
              type="submit"
              variant="primary"
              icon={<MailPlus className="size-3.5" />}
              loading={invite.isPending}
              disabled={!email.trim()}
            >
              Invite
            </Button>
          </div>
        </form>
      )}
      {notOnInstance && (
        <Banner tone="info" className="mt-3">
          {email.trim()} can't sign in to this instance yet.{" "}
          {instanceAdmin ? (
            <>
              You are an instance admin: invite them to it and to {w.name} at once.
              <div className="mt-3">
                <Button
                  size="sm"
                  variant="primary"
                  loading={inviteToInstance.isPending}
                  onClick={() => inviteToInstance.mutate()}
                >
                  Invite them to the instance and {w.name}
                </Button>
              </div>
            </>
          ) : (
            "An instance admin invites them first, and can invite them to this workspace at the same time."
          )}
        </Banner>
      )}
      <ul className={cn("border-t border-line", admin && "mt-8")}>
        {list.isPending && (
          <li className="space-y-3 py-4">
            <Skeleton className="h-10" />
            <Skeleton className="h-10" />
          </li>
        )}
        {list.data?.members.map((m) => {
          const you = m.user.id === me?.user.id;
          return (
            <li key={m.user.id} className="flex items-center gap-3 border-b border-line py-3">
              <Avatar user={m.user} size={30} />
              <div className="flex min-w-0 flex-1 flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
                <div className="min-w-0 flex-1">
                  <div className="flex items-center gap-1.5 text-[14px] text-ink">
                    <span className="truncate">{m.user.name}</span>
                    {you && <span className="text-[12px] text-ink-4">you</span>}
                    {admin && m.two_factor && (
                      <ShieldCheck className="size-3.5 text-accent" aria-label="Has two-factor sign-in" />
                    )}
                  </div>
                  <div className="truncate text-[12px] text-ink-4">
                    {m.email} · joined {ago(m.joined_at)}
                    {admin && w.require_two_factor && !m.two_factor && (
                      <span className="text-amber"> · closed to them until they set up two-factor</span>
                    )}
                  </div>
                </div>
                <div className="-ml-1 flex shrink-0 items-center gap-1 sm:ml-0">
                  {m.role === "owner" ? (
                    <span className="px-2 text-[12.5px] text-ink-3">Primary admin</span>
                  ) : admin && !you ? (
                    <Select
                      size="sm"
                      aria-label={`Role of ${m.user.name}`}
                      value={m.role}
                      onChange={(r) => setMemberRole.mutate({ user: m.user.id, role: r })}
                      options={inviteRoles}
                    />
                  ) : (
                    <span className="px-2 text-[12.5px] text-ink-3">{roleLabel(m.role)}</span>
                  )}
                  {w.role === "owner" && m.role === "admin" && (
                    <Button size="sm" variant="ghost" onClick={() => setTransferTo(m)}>
                      Make primary admin
                    </Button>
                  )}
                  {admin && !you && m.role !== "owner" && (
                    <button
                      type="button"
                      aria-label={`Remove ${m.user.name}`}
                      className="rounded p-1.5 text-ink-4 hover:text-danger"
                      onClick={() => setRemoving(m)}
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  )}
                </div>
              </div>
            </li>
          );
        })}
        {list.data?.invitations.map((i) => (
          <li key={i.id} className="flex items-center gap-3 border-b border-line py-3">
            <span className="flex size-[30px] items-center justify-center rounded-full border border-dashed border-line-strong text-ink-4">
              <MailPlus className="size-3.5" />
            </span>
            <div className="min-w-0 flex-1">
              <div className="truncate text-[14px] text-ink-2">{i.email}</div>
              <div className="truncate text-[12px] text-ink-4">
                Invited as {roleLabel(i.role).toLowerCase()}
                {i.invited_by && ` by ${i.invited_by.name}`} · {ago(i.created_at)} · until{" "}
                {new Date(i.expires_at).toLocaleDateString(undefined, { month: "short", day: "numeric" })}
              </div>
            </div>
            <Button
              size="sm"
              variant="ghost"
              loading={revoke.isPending && revoke.variables === i.id}
              onClick={() => revoke.mutate(i.id)}
            >
              Take back
            </Button>
          </li>
        ))}
      </ul>
      <Dialog
        open={!!removing}
        onClose={() => setRemoving(null)}
        title={`Remove ${removing?.user.name ?? ""} from ${w.name}?`}
        description="They lose access to its projects and sessions at once. Their sessions here stop, and so do its sessions on their runners, which leave it. Their project memberships and shares here go."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setRemoving(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={remove.isPending}
            onClick={() => removing && remove.mutate(removing)}
          >
            Remove
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={!!transferTo}
        onClose={() => setTransferTo(null)}
        title={`Make ${transferTo?.user.name ?? ""} the primary admin?`}
        description="A workspace has one primary admin, whom no other admin can demote or remove. You stay an admin, and only they can hand the role back."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setTransferTo(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            loading={transfer.isPending}
            onClick={() => transferTo && transfer.mutate(transferTo)}
          >
            Hand it over
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={!!link}
        onClose={() => setLink(null)}
        title="Invitation ready"
        description={
          link?.emailed
            ? `We emailed ${link.email}. You can also send them this link: they join ${w.name} with the account they make.`
            : `Send this link to ${link?.email}. It works once, for 14 days, and they join ${w.name} with the account they make.`
        }
      >
        {link?.url && <CodeBox value={link.url} />}
        <div className="mt-5 flex justify-end">
          <Button variant="primary" onClick={() => setLink(null)}>
            Done
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

// ───────────────────────────── runners ─────────────────────────────

function Runners({ w }: { w: G.Workspace }) {
  const admin = w.role === "owner" || w.role === "admin";
  const list = useQuery({ queryKey: keys.workspaceRunners(w.id), queryFn: () => workspaces.runners(w.id) });
  const all = useRunners();
  const [name, setName] = useState("");
  const [reg, setReg] = useState<G.RunnerRegistration | null>(null);
  const [adding, setAdding] = useState<string>();
  const done = () => {
    void list.refetch();
    refresh();
  };
  const create = useMutation({
    mutationFn: () => workspaces.createRunner(w.id, name.trim()),
    onSuccess: (r) => {
      setReg(r);
      setName("");
      done();
    },
  });
  const add = useMutation({
    mutationFn: (runner: string) => workspaces.addRunner(w.id, runner),
    onSuccess: () => {
      setAdding(undefined);
      done();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const [takingOut, setTakingOut] = useState<G.Runner | null>(null);
  const remove = useMutation({
    mutationFn: (runner: string) => workspaces.removeRunner(w.id, runner),
    onSuccess: () => {
      setTakingOut(null);
      done();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const here = new Set(list.data?.runners.map((r) => r.id));
  const yours = (all.data?.runners ?? []).filter((r) => r.yours && !here.has(r.id));
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        Sessions in {w.name} start only on these machines, which see its code and secrets while they work.
        Each runner belongs to the person who registered it, who can let it work for several workspaces.
      </p>
      {admin && (
        <div className="mt-6 flex flex-col gap-4 sm:flex-row sm:items-end">
          <form
            className="flex flex-1 items-end gap-2"
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) create.mutate();
            }}
          >
            <Field label="Register a machine" className="flex-1" error={create.error?.message}>
              {(id) => (
                <Input
                  id={id}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="studio mac"
                />
              )}
            </Field>
            <Button
              type="submit"
              variant="primary"
              icon={<Plus className="size-3.5" />}
              loading={create.isPending}
              disabled={!name.trim()}
            >
              Register
            </Button>
          </form>
          {yours.length > 0 && (
            <div className="flex items-end gap-2">
              <Field label="Or add one of yours">
                {(id) => (
                  <Select
                    id={id}
                    value={adding}
                    placeholder="Your runners"
                    onChange={setAdding}
                    options={yours.map((r) => ({
                      value: r.id,
                      label: r.name,
                      description: r.workspaces?.length
                        ? `Also runs ${r.workspaces.map((x) => x.name).join(", ")}`
                        : "Runs no workspace yet",
                    }))}
                  />
                )}
              </Field>
              <Button disabled={!adding} loading={add.isPending} onClick={() => adding && add.mutate(adding)}>
                Add
              </Button>
            </div>
          )}
        </div>
      )}
      {list.data?.runners.length === 0 ? (
        admin ? (
          <FirstRunner w={w} yours={yours.length > 0} />
        ) : (
          <Empty icon={<Cpu className="size-6" />} title="No runners yet">
            {w.name}'s sessions wait until an admin adds a runner: a machine that runs its agents in
            sandboxes.{" "}
            {w.primary
              ? `Ask ${w.primary.name}, its primary admin, to add one.`
              : "Ask one of its admins to add one."}
          </Empty>
        )
      ) : (
        <ul className="mt-6 border-t border-line">
          {list.data?.runners.map((r) => {
            const others = (r.workspaces ?? []).filter((x) => x.id !== w.id);
            return (
              <li key={r.id} className="flex items-center gap-4 border-b border-line py-4">
                <span
                  className={cn(
                    "size-2 shrink-0 rounded-full",
                    r.status === "disabled"
                      ? "bg-danger"
                      : r.drain
                        ? "bg-amber"
                        : r.status === "online"
                          ? "bg-accent shadow-[0_0_10px_var(--c-accent)]"
                          : "bg-ink-4",
                  )}
                />
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] text-ink">
                    {r.name}{" "}
                    <span className="ml-1 text-[12px] text-ink-4">
                      {r.status === "online" && r.drain ? "draining" : r.status}
                    </span>
                  </div>
                  <div className="mt-0.5 truncate text-[12px] text-ink-4">
                    {r.created_by ? `${r.created_by.name}'s` : "A"}{" "}
                    {r.driver ? `${r.driver} runner` : "runner"} · {r.active_sessions}/{r.capacity} sessions
                    {r.last_seen_at ? ` · seen ${ago(r.last_seen_at)}` : " · never connected"}
                  </div>
                  {others.length > 0 && (
                    <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11.5px] text-ink-4">
                      Also runs
                      {others.map((x) => (
                        <span key={x.id} className="inline-flex items-center gap-1 text-ink-3">
                          <WorkspaceAvatar workspace={x} size={13} /> {x.name}
                        </span>
                      ))}
                    </div>
                  )}
                </div>
                {(admin || r.yours) && (
                  <Button size="sm" variant="ghost" onClick={() => setTakingOut(r)}>
                    Take out
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}
      <Dialog
        open={!!takingOut}
        onClose={() => setTakingOut(null)}
        title={`Take ${takingOut?.name ?? ""} out of ${w.name}?`}
        description={
          takingOut?.yours
            ? `It stops taking ${w.name}'s sessions; the ones running there carry on. You can add it again.`
            : `It stops taking ${w.name}'s sessions; the ones running there carry on. Only its owner can add it again.`
        }
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setTakingOut(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={remove.isPending}
            onClick={() => takingOut && remove.mutate(takingOut.id)}
          >
            Take it out
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={!!reg}
        onClose={() => setReg(null)}
        wide
        title={`Bring ${reg?.runner.name ?? "it"} online`}
        description={`Run these on the machine. It runs ${w.name}'s sessions. The token is shown once and lets the machine run sessions: keep it private.`}
      >
        {reg && (
          <div className="flex flex-col gap-3">
            <div className="text-[12.5px] text-ink-3">1. Install fctl</div>
            <CodeBox value={installCommand()} />
            <div className="text-[12.5px] text-ink-3">2. Register it as a runner and start the service</div>
            <CodeBox value={`FCTL_SERVER=${location.origin} fctl host install --token ${reg.token}`} />
            <p className="text-[12.5px] leading-relaxed text-ink-4">
              It shows up here as online within seconds. Add it to your other workspaces from their Runners
              tab.
            </p>
          </div>
        )}
      </Dialog>
    </div>
  );
}

/** For admins of a workspace without runners: what bringing a machine online takes, under the form. */
function FirstRunner({ w, yours }: { w: G.Workspace; yours: boolean }) {
  const steps: { title: string; body: ReactNode }[] = [
    {
      title: "Choose a machine that stays on",
      body: "A Mac with Apple silicon or a Linux machine. Agents work there in sandboxes: install Apple's container on macOS 26 or later, Lima on older macOS, or Docker on Linux.",
    },
    {
      title: "Install fctl on it",
      body: (
        <>
          <CodeBox value={installCommand()} />
          <p>
            Or with Homebrew:{" "}
            <code className="rounded bg-raised px-1 font-mono text-[12px] text-ink-2">{BREW_INSTALL}</code>
          </p>
        </>
      ),
    },
    {
      title: `Register it for ${w.name}`,
      body: (
        <>
          <p>
            Sign in, then register it as a runner and start its service. It shows up here as online within
            seconds.
          </p>
          <CodeBox value={`fctl login ${location.origin}`} />
          <CodeBox value={`fctl host install --workspace ${w.slug}`} />
          <p className="text-[12.5px] text-ink-4">
            Rather not sign in there? Name it above and choose Register for a command with a token.
          </p>
        </>
      ),
    },
  ];
  return (
    <section className="mt-10 animate-rise">
      <h3 className="font-display text-xl text-ink">No runners yet</h3>
      <p className="mt-1 max-w-2xl text-[14px] leading-relaxed text-ink-3">
        {w.name}'s sessions wait until one of its runners is online.{" "}
        {yours ? "Add one of yours above, or bring another machine online:" : "To bring a machine online:"}
      </p>
      <ol className="mt-4 divide-y divide-line border-y border-line">
        {steps.map((s, i) => (
          <li key={s.title} className="flex gap-4 py-4">
            <span className="mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border border-line-strong font-mono text-[11.5px] text-ink-3">
              {i + 1}
            </span>
            <div className="min-w-0 flex-1">
              <div className="text-[14.5px] font-medium text-ink">{s.title}</div>
              <div className="mt-1 flex max-w-2xl flex-col gap-2 text-[13.5px] leading-relaxed text-ink-3">
                {s.body}
              </div>
            </div>
          </li>
        ))}
      </ol>
    </section>
  );
}

// ───────────────────────────── GitHub ─────────────────────────────

/** How coming back from GitHub went (`api::github`), on the GitHub tab. */
export const GITHUB_OUTCOMES = [
  "choose",
  "connected",
  "refused",
  "cancelled",
  "failed",
  "expired",
  "other_user",
  "asked_owner",
  "installed",
] as const;
export type GithubOutcome = (typeof GITHUB_OUTCOMES)[number];

/** Why GitHub's answer let nothing be connected (`fctl_domain::github::Refusal`). */
export const GITHUB_REFUSALS = ["no_installation", "no_admin", "not_admin", "unreachable"] as const;
export type GithubRefusal = (typeof GITHUB_REFUSALS)[number];

/** The most repositories a connection names (`fctl_domain::repos::MAX_REPOS`). */
const MAX_REPOS = 100;

/** Up to four repository names, and how many more. */
function someRepos(repos: string[]) {
  return `${repos.slice(0, 4).join(", ")}${repos.length > 4 ? ` and ${repos.length - 4} more` : ""}`;
}

/** What the tab says when the browser comes back from GitHub. */
function Returned({
  w,
  outcome,
  account,
  reason,
  connection,
  choosing,
  app,
}: {
  w: G.Workspace;
  outcome: GithubOutcome;
  account?: string;
  reason?: GithubRefusal;
  connection?: G.GithubConnection;
  /** What the sign-in showed waits below for a choice. */
  choosing: boolean;
  app: string;
}) {
  const repos = connection?.repos;
  switch (outcome) {
    case "choose":
      return choosing ? null : (
        <Banner tone="info">
          What GitHub showed you waits 10 minutes for you to choose from, and that ran out. Connect with
          GitHub again.
        </Banner>
      );
    case "connected":
      return (
        <Banner tone="ok">
          {w.name} works in {account ?? "the account"} now
          {repos ? `: ${someRepos(repos)}` : ", in every repository the app reaches there"}.
        </Banner>
      );
    case "refused":
      return (
        <Banner tone="error">
          {reason === "no_installation"
            ? account
              ? `GitHub shows you no installation of the ${app} app on ${account}: it isn't installed there, or it reaches no repository you can see. Install it there, or ask an admin of its repositories to connect it.`
              : `GitHub shows you no installation of the ${app} app. Install it on your own account or an organization you own, or ask an admin of the repositories to connect them.`
            : reason === "not_admin"
              ? `GitHub says you aren't an admin of every repository you named in ${account}. Name the ones you are an admin of, or ask an admin of the others to connect them.`
              : reason === "unreachable"
                ? `The app doesn't reach every repository you named in ${account}, or you can't see them on GitHub. Add them to the app's installation there, or check their names.`
                : account
                  ? `GitHub says you aren't an admin of any repository of ${account} the app reaches, so you can't connect it. An admin of its repositories, such as an owner of ${account}, connects it.`
                  : `GitHub says you aren't an admin of any repository the ${app} app reaches on the accounts it shows you, so there is nothing for you to connect. An admin of their repositories, such as an owner of the organization, connects them.`}
        </Banner>
      );
    case "cancelled":
      return <Banner tone="info">You didn't sign in to GitHub, so nothing was connected.</Banner>;
    case "failed":
      return (
        <Banner tone="error">
          Connecting {account ?? "to GitHub"} didn't work: GitHub didn't answer as expected. Try again.
        </Banner>
      );
    case "expired":
      return (
        <Banner tone="info">
          {account
            ? `That link to connect ${account} ran out. Connect it again.`
            : "That link ran out. Try again."}
        </Banner>
      );
    case "other_user":
      return (
        <Banner tone="info">
          That link to connect {account ?? "GitHub"} is for another fctl account. Open it where you're signed
          in as that account, or connect here.
        </Banner>
      );
    case "asked_owner":
      return (
        <Banner tone="info">
          GitHub asked the organization's owners to install the app. Once they have, connect the organization
          here.
        </Banner>
      );
    case "installed":
      return (
        <Banner tone="info">
          The {app} app changed on GitHub. To choose what {w.name} works in, connect with GitHub below: GitHub
          shows fctl what you may connect.
        </Banner>
      );
  }
}

/**
 * What a GitHub sign-in showed the admin they may connect of an account, to choose from: what the workspace
 * works in there now, else everything, to begin with.
 */
function Choice({
  w,
  offer,
  connection,
  app,
  onDone,
}: {
  w: G.Workspace;
  offer: G.GithubOffer;
  connection?: G.GithubConnection;
  app: string;
  /** `connected`: the choice is made; otherwise nothing was. */
  onDone: (g: G.WorkspaceGithub, connected: boolean) => void;
}) {
  const [every, setEvery] = useState(offer.all && (!connection?.repos || offer.repos.length === 0));
  const [picked, setPicked] = useState<string[]>(() => {
    const now = connection?.repos?.map((r) => r.toLowerCase());
    if (now) return offer.repos.filter((r) => now.includes(r.toLowerCase()));
    return offer.repos.length <= MAX_REPOS ? offer.repos : [];
  });
  const [filter, setFilter] = useState("");
  const choose = useMutation({
    mutationFn: () => workspaces.chooseGithub(w.id, offer.account, { repos: every ? undefined : picked }),
    onSuccess: (g) => onDone(g, true),
    onError: (e) => {
      // It ran out: the choice goes, and the message stays.
      if (e instanceof ApiFailure && e.status === 404) {
        toast(e.message, "error");
        void queryClient.invalidateQueries({ queryKey: keys.workspaceGithub(w.id) });
      }
    },
  });
  const dismiss = useMutation({
    mutationFn: () => workspaces.dismissGithubOffer(w.id, offer.account),
    onSuccess: (g) => onDone(g, false),
    onError: (e) => toast(e.message, "error"),
  });
  const needle = filter.trim().toLowerCase();
  const shown = needle ? offer.repos.filter((r) => r.toLowerCase().includes(needle)) : offer.repos;
  const toggle = (r: string, on: boolean) => setPicked((p) => (on ? [...p, r] : p.filter((x) => x !== r)));
  const ready = every || (picked.length > 0 && picked.length <= MAX_REPOS);
  const heading = `choice-${offer.account}`;
  return (
    <section
      aria-labelledby={heading}
      className="animate-rise rounded-lg border border-line-strong p-4 sm:p-5"
    >
      <div className="flex items-start gap-3 sm:items-center">
        <GithubAvatar account={offer.account} />
        <div className="min-w-0">
          <h3 id={heading} className="text-[14px] font-semibold text-ink [overflow-wrap:anywhere]">
            Choose what {w.name} works in on {offer.account}
          </h3>
          <p className="text-[12.5px] text-ink-3">Signed in to GitHub as @{offer.github_user}</p>
        </div>
      </div>
      {offer.all && (
        <fieldset className="mt-4 flex flex-col gap-2.5">
          <legend className="sr-only">Which repositories</legend>
          <label className="flex cursor-pointer items-start gap-2.5 text-[13.5px] text-ink select-none">
            <input
              type="radio"
              name={heading}
              checked={every}
              onChange={() => setEvery(true)}
              className="mt-[3px] size-4 shrink-0 accent-[var(--c-accent)]"
            />
            <span>
              Every repository, also those the app reaches there later
              <span className="block text-[12.5px] text-ink-3">
                {offer.repos.length ? `It reaches ${someRepos(offer.repos)} now.` : "It reaches none yet."}
              </span>
            </span>
          </label>
          {offer.repos.length > 0 && (
            <label className="flex cursor-pointer items-start gap-2.5 text-[13.5px] text-ink select-none">
              <input
                type="radio"
                name={heading}
                checked={!every}
                onChange={() => setEvery(false)}
                className="mt-[3px] size-4 shrink-0 accent-[var(--c-accent)]"
              />
              Only the ones you choose
            </label>
          )}
        </fieldset>
      )}
      {!offer.all && (
        <p className="mt-4 max-w-2xl text-[13px] leading-relaxed text-ink-3">
          The {app} app reaches these repositories of {offer.account} that you're an admin of. {w.name}'s
          projects may use the ones you choose.
        </p>
      )}
      {!every && (
        <div className={cn("mt-3 flex flex-col gap-2", offer.all && "sm:pl-6.5")}>
          {offer.repos.length > 8 && (
            <div className="flex items-center gap-2">
              <Input
                value={filter}
                onChange={(e) => setFilter(e.target.value)}
                placeholder="Find a repository"
                aria-label="Find a repository"
                className="flex-1"
              />
              <Button
                size="sm"
                variant="quiet"
                onClick={() => setPicked((p) => [...p, ...shown.filter((r) => !p.includes(r))])}
              >
                All
              </Button>
              <Button size="sm" variant="quiet" onClick={() => setPicked([])}>
                None
              </Button>
            </div>
          )}
          <ul className="max-h-72 divide-y divide-line overflow-y-auto rounded-lg border border-line">
            {shown.map((r) => (
              <li key={r}>
                <label className="flex cursor-pointer items-center gap-3 px-3 py-2 hover:bg-hover">
                  <input
                    type="checkbox"
                    checked={picked.includes(r)}
                    onChange={(e) => toggle(r, e.target.checked)}
                    className="size-4 shrink-0 accent-[var(--c-accent)]"
                  />
                  <span className="min-w-0 font-mono text-[13px] text-ink [overflow-wrap:anywhere]">{r}</span>
                </label>
              </li>
            ))}
            {shown.length === 0 && (
              <li className="px-3 py-2 text-[13px] text-ink-4">No repository matches.</li>
            )}
          </ul>
          <p className="text-[12.5px] text-ink-3">
            {picked.length} of {offer.repos.length} chosen
            {picked.length > MAX_REPOS && `: a connection names at most ${MAX_REPOS}`}
          </p>
        </div>
      )}
      {offer.others > 0 && (
        <p className="mt-3 max-w-2xl text-[12.5px] text-ink-3">
          {offer.others === 1
            ? "It also reaches a repository there that you aren't an admin of: its admins can connect it."
            : `It also reaches ${offer.others} repositories there that you aren't an admin of: their admins can connect them.`}
        </p>
      )}
      {choose.error && (
        <p role="alert" className="mt-3 text-[12.5px] text-danger [overflow-wrap:anywhere]">
          {choose.error.message}
        </p>
      )}
      <div className="mt-4 flex flex-wrap gap-2">
        <Button
          variant="primary"
          icon={<Check className="size-3.5" />}
          loading={choose.isPending}
          disabled={!ready || dismiss.isPending}
          onClick={() => choose.mutate()}
        >
          Connect {offer.account}
        </Button>
        <Button variant="ghost" loading={dismiss.isPending} onClick={() => dismiss.mutate()}>
          Not now
        </Button>
      </div>
    </section>
  );
}

function Github({
  w,
  returned,
  account,
  reason,
}: {
  w: G.Workspace;
  returned?: GithubOutcome;
  account?: string;
  reason?: GithubRefusal;
}) {
  const admin = w.role === "owner" || w.role === "admin";
  const navigate = useNavigate();
  const gh = useQuery({ queryKey: keys.workspaceGithub(w.id), queryFn: () => workspaces.github(w.id) });
  // A GitHub sign-in for the account, or for every one GitHub shows the admin the app on.
  const connect = useMutation({
    mutationFn: (account?: string) => workspaces.connectGithub(w.id, { account }),
    // On to GitHub, which brings the browser back here.
    onSuccess: (link) => window.location.assign(link.url),
    onError: (e) => toast(e.message, "error"),
  });
  const signingIn = (account?: string) =>
    (connect.isPending || connect.isSuccess) && connect.variables === account;
  const [dropping, setDropping] = useState<G.GithubConnection | null>(null);
  const disconnect = useMutation({
    mutationFn: (a: string) => workspaces.disconnectGithub(w.id, a),
    onSuccess: (g) => {
      queryClient.setQueryData(keys.workspaceGithub(w.id), g);
      setDropping(null);
      refresh();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const chosen = (account: string) => (g: G.WorkspaceGithub, connected: boolean) => {
    queryClient.setQueryData(keys.workspaceGithub(w.id), g);
    if (connected) refresh();
    void navigate({
      to: "/w/$slug",
      params: { slug: w.slug },
      search: connected ? { tab: "github", github: "connected", account } : { tab: "github" },
      replace: true,
    });
  };
  const g = gh.data;
  if (g && !g.configured)
    return (
      <Banner tone="info">
        This instance has no GitHub App, so projects can't link repositories yet. Its admins set one up (see
        the self-hosting guide).
      </Banner>
    );
  const app = g?.app_slug ?? "GitHub";
  const connection = (account?: string) =>
    g?.connections.find((c) => c.account.toLowerCase() === account?.toLowerCase());
  const offers = admin ? (g?.offers ?? []) : [];
  return (
    <div>
      {returned && g && (
        <div className="mb-6">
          <Returned
            w={w}
            outcome={returned}
            account={account}
            reason={reason}
            connection={connection(account)}
            choosing={offers.length > 0}
            app={app}
          />
        </div>
      )}
      {offers.length > 0 && (
        <div className="mb-8 flex flex-col gap-4">
          {offers.map((o) => (
            <Choice
              key={o.account}
              w={w}
              offer={o}
              connection={connection(o.account)}
              app={app}
              onDone={chosen(o.account)}
            />
          ))}
        </div>
      )}
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        {w.name}'s projects use repositories of the GitHub accounts it works in: ones the{" "}
        <span className="font-mono text-[13px]">{app}</span> app is installed on. Agents get tokens for the
        project's repositories only, for an hour at a time.
      </p>
      <ul className="mt-6 border-t border-line">
        {g?.connections.map((c) => (
          <li key={c.account} className="flex items-center gap-3.5 border-b border-line py-3.5">
            <GithubAvatar account={c.account} />
            <div className="min-w-0 flex-1">
              <a
                href={`https://github.com/${encodeURIComponent(c.account)}`}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1.5 text-[14px] text-ink hover:underline"
              >
                {c.account} <ExternalLink className="size-3 text-ink-4" />
              </a>
              <div className="mt-0.5 text-[12px] text-ink-4">
                {c.repos ? (
                  <>
                    Only <span className="font-mono">{someRepos(c.repos)}</span>
                  </>
                ) : (
                  "Every repository the app reaches there"
                )}
                {c.github_user &&
                  ` · connected by ${c.requested_by?.name ?? "an admin"} as @${c.github_user} on GitHub`}
              </div>
            </div>
            {c.status === "active" ? (
              <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
                <Check className="size-3.5" /> Connected
              </span>
            ) : (
              <span className="text-[12.5px] text-ink-4">Not connected</span>
            )}
            {admin && g.can_connect && (
              <button
                type="button"
                aria-label={`Choose which repositories of ${c.account} ${w.name} works in`}
                title="Choose repositories"
                className="rounded p-1.5 text-ink-4 hover:text-ink disabled:opacity-50"
                disabled={connect.isPending || connect.isSuccess}
                onClick={() => connect.mutate(c.account)}
              >
                {signingIn(c.account) ? <Spinner className="size-3.5" /> : <Pencil className="size-3.5" />}
              </button>
            )}
            {admin && (
              <button
                type="button"
                aria-label={`Disconnect ${c.account}`}
                className="rounded p-1.5 text-ink-4 hover:text-danger"
                onClick={() => (c.status === "active" ? setDropping(c) : disconnect.mutate(c.account))}
              >
                <Trash2 className="size-3.5" />
              </button>
            )}
          </li>
        ))}
        {g && g.connections.length === 0 && (
          <li className="py-6 text-[13.5px] text-ink-4">{w.name} works in no GitHub account yet.</li>
        )}
      </ul>
      {admin && g && !g.can_connect && (
        <Banner tone="info" className="mt-8">
          Connecting an account takes signing in to GitHub, which this instance isn't set up for. Its admins
          add the GitHub App's client secret (see the self-hosting guide).
        </Banner>
      )}
      {admin && g?.can_connect && (
        <>
          <p className="mt-10 max-w-2xl text-[13px] leading-relaxed text-ink-3">
            Connecting an account takes signing in to GitHub, which shows what you may give {w.name}: every
            repository of your own account, or of an organization whose every repository the app reaches and
            you're an admin of; otherwise the repositories you're an admin of. Then you choose from them here.
          </p>
          <div className="mt-6 grid gap-8 md:grid-cols-2">
            <div>
              <h3 className="text-[13.5px] font-semibold text-ink">Install the app on an account</h3>
              <p className="mt-1 text-[13px] leading-relaxed text-ink-3">
                On your own account or an organization you own, or give it more repositories where it's
                installed. GitHub brings you back here to choose what {w.name} works in.
              </p>
              {g.install_url && (
                <a href={g.install_url} className="mt-4 inline-block">
                  <Button icon={<GithubIcon className="size-3.5" />}>Install {app} on GitHub</Button>
                </a>
              )}
            </div>
            <div>
              <h3 className="text-[13.5px] font-semibold text-ink">Connect an account it is installed on</h3>
              <p className="mt-1 text-[13px] leading-relaxed text-ink-3">
                GitHub shows you the accounts it's installed on and what you may connect of each, and you
                choose here.
              </p>
              <Button
                className="mt-4"
                icon={<GithubIcon className="size-3.5" />}
                loading={signingIn(undefined)}
                disabled={connect.isPending || connect.isSuccess}
                onClick={() => connect.mutate(undefined)}
              >
                Connect with GitHub
              </Button>
            </div>
          </div>
        </>
      )}
      <Dialog
        open={!!dropping}
        onClose={() => setDropping(null)}
        title={`Stop working in ${dropping?.account ?? ""}?`}
        description={`${w.name}'s projects can't use its repositories any more, and running sessions lose their tokens for it within the hour.`}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDropping(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={disconnect.isPending}
            onClick={() => dropping && disconnect.mutate(dropping.account)}
          >
            Disconnect
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

// ───────────────────────────── settings ─────────────────────────────

function WorkspaceSettings({ w }: { w: G.Workspace }) {
  const admin = w.role === "owner" || w.role === "admin";
  return (
    <>
      <You w={w} />
      {admin && <General w={w} />}
      {admin && <Access w={w} />}
      {w.role === "owner" && <Primary w={w} />}
    </>
  );
}

/** Leaving a workspace, asked first; it works while the workspace is closed to you too. */
function Leave({ w }: { w: G.Workspace }) {
  const { data: me } = useMe();
  const navigate = useNavigate();
  const [leaving, setLeaving] = useState(false);
  const leave = useMutation({
    mutationFn: () => workspaces.removeMember(w.id, me?.user.id ?? ""),
    onSuccess: async () => {
      await navigate({ to: "/" });
      // Its pages are closed to you now: forget them rather than ask again.
      queryClient.removeQueries({ queryKey: ["workspace", w.id] });
      queryClient.removeQueries({ queryKey: keys.workspace(w.slug) });
      for (const k of [keys.workspaces, keys.projects, ["sessions"], keys.runners])
        void queryClient.invalidateQueries({ queryKey: k });
      toast(`You left ${w.name}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  if (w.role === "owner") return null;
  return (
    <>
      <Button
        variant="ghost"
        icon={<LogOut className="size-3.5 shrink-0" />}
        onClick={() => setLeaving(true)}
        className="max-w-full"
      >
        <span className="truncate">Leave {w.name}</span>
      </Button>
      <Dialog
        open={leaving}
        onClose={() => setLeaving(false)}
        title={`Leave ${w.name}?`}
        description="You lose access to its projects and sessions at once. Your sessions there stop, and so do its sessions on your runners, which leave it. Its admins can invite you again."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setLeaving(false)}>
            Cancel
          </Button>
          <Button variant="danger" loading={leave.isPending} onClick={() => leave.mutate()}>
            Leave
          </Button>
        </div>
      </Dialog>
    </>
  );
}

/** Your own place in the workspace: the address, the Claude connection, leaving. */
function You({ w }: { w: G.Workspace }) {
  const { data: me } = useMe();
  const membership = useMutation({
    mutationFn: (claude: string) => workspaces.updateMembership(w.id, { claude }),
    onSuccess: (x) => {
      queryClient.setQueryData(keys.workspace(w.slug), x);
      refresh();
      toast("Saved: sessions take it when they next start or resume");
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <Section title="You" description="How this workspace knows you, and what your sessions here run on.">
      <div className="flex flex-col gap-6">
        <div className="text-[13.5px] text-ink-2">
          You are {w.role === "member" ? "a member" : w.role === "admin" ? "an admin" : "the primary admin"}{" "}
          as <span className="text-ink">{w.email}</span>
          <span className="block text-[12.5px] leading-relaxed text-ink-4">
            Its people see this address. Your commits in its projects credit the commit email you chose for it
            in{" "}
            <Link
              to="/settings"
              search={{ linked: undefined, error: undefined }}
              className="underline underline-offset-2"
            >
              Settings → Email addresses
            </Link>
            .
          </span>
        </div>
        <Field
          label="Claude connection"
          hint="Your sessions in this workspace run on it: a company's seat here, your own plan elsewhere."
        >
          {(id) => (
            <Select
              id={id}
              value={w.claude ?? "default"}
              onChange={(v) => membership.mutate(v === "default" ? "" : v)}
              options={claudeOptions(me, w.claude)}
            />
          )}
        </Field>
        <div>
          <Leave w={w} />
        </div>
      </div>
    </Section>
  );
}

/** Name, address, avatar and color. */
function General({ w }: { w: G.Workspace }) {
  const navigate = useNavigate();
  const [name, setName] = useState(w.name);
  const [slug, setSlug] = useState(w.slug);
  const file = useRef<HTMLInputElement>(null);
  const save = useMutation({
    mutationFn: (b: G.UpdateWorkspaceRequest) => workspaces.update(w.id, b),
    onSuccess: (x) => {
      refresh();
      toast("Saved");
      if (x.slug !== w.slug)
        void navigate({
          to: "/w/$slug",
          params: { slug: x.slug },
          search: { tab: "settings" },
          replace: true,
        });
    },
  });
  const upload = useMutation({
    mutationFn: async (f: File) => workspaces.setAvatar(w.id, await avatarImage(f)),
    onSuccess: () => {
      refresh();
      toast("New avatar saved");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const removeImage = useMutation({
    mutationFn: () => workspaces.removeAvatar(w.id),
    onSuccess: () => refresh(),
    onError: (e) => toast(e.message, "error"),
  });
  const field = (save.error as { field?: string } | null)?.field;
  const changed = name.trim() !== w.name || slug.trim() !== w.slug;
  return (
    <Section title="General" description="What everyone sees: in the sidebar, on the home page and in lists.">
      <div className="flex flex-col gap-6">
        <div className="flex items-center gap-5">
          <WorkspaceAvatar workspace={w} size={64} />
          <div className="flex flex-col gap-2">
            <div className="flex flex-wrap gap-2">
              <Button
                size="sm"
                icon={<ImageUp className="size-3.5" />}
                loading={upload.isPending}
                onClick={() => file.current?.click()}
              >
                Upload an image
              </Button>
              {w.avatar_url && (
                <Button
                  size="sm"
                  variant="ghost"
                  loading={removeImage.isPending}
                  onClick={() => removeImage.mutate()}
                >
                  Use its initials
                </Button>
              )}
            </div>
            <p className="text-[12px] text-ink-4">
              A square picture works best: we keep its middle, 256 px a side.
            </p>
            <input
              ref={file}
              type="file"
              accept="image/png,image/jpeg,image/gif,image/webp"
              className="hidden"
              onChange={(e) => {
                const f = e.target.files?.[0];
                if (f) upload.mutate(f);
                e.target.value = "";
              }}
            />
          </div>
        </div>
        <fieldset>
          <legend className="text-[12.5px] font-medium text-ink-2">Color</legend>
          <div className="mt-2 flex gap-2">
            {(Object.keys(WORKSPACE_COLORS) as G.WorkspaceColor[]).map((c) => (
              <button
                key={c}
                type="button"
                aria-pressed={w.color === c}
                aria-label={WORKSPACE_COLORS[c].label}
                onClick={() => save.mutate({ color: c })}
                className={cn(
                  "rounded-lg p-1 transition-shadow duration-150",
                  w.color === c
                    ? "shadow-[0_0_0_2px_var(--c-accent)]"
                    : "hover:shadow-[0_0_0_1px_var(--c-line-strong)]",
                )}
              >
                <WorkspaceAvatar workspace={{ name: w.name, color: c }} size={26} title={false} />
              </button>
            ))}
          </div>
          {w.avatar_url && (
            <p className="mt-2 text-[12px] text-ink-4">
              Its image shows instead, until you use its initials again.
            </p>
          )}
        </fieldset>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate({
              name: name.trim() !== w.name ? name.trim() : undefined,
              slug: slug.trim() !== w.slug ? slug.trim() : undefined,
            });
          }}
        >
          <Field label="Name" error={field === "name" ? save.error?.message : undefined}>
            {(id) => <Input id={id} value={name} onChange={(e) => setName(e.target.value)} maxLength={80} />}
          </Field>
          <Field
            label="Address"
            error={field === "slug" ? save.error?.message : undefined}
            hint={
              <span className="font-mono text-[11.5px]">
                {location.host}/w/{slug || "…"}
              </span>
            }
          >
            {(id) => (
              <Input id={id} value={slug} onChange={(e) => setSlug(e.target.value)} className="font-mono" />
            )}
          </Field>
          <div>
            <Button type="submit" variant="primary" disabled={!changed} loading={save.isPending}>
              Save
            </Button>
          </div>
        </form>
      </div>
    </Section>
  );
}

/** Who sees which projects, and two-factor sign-in. */
function Access({ w }: { w: G.Workspace }) {
  const members = useQuery({
    queryKey: keys.workspaceMembers(w.id),
    queryFn: () => workspaces.members(w.id),
  });
  const [confirm, setConfirm] = useState(false);
  const save = useMutation({
    mutationFn: (b: G.UpdateWorkspaceRequest) => workspaces.update(w.id, b),
    onSuccess: (_, b) => {
      refresh();
      setConfirm(false);
      toast(
        b.require_two_factor === undefined
          ? "Saved"
          : b.require_two_factor
            ? `${w.name} requires two-factor sign-in`
            : `Two-factor sign-in is up to ${w.name}'s members`,
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  const without = (members.data?.members ?? []).filter((m) => !m.two_factor);
  return (
    <Section title="Access" description="What members may do in its projects, and how they sign in.">
      <div className="flex flex-col gap-7">
        <Field
          label="Base role"
          hint="Every member has it in every project, on top of the role they have as a project's member. Admins own every project."
        >
          {(id) => (
            <Select
              id={id}
              value={w.base_role}
              onChange={(v) => save.mutate({ base_role: v })}
              options={BASE_ROLES.map((r) => ({
                value: r.value,
                label: r.label,
                description: r.description,
              }))}
            />
          )}
        </Field>
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Require two-factor sign-in
            <span className="block text-[12.5px] leading-relaxed text-ink-4">
              Members who sign in with a password also need an authenticator app or a passkey, and sign-ins
              ask for it. Until they set one up, the workspace is closed to them. Joining it takes one too.
            </span>
          </span>
          <Switch
            checked={w.require_two_factor}
            disabled={save.isPending}
            onChange={(v) =>
              v && without.length > 0 ? setConfirm(true) : save.mutate({ require_two_factor: v })
            }
            label="Require two-factor sign-in"
          />
        </div>
        {members.data && (
          <div className="flex items-center gap-2.5 rounded-lg border border-line bg-sunken/40 px-4 py-3 text-[12.5px] text-ink-2">
            <ShieldCheck className="size-4 text-accent" />
            {members.data.members.length - without.length} of {members.data.members.length} members have
            two-factor sign-in
          </div>
        )}
      </div>
      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title="Require two-factor sign-in?"
        description={`These members haven't set it up yet. ${w.name} closes to them until they do; they keep their other workspaces.`}
      >
        <div className="flex flex-col gap-5">
          <ul className="flex max-h-48 flex-col gap-1.5 overflow-y-auto">
            {without.map((m) => (
              <li key={m.user.id} className="flex items-center gap-2.5 text-[13px] text-ink">
                <Avatar user={m.user} size={22} />
                <span className="truncate">{m.user.name}</span>
                <span className="truncate text-[12px] text-ink-4">{m.email}</span>
              </li>
            ))}
          </ul>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setConfirm(false)}>
              Cancel
            </Button>
            <Button
              variant="primary"
              loading={save.isPending}
              onClick={() => save.mutate({ require_two_factor: true })}
            >
              Require it
            </Button>
          </div>
        </div>
      </Dialog>
    </Section>
  );
}

/** The primary admin's own: handing the role over, and deleting the workspace. */
function Primary({ w }: { w: G.Workspace }) {
  const navigate = useNavigate();
  const members = useQuery({
    queryKey: keys.workspaceMembers(w.id),
    queryFn: () => workspaces.members(w.id),
  });
  const admins = (members.data?.members ?? []).filter((m) => m.role === "admin");
  const [to, setTo] = useState<string>();
  const [deleting, setDeleting] = useState(false);
  const [typed, setTyped] = useState("");
  const transfer = useMutation({
    mutationFn: (user: string) => workspaces.transfer(w.id, user),
    onSuccess: () => {
      refresh();
      toast("You handed the primary admin role over; you stay an admin");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: () => workspaces.remove(w.id, typed),
    onSuccess: () => {
      refresh();
      toast(`${w.name} was deleted`);
      void navigate({ to: "/" });
    },
  });
  return (
    <>
      <Section
        title="Primary admin"
        description="You. Other admins can't demote or remove you; you can hand the role to one of them."
      >
        {admins.length === 0 ? (
          <p className="text-[13.5px] text-ink-3">
            Make someone an admin first (People): the role goes to an admin.
          </p>
        ) : (
          <div className="flex items-end gap-2">
            <Field label="Hand it to" className="flex-1">
              {(id) => (
                <Select
                  id={id}
                  value={to}
                  placeholder="An admin"
                  onChange={setTo}
                  options={admins.map((m) => ({
                    value: m.user.id,
                    label: m.user.name,
                    description: m.email,
                  }))}
                />
              )}
            </Field>
            <Button disabled={!to} loading={transfer.isPending} onClick={() => to && transfer.mutate(to)}>
              Hand it over
            </Button>
          </div>
        )}
      </Section>
      <DangerZone
        title={`Delete ${w.name}`}
        description="Deletes its projects, their sessions and transcripts, for everyone in it. Its runners stay with their owners, and its GitHub accounts keep the app. This can't be undone."
        action="Delete workspace"
        onAction={() => setDeleting(true)}
      >
        <Dialog
          open={deleting}
          onClose={() => setDeleting(false)}
          title={`Delete ${w.name}?`}
          description={`${w.projects} ${w.projects === 1 ? "project" : "projects"} and every session in them go, for its ${w.members} ${w.members === 1 ? "member" : "members"}. Stop its sessions first.`}
        >
          <form
            className="flex flex-col gap-4"
            onSubmit={(e) => {
              e.preventDefault();
              remove.mutate();
            }}
          >
            <Field label={`Type ${w.name} to confirm`} error={remove.error?.message}>
              {(id) => (
                <Input id={id} value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" />
              )}
            </Field>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={() => setDeleting(false)}>
                Cancel
              </Button>
              <Button
                type="submit"
                variant="danger"
                loading={remove.isPending}
                disabled={typed.trim() !== w.name}
              >
                Delete workspace
              </Button>
            </div>
          </form>
        </Dialog>
      </DangerZone>
    </>
  );
}

// ───────────────────────────── audit ─────────────────────────────

function Audit({ w }: { w: G.Workspace }) {
  return (
    <div>
      <p className="mb-6 max-w-2xl text-[14px] leading-relaxed text-ink-3">
        What happened in {w.name}: people joining and leaving, roles, runners, GitHub accounts, projects and
        sessions. Kept for a year.
      </p>
      <AuditLog queryKey={keys.workspaceAudit(w.id)} load={(before) => workspaces.audit(w.id, before)} />
    </div>
  );
}

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
  Plus,
  ShieldCheck,
  Trash2,
} from "lucide-react";
import { type ReactNode, useRef, useState } from "react";
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
import { keys, queryClient, useMe, useProjects, useRunners, useSessions, useWorkspace } from "@/lib/queries";
import { BASE_ROLES, roleLabel } from "@/lib/workspaces";

export type WorkspaceTab = "projects" | "people" | "runners" | "github" | "settings" | "audit";

const TABS: { tab: WorkspaceTab; label: string; admin?: boolean }[] = [
  { tab: "projects", label: "Projects" },
  { tab: "people", label: "People" },
  { tab: "runners", label: "Runners" },
  { tab: "github", label: "GitHub" },
  { tab: "settings", label: "Settings" },
  { tab: "audit", label: "Audit log", admin: true },
];

/** Refetches what a change to a workspace touches. */
function refresh(id: string) {
  for (const k of [keys.workspaces, keys.workspace(id), ["workspace", id], keys.projects, keys.runners])
    void queryClient.invalidateQueries({ queryKey: k });
}

export function WorkspacePage() {
  const { slug } = useParams({ from: "/app/w/$slug" });
  const { tab = "projects", github, account } = useSearch({ from: "/app/w/$slug" });
  const ws = useWorkspace(slug);
  const w = ws.data;
  const admin = w?.role === "owner" || w?.role === "admin";
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
        <Banner tone="warn" className="mt-8">
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
            {w && tab === "github" && <Github w={w} returned={github} account={account} />}
            {w && tab === "settings" && <WorkspaceSettings w={w} />}
            {w && tab === "audit" && admin && <Audit w={w} />}
          </div>
        </>
      )}
    </div>
  );
}

// ───────────────────────────── projects ─────────────────────────────

function Projects({ w }: { w: G.Workspace }) {
  const projects = useProjects();
  const sessions = useSessions({ workspace: w.id, limit: 8 });
  const mine = (projects.data?.projects ?? []).filter((p) => p.workspace?.id === w.id);
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
                      {p.repo ? (
                        <span className="font-mono">
                          {p.repo.owner}/{p.repo.name}
                        </span>
                      ) : (
                        "No repository"
                      )}
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
                  <ArrowRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-hover:translate-x-0.5 group-hover:opacity-100" />
                </Link>
              </li>
            ))}
          </ul>
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
  const all: { done: boolean; skip?: boolean; title: string; body: string; action: ReactNode }[] = [
    {
      done: w.members > 1,
      title: "Invite the people you work with",
      body: "They join with the address you invite, and see its projects as the base role lets them.",
      action: (
        <Link to="/w/$slug" params={{ slug: w.slug }} search={{ tab: "people" }}>
          <Button size="sm" icon={<MailPlus className="size-3.5" />}>
            Invite
          </Button>
        </Link>
      ),
    },
    {
      done: w.runners > 0,
      title: "Bring a machine online",
      body: "Sessions here run only on the workspace's runners: register one, or add one of yours.",
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
      <div className="flex items-baseline justify-between">
        <h2 className="font-display text-[22px] tracking-[-0.01em] text-ink">Set up {w.name}</h2>
        <span className="text-[12px] text-ink-4 tabular-nums">
          {done} of {steps.length} done
        </span>
      </div>
      <ol className="mt-3 divide-y divide-line border-y border-line">
        {steps.map((s) => (
          <li key={s.title} className="flex gap-4 py-4">
            <span
              className={cn(
                "mt-0.5 flex size-6 shrink-0 items-center justify-center rounded-full border transition-colors",
                s.done ? "border-highlight bg-highlight text-highlight-ink" : "border-line-strong text-ink-4",
              )}
            >
              {s.done ? <Check className="size-3.5" /> : <span className="size-1.5 rounded-full bg-ink-4" />}
            </span>
            <div className="min-w-0 flex-1">
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
            {!s.done && s.action}
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
    refresh(w.id);
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
            hint="An address they sign in with. They join from the floor; Settings shows which of their addresses they joined as."
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
        description="They lose access to its projects and sessions at once. Their sessions here stop, and their project memberships and shares here go."
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
    refresh(w.id);
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
  const remove = useMutation({
    mutationFn: (runner: string) => workspaces.removeRunner(w.id, runner),
    onSuccess: done,
    onError: (e) => toast(e.message, "error"),
  });
  const here = new Set(list.data?.runners.map((r) => r.id));
  const yours = (all.data?.runners ?? []).filter((r) => r.yours && !here.has(r.id));
  const site = location.hostname.startsWith("app.")
    ? `https://${location.hostname.slice(4)}`
    : location.origin;
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
                      description: r.workspaces.length
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
        <Empty icon={<Cpu className="size-6" />} title="No runners yet">
          {admin
            ? "Register a machine above, then run two commands on it."
            : "Its admins add the machines its sessions run on."}
        </Empty>
      ) : (
        <ul className="mt-6 border-t border-line">
          {list.data?.runners.map((r) => {
            const others = r.workspaces.filter((x) => x.id !== w.id);
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
                  <Button
                    size="sm"
                    variant="ghost"
                    loading={remove.isPending && remove.variables === r.id}
                    onClick={() => remove.mutate(r.id)}
                  >
                    Take out
                  </Button>
                )}
              </li>
            );
          })}
        </ul>
      )}
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
            <CodeBox value={`curl -fsSL ${site}/install | sh`} />
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

// ───────────────────────────── GitHub ─────────────────────────────

function Github({
  w,
  returned,
  account,
}: {
  w: G.Workspace;
  returned?: "connected" | "requested" | "asked_owner";
  account?: string;
}) {
  const admin = w.role === "owner" || w.role === "admin";
  const gh = useQuery({ queryKey: keys.workspaceGithub(w.id), queryFn: () => workspaces.github(w.id) });
  const [name, setName] = useState("");
  const [scope, setScope] = useState("");
  const connect = useMutation({
    mutationFn: () => {
      const repos = scope
        .split(/[\s,]+/)
        .map((r) => r.trim())
        .filter(Boolean);
      return workspaces.connectGithub(w.id, {
        account: name.trim(),
        repos: repos.length ? repos : undefined,
      });
    },
    onSuccess: (g) => {
      queryClient.setQueryData(keys.workspaceGithub(w.id), g);
      refresh(w.id);
      const c = g.connections.find((x) => x.account.toLowerCase() === name.trim().toLowerCase());
      toast(
        c?.status === "active"
          ? `${w.name} works in ${c.account}`
          : `Asked an instance admin to let ${w.name} work in ${name.trim()}`,
      );
      setName("");
      setScope("");
    },
  });
  const disconnect = useMutation({
    mutationFn: (a: string) => workspaces.disconnectGithub(w.id, a),
    onSuccess: (g) => {
      queryClient.setQueryData(keys.workspaceGithub(w.id), g);
      refresh(w.id);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const g = gh.data;
  if (g && !g.configured)
    return (
      <Banner tone="info">
        This instance has no GitHub App, so projects can't link repositories yet. Its admins set one up (see
        the self-hosting guide).
      </Banner>
    );
  return (
    <div>
      {returned === "connected" && (
        <Banner tone="ok" className="mb-6">
          Installed on {account}, and {w.name} works in it now.
        </Banner>
      )}
      {returned === "requested" && (
        <Banner tone="ok" className="mb-6">
          Installed on {account}. An instance admin approves {w.name}'s request to work in it; it shows as
          connected here then.
        </Banner>
      )}
      {returned === "asked_owner" && (
        <Banner tone="info" className="mb-6">
          GitHub asked the organization's owners to install the app. Once they have, connect the organization
          here.
        </Banner>
      )}
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        {w.name}'s projects use repositories of the GitHub accounts it works in: ones the{" "}
        <span className="font-mono text-[13px]">{g?.app_slug ?? "GitHub"}</span> app is installed on. Agents
        get tokens for the project's repositories only, for an hour at a time.
      </p>
      <ul className="mt-6 border-t border-line">
        {g?.connections.map((c) => (
          <li key={c.account} className="flex items-center gap-3.5 border-b border-line py-3.5">
            <img
              src={`https://github.com/${encodeURIComponent(c.account)}.png?size=64`}
              alt=""
              className="size-8 shrink-0 rounded-md bg-raised"
              referrerPolicy="no-referrer"
            />
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
                    Only{" "}
                    <span className="font-mono">
                      {c.repos.slice(0, 4).join(", ")}
                      {c.repos.length > 4 && ` and ${c.repos.length - 4} more`}
                    </span>
                  </>
                ) : (
                  "Every repository the app reaches there"
                )}
                {c.requested_by && ` · asked by ${c.requested_by.name}`}
              </div>
            </div>
            {c.status === "active" ? (
              <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
                <Check className="size-3.5" /> Connected
              </span>
            ) : (
              <span className="text-[12.5px] text-amber">Waiting for an instance admin</span>
            )}
            {admin && (
              <button
                type="button"
                aria-label={`Disconnect ${c.account}`}
                className="rounded p-1.5 text-ink-4 hover:text-danger"
                onClick={() => disconnect.mutate(c.account)}
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
      {admin && g && (
        <div className="mt-10 grid gap-8 md:grid-cols-2">
          <div>
            <h3 className="text-[13.5px] font-semibold text-ink">Install the app on an account</h3>
            <p className="mt-1 text-[13px] leading-relaxed text-ink-3">
              On your own account, or an organization you own. GitHub brings you back here, and the account is
              connected{g.approval_required ? " once an instance admin approves it" : ""}.
            </p>
            {g.install_url && (
              <a href={g.install_url} className="mt-4 inline-block">
                <Button icon={<GithubIcon className="size-3.5" />}>Install {g.app_slug} on GitHub</Button>
              </a>
            )}
          </div>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              if (name.trim()) connect.mutate();
            }}
            className="flex flex-col gap-3"
          >
            <h3 className="text-[13.5px] font-semibold text-ink">Connect an account it is installed on</h3>
            <Field label="Account" error={connect.error?.message}>
              {(id) => (
                <Input
                  id={id}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="hyperstructure"
                />
              )}
            </Field>
            <Field
              label="Only these repositories"
              hint="Optional: names, separated by commas. Empty: every one."
            >
              {(id) => (
                <Input
                  id={id}
                  value={scope}
                  onChange={(e) => setScope(e.target.value)}
                  placeholder="web, design"
                />
              )}
            </Field>
            <div>
              <Button type="submit" loading={connect.isPending} disabled={!name.trim()}>
                {g.approval_required ? "Ask an instance admin" : "Connect"}
              </Button>
            </div>
          </form>
        </div>
      )}
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

/** Your own place in the workspace: the address, the Claude connection, leaving. */
function You({ w }: { w: G.Workspace }) {
  const { data: me } = useMe();
  const navigate = useNavigate();
  const [leaving, setLeaving] = useState(false);
  const membership = useMutation({
    mutationFn: (claude: string) => workspaces.updateMembership(w.id, { claude }),
    onSuccess: (x) => {
      queryClient.setQueryData(keys.workspace(w.slug), x);
      refresh(w.id);
      toast("Saved: sessions take it when they next start or resume");
    },
    onError: (e) => toast(e.message, "error"),
  });
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
  const creds = (me?.claude ?? []).filter((c) => c.status === "active");
  const fallback = me?.claude.find((c) => c.is_default);
  const name = (c: G.ClaudeCredentialInfo) => c.account_email ?? c.label ?? "Claude";
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
              options={[
                {
                  value: "default",
                  label: fallback ? `Your default (${name(fallback)})` : "Your default",
                  description: "The one Settings → Claude marks as default",
                },
                ...creds.map((c) => ({
                  value: c.id,
                  label: name(c),
                  description: c.subscription_type ?? c.kind,
                })),
              ]}
            />
          )}
        </Field>
        {w.role !== "owner" && (
          <div>
            <Button variant="ghost" icon={<LogOut className="size-3.5" />} onClick={() => setLeaving(true)}>
              Leave {w.name}
            </Button>
          </div>
        )}
      </div>
      <Dialog
        open={leaving}
        onClose={() => setLeaving(false)}
        title={`Leave ${w.name}?`}
        description="You lose access to its projects and sessions at once, and your sessions there stop. Its admins can invite you again."
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
      refresh(w.id);
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
      refresh(w.id);
      toast("New avatar saved");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const removeImage = useMutation({
    mutationFn: () => workspaces.removeAvatar(w.id),
    onSuccess: () => refresh(w.id),
    onError: (e) => toast(e.message, "error"),
  });
  const field = (save.error as { field?: string } | null)?.field;
  const changed = name.trim() !== w.name || slug.trim() !== w.slug;
  return (
    <Section title="General" description="What everyone sees: in the sidebar, on the floor and in lists.">
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
      refresh(w.id);
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
      refresh(w.id);
      toast("You handed the primary admin role over; you stay an admin");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: () => workspaces.remove(w.id, typed),
    onSuccess: () => {
      refresh(w.id);
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
  const [pages, setPages] = useState<G.AuditList[]>([]);
  const first = useQuery({ queryKey: keys.workspaceAudit(w.id), queryFn: () => workspaces.audit(w.id) });
  const more = useMutation({
    mutationFn: (before: number) => workspaces.audit(w.id, before),
    onSuccess: (p) => setPages((x) => [...x, p]),
  });
  const all = [first.data, ...pages].filter((p): p is G.AuditList => !!p);
  const entries = all.flatMap((p) => p.entries);
  const next = all[all.length - 1]?.next_before;
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        What happened in {w.name}: people joining and leaving, roles, runners, GitHub accounts, projects and
        sessions. Kept for a year.
      </p>
      <ul className="mt-6 border-t border-line">
        {entries.map((e) => (
          <li key={e.id} className="flex items-baseline gap-3 border-b border-line py-2.5 text-[13px]">
            <span className="w-24 shrink-0 text-[12px] text-ink-4">{ago(e.ts)}</span>
            <span className="w-40 shrink-0 truncate text-ink-2">{e.actor?.name ?? "system"}</span>
            <span className="font-mono text-[12.5px] text-ink">{e.action}</span>
            <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink-4">{e.target}</span>
          </li>
        ))}
        {first.data && entries.length === 0 && <li className="py-6 text-[13px] text-ink-4">Nothing yet.</li>}
      </ul>
      {next && (
        <div className="mt-4">
          <Button size="sm" variant="ghost" loading={more.isPending} onClick={() => more.mutate(next)}>
            Older
          </Button>
        </div>
      )}
    </div>
  );
}

// Admin: who is here and who gets in (and, where the instance takes them,
// who asked for early access), the workspaces, which of them work in which
// GitHub accounts, the machines doing the work, what the instance allows,
// and what happened. Instance admins see what workspaces are and who runs
// them, not what is inside the ones they aren't members of.

import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import {
  Cpu,
  Crown,
  Ellipsis,
  ExternalLink,
  Fingerprint,
  Globe,
  Inbox,
  Link2,
  Mail,
  MailPlus,
  Plus,
  ShieldCheck,
  ShieldOff,
  Trash2,
  Users as UsersIcon,
  X,
} from "lucide-react";
import { useEffect, useState } from "react";
import { AuditLog } from "@/components/audit-log";
import { GithubAvatar } from "@/components/github-avatar";
import {
  Avatar,
  AvatarStack,
  Banner,
  Button,
  CodeBox,
  CopyButton,
  Dialog,
  Empty,
  Field,
  IconButton,
  Input,
  Menu,
  MenuItem,
  Section,
  Select,
  type SelectOption,
  Switch,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { invitationMail, ordered, requestState } from "@/lib/early-access";
import { ago, cpus, day, diskBelow, lowOnDisk, memory, sandboxLimits, stamp } from "@/lib/format";
import { buildsImages, runsImages } from "@/lib/images";
import { installCommand } from "@/lib/install";
import { keys, queryClient, useInstance, useMe, useRunners, useWorkspaces } from "@/lib/queries";

type Tab = "people" | "workspaces" | "github" | "runners" | "access" | "early-access" | "settings" | "audit";

const TABS: { tab: Tab; label: string }[] = [
  { tab: "people", label: "People" },
  { tab: "workspaces", label: "Workspaces" },
  { tab: "github", label: "GitHub" },
  { tab: "runners", label: "Runners" },
  { tab: "access", label: "Access" },
  // Where the instance takes requests for early access.
  { tab: "early-access", label: "Early access" },
  { tab: "settings", label: "Settings" },
  { tab: "audit", label: "Audit log" },
];

export function Admin() {
  const { tab = "people" } = useSearch({ from: "/app/admin" });
  const early = !!useInstance().data?.features.early_access;
  const requests = useQuery({
    queryKey: keys.accessRequests,
    queryFn: api.accessRequests,
    enabled: early,
  });
  // People who wait for an invitation, on the Early access tab.
  const asking = requests.data?.waiting ?? 0;
  const { data: me } = useMe();
  if (me && me.user.role === "member")
    return (
      <div className="mx-auto max-w-xl px-6 py-24">
        <Banner tone="error">Only admins can see this page.</Banner>
      </div>
    );
  return (
    <div className="mx-auto max-w-[920px] px-6 pt-12 pb-24">
      <h1 className="font-display text-[34px] leading-tight font-medium tracking-[-0.02em]">Admin</h1>
      <nav className="-mx-6 mt-8 overflow-x-auto px-6 [scrollbar-width:none]">
        <div className="flex w-max min-w-full gap-1 border-b border-line">
          {TABS.filter((t) => t.tab !== "early-access" || early).map((t) => (
            <Link
              key={t.tab}
              to="/admin"
              search={{ tab: t.tab === "people" ? undefined : t.tab }}
              className={cn(
                "relative shrink-0 px-3 py-2.5 text-[13.5px] transition-colors",
                tab === t.tab ? "text-ink" : "text-ink-3 hover:text-ink",
              )}
            >
              {t.label}
              {t.tab === "early-access" && asking > 0 && (
                <span className="ml-1.5 inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-highlight px-1 text-[10.5px] font-semibold text-highlight-ink tabular-nums">
                  {asking}
                </span>
              )}
              {tab === t.tab && (
                <span className="absolute inset-x-2 -bottom-px h-[2px] rounded-full bg-accent" />
              )}
            </Link>
          ))}
        </div>
      </nav>
      <div className="mt-8">
        {tab === "people" && <Users />}
        {tab === "workspaces" && <Workspaces />}
        {tab === "github" && <Github />}
        {tab === "access" && <Access />}
        {tab === "early-access" && early && <EarlyAccess />}
        {tab === "runners" && <Runners />}
        {tab === "settings" && <InstanceSettings />}
        {tab === "audit" && <Audit />}
      </div>
    </div>
  );
}

function Access() {
  const list = useQuery({ queryKey: keys.allowlist, queryFn: api.allowlist });
  const { data: me } = useMe();
  const [pattern, setPattern] = useState("");
  const [admin, setAdmin] = useState(false);
  const [invite, setInvite] = useState<G.Invite | null>(null);
  const add = useMutation({
    mutationFn: () => api.allow({ pattern, role: admin ? "admin" : "member", invite: true }),
    onSuccess: (r) => {
      void list.refetch();
      void queryClient.invalidateQueries({ queryKey: keys.accessRequests });
      setPattern("");
      setInvite(r.invite ?? null);
      if (!r.invite) toast(`${r.entry.pattern} can now sign in`);
    },
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.disallow(id),
    onSuccess: (r) => {
      void list.refetch();
      void queryClient.invalidateQueries({ queryKey: keys.users });
      const what = [
        r.disabled.length > 0 && `disabled ${r.disabled.join(", ")}`,
        r.disconnected?.length && `disconnected ${r.disconnected.join(", ")} from their accounts`,
      ].filter(Boolean);
      toast(what.length ? `Removed, and ${what.join("; ")}` : "Removed");
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        Only people listed here can use this instance. Add an email to invite someone (you get a link to send
        them), or a domain like{" "}
        <code className="rounded bg-raised px-1 font-mono text-[12.5px]">*@company.com</code> to let
        colleagues sign in with Google. Removing an entry disables the accounts it admitted.
      </p>
      <form
        className="mt-6"
        onSubmit={(e) => {
          e.preventDefault();
          add.mutate();
        }}
      >
        <Field label="Email or domain" error={add.error?.message}>
          {(id) => (
            <div className="flex gap-2">
              <Input
                id={id}
                value={pattern}
                onChange={(e) => setPattern(e.target.value)}
                placeholder="ada@example.com or *@example.com"
                className="min-w-0 flex-1"
              />
              {me?.user.role === "owner" && (
                <label className="flex items-center gap-2 text-[13px] text-ink-3">
                  <input
                    type="checkbox"
                    checked={admin}
                    onChange={(e) => setAdmin(e.target.checked)}
                    className="accent-[var(--c-accent)]"
                  />{" "}
                  Admin
                </label>
              )}
              <Button
                type="submit"
                variant="primary"
                size="field"
                icon={<MailPlus className="size-3.5" />}
                loading={add.isPending}
                disabled={!pattern}
              >
                Allow
              </Button>
            </div>
          )}
        </Field>
      </form>
      <ul className="mt-6 border-t border-line">
        {list.data?.entries.map((e) => (
          <li key={e.id} className="flex items-center gap-3 border-b border-line py-3">
            <span className="text-ink-4">
              {e.pattern.startsWith("*@") ? <Globe className="size-4" /> : <MailPlus className="size-4" />}
            </span>
            <div className="min-w-0 flex-1">
              <div className="font-mono text-[13px] text-ink [overflow-wrap:anywhere]">{e.pattern}</div>
              <div className="text-[12px] text-ink-4">
                {e.role} ·{" "}
                {e.user_id ? "joined" : e.pattern.startsWith("*@") ? "anyone at this domain" : "invited"} ·
                added {ago(e.created_at)}
                {e.created_by && ` by ${e.created_by.name}`}
              </div>
            </div>
            {e.pattern !== me?.user.email && (
              <button
                type="button"
                aria-label={`Remove ${e.pattern}`}
                className="rounded p-1.5 text-ink-4 hover:text-danger"
                onClick={() => remove.mutate(e.id)}
              >
                <Trash2 className="size-3.5" />
              </button>
            )}
          </li>
        ))}
      </ul>
      <Dialog
        open={!!invite}
        onClose={() => setInvite(null)}
        title="Invitation ready"
        description={invitationSent(invite)}
      >
        {invite?.url && <CodeBox value={invite.url} />}
        <div className="mt-5 flex justify-end">
          <Button variant="primary" onClick={() => setInvite(null)}>
            Done
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

/**
 * Early access: the people who asked on the sign-in page and have no account yet. Inviting
 * one makes their link, which the row then shows, to copy or send from the admin's mail app.
 */
function EarlyAccess() {
  const list = useQuery({ queryKey: keys.accessRequests, queryFn: api.accessRequests });
  const { data: instance } = useInstance();
  const [view, setView] = useState<"waiting" | "invited">("waiting");
  /** The invitations made on this page, by address: their links, which only their answer has. */
  const [made, setMade] = useState<Record<string, G.Invite>>({});
  /** Whose invitation to make with the dialog, which also invites to workspaces. */
  const [options, setOptions] = useState<string | null>(null);
  const keep = (i: G.Invite) => setMade((m) => ({ ...m, [i.email]: i }));
  const invite = useMutation({
    mutationFn: (email: string) => api.createInvite({ email }),
    onSuccess: (i) => {
      keep(i);
      void list.refetch();
      void queryClient.invalidateQueries({ queryKey: keys.allowlist });
    },
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: (r: G.AccessRequest) => api.removeAccessRequest(r.id),
    onSuccess: (_, r) => {
      toast(`Removed ${r.email}'s request`);
      void list.refetch();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const { waiting, invited } = ordered(list.data?.requests ?? []);
  // Those invited from here stay where they were, with their links, until the page is left.
  const shown =
    view === "waiting"
      ? [...waiting, ...invited.filter((r) => made[r.email])].sort((a, b) => a.created_at - b.created_at)
      : invited;
  const page = `${location.origin}/early-access`;
  const all = (list.data?.waiting ?? 0) + (list.data?.invited ?? 0);
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        People who asked for early access on the sign-in page, longest waiting first. Invite someone to make
        their link, then send it to them{instance?.features.email ? " (we email it too)" : ""}: it works once,
        for 14 days, and sets up their account. They leave the list once they have one.
      </p>
      <div className="mt-4 flex max-w-full items-center gap-2 text-[12.5px] text-ink-4">
        <span className="shrink-0">They ask at</span>
        <code className="min-w-0 truncate font-mono text-ink-2">{page}</code>
        <CopyButton text={page} label="Copy link" className="shrink-0" />
      </div>
      <fieldset className="mt-6 flex gap-1" aria-label="Show">
        {(
          [
            ["waiting", "Waiting", list.data?.waiting],
            ["invited", "Invited", list.data?.invited],
          ] as const
        ).map(([v, label, n]) => (
          <button
            key={v}
            type="button"
            aria-pressed={view === v}
            onClick={() => setView(v)}
            className={cn(
              "inline-flex h-7 shrink-0 items-center gap-1.5 rounded-full border px-2.5 text-[12.5px] transition-colors duration-150",
              view === v
                ? "border-line-strong bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                : "border-transparent text-ink-3 hover:bg-hover hover:text-ink",
            )}
          >
            {label}
            {!!n && <span className="text-[11px] text-ink-4 tabular-nums">{n}</span>}
          </button>
        ))}
      </fieldset>
      {list.data && shown.length === 0 ? (
        view === "waiting" ? (
          <Empty title="Nobody is waiting" icon={<Inbox className="size-6" />}>
            When someone asks for early access, they show up here, longest waiting first.
          </Empty>
        ) : (
          <Empty title="Nobody invited yet" icon={<UsersIcon className="size-6" />}>
            The people you invite stay here, with whether their link still works, until they have an account.
          </Empty>
        )
      ) : (
        <ul className="mt-4 border-t border-line">
          {shown.map((r) => (
            <RequestRow
              key={r.id}
              request={r}
              invitation={made[r.email]}
              instance={instance?.name ?? "fctl"}
              inviting={invite.isPending && invite.variables === r.email}
              onInvite={() => invite.mutate(r.email)}
              onOptions={() => setOptions(r.email)}
              onRemove={() => remove.mutate(r)}
            />
          ))}
        </ul>
      )}
      {list.data && list.data.requests.length < all && (
        <p className="mt-4 text-[12.5px] text-ink-4">
          The first {list.data.requests.length} of {all}: invite or remove some to see the rest.
        </p>
      )}
      <InviteDialog
        open={!!options}
        email={options ?? undefined}
        onClose={() => setOptions(null)}
        onInvited={keep}
      />
    </div>
  );
}

function RequestRow({
  request: r,
  invitation,
  instance,
  inviting,
  onInvite,
  onOptions,
  onRemove,
}: {
  request: G.AccessRequest;
  /** The invitation made for them on this page. */
  invitation?: G.Invite;
  instance: string;
  inviting: boolean;
  onInvite: () => void;
  onOptions: () => void;
  onRemove: () => void;
}) {
  const state = invitation ? "invited" : requestState(r);
  // Asking again within the hour isn't news.
  const again = r.asked_at - r.created_at > 3_600_000 ? ` · asked again ${ago(r.asked_at)}` : "";
  const by = r.invited_by ? ` by ${r.invited_by.name}` : "";
  const expires = invitation?.expires_at ?? r.invite_expires_at;
  return (
    <li className="flex flex-col gap-3 border-b border-line py-3">
      <div className="flex items-center gap-3">
        <Avatar user={{ id: r.email, name: r.email }} size={30} />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[14px] text-ink">{r.email}</div>
          <div className="text-[12px] text-ink-4">
            {state === "waiting" ? (
              <span title={stamp(r.created_at)}>
                Asked {ago(r.created_at)}
                {again}
              </span>
            ) : (
              <>
                <span title={r.invited_at ? stamp(r.invited_at) : undefined}>
                  Invited {invitation ? "just now" : `${ago(r.invited_at)}${by}`}
                </span>
                {/* A link made here says itself how long it works. */}
                {!invitation &&
                  (state === "invited" && expires ? (
                    ` · the link works until ${day(expires)}`
                  ) : (
                    <>
                      {" · "}
                      <span className="text-amber">the link expired</span>
                    </>
                  ))}
              </>
            )}
          </div>
        </div>
        <div className="flex shrink-0 items-center gap-1">
          {!invitation && (
            <Button
              size="sm"
              variant={state === "invited" ? "ghost" : "secondary"}
              icon={state === "waiting" ? <MailPlus className="size-3.5" /> : <Link2 className="size-3.5" />}
              loading={inviting}
              onClick={onInvite}
              title={state === "invited" ? "Their first link works too, until it expires" : undefined}
            >
              {state === "waiting" ? "Invite" : state === "invited" ? "New link" : "Invite again"}
            </Button>
          )}
          <Menu
            trigger={(toggle, open) => (
              <IconButton
                label={`More for ${r.email}`}
                aria-haspopup="menu"
                aria-expanded={open}
                onClick={toggle}
                className="size-7"
              >
                <Ellipsis className="size-4" />
              </IconButton>
            )}
          >
            {(close) => (
              <>
                {state === "waiting" && (
                  <MenuItem
                    icon={<MailPlus />}
                    onClick={() => {
                      close();
                      onOptions();
                    }}
                  >
                    Invite to workspaces too…
                  </MenuItem>
                )}
                <MenuItem
                  danger
                  icon={<Trash2 />}
                  onClick={() => {
                    close();
                    onRemove();
                  }}
                >
                  Remove the request
                </MenuItem>
              </>
            )}
          </Menu>
        </div>
      </div>
      {invitation && !invitation.url && (
        <p className="animate-rise text-[12px] text-ink-4 sm:pl-[42px]">{invitationSent(invitation)}</p>
      )}
      {invitation?.url && (
        <div className="flex animate-rise flex-col gap-2 sm:pl-[42px]">
          <CodeBox value={invitation.url} />
          <div className="flex flex-wrap items-center gap-x-3 gap-y-2 text-[12px] text-ink-4">
            <a
              href={invitationMail({
                email: r.email,
                url: invitation.url,
                instance,
                expiresAt: invitation.expires_at,
              })}
              className="inline-flex h-7 items-center gap-1.5 rounded-sm border border-line-strong bg-raised px-2.5 text-[12.5px] font-medium text-ink transition-colors hover:border-ink-4"
            >
              <Mail className="size-3.5" /> Email it to them
            </a>
            <span>
              {invitation.emailed ? "We emailed it to them too. " : ""}It works once, until{" "}
              {day(invitation.expires_at)}.
            </span>
          </div>
        </div>
      )}
    </li>
  );
}

/**
 * What happened to an invitation's link. Where the instance sends email, it goes to the address alone: whoever holds
 * it joins the workspaces that invited the address.
 */
function invitationSent(invite: G.Invite | null | undefined): string {
  if (!invite) return "";
  if (invite.url)
    return invite.emailed
      ? `We emailed ${invite.email}. You can also send them this link.`
      : `Send this link to ${invite.email}. It works once, for 14 days.`;
  return invite.emailed
    ? `We emailed ${invite.email} the link. It works once, for 14 days.`
    : `The invitation couldn't be emailed to ${invite.email}. Invite them again in a while.`;
}

/** Instance roles: the primary instance owner hands their own role over instead. */
const instanceRoles = (actor?: G.InstanceRole): SelectOption<G.InstanceRole>[] => [
  { value: "member", label: "Member", description: "Uses the workspaces they are members of" },
  {
    value: "admin",
    label: "Admin",
    description: "Also manages people, invitations, runners, GitHub and settings",
    disabled: actor !== "owner",
  },
];

/** Whether a user has what requiring two-factor asks for: an authenticator app or a passkey. */
const secured = (u: G.User) => u.two_factor || u.passkeys > 0;

/** Whether the instance requires two-factor of a user. */
const requiredOf = (u: G.User, s?: G.InstanceSettings) =>
  !!s?.require_two_factor && (u.has_password || s.two_factor_google);

/** Whether an admin's reset removes a user's passkeys too: where they are the required second factor. */
const resetsPasskeys = (u: G.User, s?: G.InstanceSettings) => requiredOf(u, s) && u.passkeys > 0;

/** Whether the instance's settings require two-factor of a user who doesn't have it. */
function lacksTwoFactor(u: G.User, s?: G.InstanceSettings) {
  return (
    !!s?.require_two_factor && !secured(u) && u.status === "active" && (u.has_password || s.two_factor_google)
  );
}

function Users() {
  const { data: me } = useMe();
  const users = useQuery({ queryKey: keys.users, queryFn: api.users });
  const [inviting, setInviting] = useState(false);
  const [primary, setPrimary] = useState<G.User | null>(null);
  const transfer = useMutation({
    mutationFn: (u: G.User) => api.transferInstance(u.id),
    onSuccess: (u) => {
      setPrimary(null);
      void users.refetch();
      void queryClient.invalidateQueries({ queryKey: keys.me });
      toast(`${u.name} is the primary instance owner; you are an admin now`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const settings = useQuery({ queryKey: keys.instanceSettings, queryFn: api.instanceSettings });
  const [reset, setReset] = useState<string | null>(null);
  const [resetTwoFactor, setResetTwoFactor] = useState<G.User | null>(null);
  const forget = useMutation({
    mutationFn: (id: string) => api.resetTwoFactor(id),
    onSuccess: (u) => {
      setResetTwoFactor(null);
      void users.refetch();
      toast(`${u.name}'s two-factor sign-in is off`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const update = useMutation({
    mutationFn: (v: { id: string; body: G.UpdateUserRequest }) => api.updateUser(v.id, v.body),
    onSuccess: () => users.refetch(),
    onError: (e) => toast(e.message, "error"),
  });
  const link = useMutation({
    mutationFn: (u: G.User) => api.resetLink(u.id),
    // Where the instance sends email, the link goes to them alone.
    onSuccess: (r, u) =>
      r.url
        ? setReset(r.url)
        : toast(`We emailed ${u.name} a link to reset their password. It works for an hour.`),
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <div>
      <div className="mb-6 flex items-start justify-between gap-6">
        <p className="max-w-xl text-[14px] leading-relaxed text-ink-3">
          Everyone with an account on this instance. Each address is invited on its own; people connect the
          addresses they have to one account, and join workspaces as one of them.
        </p>
        <Button variant="primary" icon={<MailPlus className="size-3.5" />} onClick={() => setInviting(true)}>
          Invite
        </Button>
      </div>
      <InviteDialog open={inviting} onClose={() => setInviting(false)} />
      <ul className="border-t border-line">
        {users.data?.users.map((u) => (
          <li
            key={u.id}
            className={cn(
              "flex items-center gap-3 border-b border-line py-3",
              u.status === "disabled" && "opacity-60",
            )}
          >
            <Avatar user={u} size={30} />
            {/* On a phone the actions go under the person's details. */}
            <div className="flex min-w-0 flex-1 flex-col gap-2 sm:flex-row sm:items-center sm:gap-3">
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-1.5 text-[14px] text-ink">
                  <span className="truncate">{u.name}</span>
                  {u.two_factor && (
                    <span title="Two-factor sign-in is on" className="text-accent">
                      <ShieldCheck className="size-3.5" aria-label="Two-factor sign-in is on" />
                    </span>
                  )}
                  {u.passkeys > 0 && (
                    <span
                      title={
                        u.passkeys === 1 ? "Signs in with a passkey" : `Signs in with ${u.passkeys} passkeys`
                      }
                      className="text-accent"
                    >
                      <Fingerprint className="size-3.5" aria-label={`${u.passkeys} passkeys`} />
                    </span>
                  )}
                  {u.id === me?.user.id && <span className="text-[12px] text-ink-4">you</span>}
                </div>
                <div className="truncate text-[12px] text-ink-4">
                  {u.email}
                  {u.emails.length > 0 && ` (also ${u.emails.join(", ")})`} ·{" "}
                  {u.workspaces === 0
                    ? "in no workspace"
                    : `in ${u.workspaces} ${u.workspaces === 1 ? "workspace" : "workspaces"}`}{" "}
                  · {u.last_active_at ? `active ${ago(u.last_active_at)}` : "never signed in"}
                  {lacksTwoFactor(u, settings.data) && (
                    <span className="text-amber"> · sets up two-factor before anything else</span>
                  )}
                </div>
              </div>
              <div className="-ml-1 flex shrink-0 flex-wrap items-center gap-1 sm:ml-0">
                {u.role === "owner" ? (
                  <span className="inline-flex items-center gap-1.5 px-2 text-[12.5px] text-ink-2">
                    <Crown className="size-3.5 text-accent" /> Primary instance owner
                  </span>
                ) : (
                  <Select
                    size="sm"
                    aria-label={`Role of ${u.name}`}
                    value={u.role}
                    disabled={u.id === me?.user.id || (u.role === "admin" && me?.user.role !== "owner")}
                    onChange={(role) => update.mutate({ id: u.id, body: { role } })}
                    options={instanceRoles(me?.user.role)}
                  />
                )}
                {me?.user.role === "owner" && u.role === "admin" && u.status === "active" && (
                  <Button size="sm" variant="ghost" onClick={() => setPrimary(u)}>
                    Make primary instance owner
                  </Button>
                )}
                {u.id !== me?.user.id &&
                  u.role !== "owner" &&
                  // Admins' accounts are the primary instance owner's to manage.
                  (u.role !== "admin" || me?.user.role === "owner") && (
                    <>
                      <Button
                        size="sm"
                        variant="ghost"
                        icon={<Link2 className="size-3.5" />}
                        onClick={() => link.mutate(u)}
                      >
                        Reset link
                      </Button>
                      {(u.two_factor || resetsPasskeys(u, settings.data)) && (
                        <Button
                          size="sm"
                          variant="ghost"
                          icon={<ShieldOff className="size-3.5" />}
                          onClick={() => setResetTwoFactor(u)}
                        >
                          Reset two-factor
                        </Button>
                      )}
                      <Button
                        size="sm"
                        variant={u.status === "active" ? "ghost" : "secondary"}
                        onClick={() =>
                          update.mutate({
                            id: u.id,
                            body: { status: u.status === "active" ? "disabled" : "active" },
                          })
                        }
                      >
                        {u.status === "active" ? "Disable" : "Enable"}
                      </Button>
                    </>
                  )}
              </div>
            </div>
          </li>
        ))}
      </ul>
      <Dialog
        open={!!primary}
        onClose={() => setPrimary(null)}
        title={`Make ${primary?.name ?? ""} the primary instance owner?`}
        description="The instance has one primary owner, whom no admin can demote, disable or remove, and who manages its admins. You become an admin, and only they can hand the role back."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setPrimary(null)}>
            Cancel
          </Button>
          <Button
            variant="primary"
            loading={transfer.isPending}
            onClick={() => primary && transfer.mutate(primary)}
          >
            Hand it over
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={!!reset}
        onClose={() => setReset(null)}
        title="Password reset link"
        description="Valid for one hour. Whoever opens it can set a new password, so send it privately."
      >
        {reset && <CodeBox value={reset} />}
      </Dialog>
      <Dialog
        open={!!resetTwoFactor}
        onClose={() => setResetTwoFactor(null)}
        title={`Reset ${resetTwoFactor?.name ?? "their"} two-factor sign-in?`}
        description="For when they lost their phone and their recovery codes. Make sure it's them asking: this undoes what protects their account if their password leaks."
      >
        <div className="flex flex-col gap-5">
          <p className="text-[13.5px] leading-relaxed text-ink-2">
            {resetTwoFactor && resetsPasskeys(resetTwoFactor, settings.data)
              ? "Their authenticator app's codes, recovery codes and passkeys stop working"
              : "Their authenticator app's codes and recovery codes stop working"}
            , and they are emailed about it when email is set up.{" "}
            {settings.data?.require_two_factor
              ? "This instance requires two-factor, so they set it up again before they can do anything else."
              : "Their password alone signs them in until they set it up again."}
          </p>
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setResetTwoFactor(null)}>
              Cancel
            </Button>
            <Button
              variant="danger"
              loading={forget.isPending}
              onClick={() => resetTwoFactor && forget.mutate(resetTwoFactor.id)}
            >
              Reset two-factor
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}

/** Inviting someone to the instance, and to workspaces the inviter is an admin of. */
function InviteDialog({
  open,
  onClose,
  email: asked,
  onInvited,
}: {
  open: boolean;
  onClose: () => void;
  /** Their address, when they asked for early access. */
  email?: string;
  onInvited?: (invite: G.Invite) => void;
}) {
  const { data: me } = useMe();
  const mine = (useWorkspaces().data?.workspaces ?? []).filter(
    (w) => !w.blocked && (w.role === "owner" || w.role === "admin"),
  );
  const [email, setEmail] = useState("");
  useEffect(() => {
    if (open && asked) setEmail(asked);
  }, [open, asked]);
  const [admin, setAdmin] = useState(false);
  const [picked, setPicked] = useState<Record<string, G.WorkspaceRole | undefined>>({});
  const [done, setDone] = useState<G.Invite | null>(null);
  const reset = () => {
    setEmail("");
    setAdmin(false);
    setPicked({});
    setDone(null);
    invite.reset();
  };
  const invite = useMutation({
    mutationFn: () =>
      api.createInvite({
        email: email.trim(),
        role: admin ? "admin" : "member",
        workspaces: Object.entries(picked)
          .filter((e): e is [string, G.WorkspaceRole] => !!e[1])
          .map(([workspace, role]) => ({ workspace, role })),
      }),
    onSuccess: (i) => {
      setDone(i);
      onInvited?.(i);
      void queryClient.invalidateQueries({ queryKey: keys.allowlist });
      void queryClient.invalidateQueries({ queryKey: keys.accessRequests });
    },
  });
  const close = () => {
    onClose();
    setTimeout(reset, 200);
  };
  return (
    <Dialog
      open={open}
      onClose={close}
      title={done ? "Invitation ready" : "Invite someone"}
      description={
        done
          ? invitationSent(done)
          : "They create an account with the link, or connect the address to the account they have."
      }
    >
      {done ? (
        <div className="flex flex-col gap-4">
          {done.url && <CodeBox value={done.url} />}
          {done.workspaces.length > 0 && (
            <div className="flex flex-wrap items-center gap-2 text-[12.5px] text-ink-3">
              They join
              {done.workspaces.map((w) => (
                <span key={w.id} className="inline-flex items-center gap-1.5 text-ink-2">
                  <WorkspaceAvatar workspace={w} size={16} /> {w.name}
                </span>
              ))}
              with it.
            </div>
          )}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={reset}>
              Invite someone else
            </Button>
            <Button variant="primary" onClick={close}>
              Done
            </Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-5"
          onSubmit={(e) => {
            e.preventDefault();
            if (email.trim()) invite.mutate();
          }}
        >
          <Field label="Email" error={invite.error?.message}>
            {(id) => (
              <Input
                id={id}
                type="email"
                autoFocus
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="ada@example.com"
              />
            )}
          </Field>
          {me?.user.role === "owner" && (
            <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
              <span>
                Instance admin
                <span className="block text-[12.5px] text-ink-4">
                  Manages people, invitations, runners, GitHub and the instance's settings.
                </span>
              </span>
              <Switch checked={admin} onChange={setAdmin} label="Instance admin" />
            </div>
          )}
          {mine.length > 0 && (
            <fieldset className="flex flex-col gap-2">
              <legend className="text-[12.5px] font-medium text-ink-2">Also invite them to</legend>
              <ul className="mt-1.5 flex flex-col divide-y divide-line rounded-lg border border-line">
                {mine.map((w) => {
                  const role = picked[w.id];
                  return (
                    <li key={w.id} className="flex items-center gap-3 px-3 py-2.5">
                      <input
                        type="checkbox"
                        id={`invite-${w.id}`}
                        checked={!!role}
                        onChange={(e) =>
                          setPicked((p) => ({ ...p, [w.id]: e.target.checked ? "member" : undefined }))
                        }
                        className="accent-[var(--c-accent)]"
                      />
                      <label htmlFor={`invite-${w.id}`} className="flex min-w-0 flex-1 items-center gap-2.5">
                        <WorkspaceAvatar workspace={w} size={22} />
                        <span className="truncate text-[13.5px] text-ink">{w.name}</span>
                      </label>
                      {role && (
                        <Select
                          size="sm"
                          aria-label={`Role in ${w.name}`}
                          value={role}
                          onChange={(r) => setPicked((p) => ({ ...p, [w.id]: r }))}
                          options={[
                            { value: "member", label: "Member" },
                            { value: "admin", label: "Admin" },
                          ]}
                        />
                      )}
                    </li>
                  );
                })}
              </ul>
              <p className="text-[12px] text-ink-4">
                The workspaces you are an admin of. Without any, they start with none and create or join one.
              </p>
            </fieldset>
          )}
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
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
    </Dialog>
  );
}

/** "1 member", "2 members". */
const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** What deleting a workspace takes with it, for its confirmation. */
function deletes(w: G.AdminWorkspace): string {
  const people = w.members === 1 ? "its one member" : `its ${w.members} members`;
  const projects =
    w.projects === 0
      ? ""
      : w.projects === 1
        ? ", with its project and that project's sessions and transcripts"
        : `, with its ${w.projects} projects and their sessions and transcripts`;
  return `It goes for ${people}${projects}. This can't be undone, and its sessions must be stopped first.`;
}

/** Every workspace: what it is and who runs it, not what is inside. */
function Workspaces() {
  const list = useQuery({ queryKey: keys.adminWorkspaces, queryFn: api.adminWorkspaces });
  const [deleting, setDeleting] = useState<G.AdminWorkspace | null>(null);
  const [typed, setTyped] = useState("");
  const [choosing, setChoosing] = useState<G.AdminWorkspace | null>(null);
  const [primary, setPrimary] = useState<string>();
  const remove = useMutation({
    mutationFn: (w: G.AdminWorkspace) => api.adminDeleteWorkspace(w.workspace.id, typed),
    onSuccess: (_, w) => {
      closeDelete();
      void list.refetch();
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      // Its GitHub accounts went with it.
      void queryClient.invalidateQueries({ queryKey: keys.adminGithub });
      toast(`${w.workspace.name} was deleted`);
    },
  });
  const assign = useMutation({
    mutationFn: (v: { workspace: G.AdminWorkspace; user: string }) =>
      api.adminWorkspacePrimary(v.workspace.workspace.id, v.user),
    onSuccess: (_, v) => {
      const who = v.workspace.candidates.find((c) => c.id === v.user)?.name ?? "They";
      setChoosing(null);
      void list.refetch();
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      toast(`${who} runs ${v.workspace.workspace.name} now`);
    },
  });
  const closeDelete = () => {
    setDeleting(null);
    setTyped("");
    remove.reset();
  };
  const choose = (w: G.AdminWorkspace) => {
    assign.reset();
    setPrimary(w.candidates[0]?.id);
    setChoosing(w);
  };
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        Each workspace has its own people, admins, runners, GitHub accounts and audit log. You see inside the
        ones you are a member of; here, what each one is and who runs it.
      </p>
      <ul className="mt-6 border-t border-line">
        {list.data?.workspaces.map((w) => (
          <li key={w.workspace.id} className="flex items-center gap-3.5 border-b border-line py-3.5">
            <WorkspaceAvatar workspace={w.workspace} size={32} />
            {/* On a phone the actions go under the workspace's details. */}
            <div className="flex min-w-0 flex-1 flex-col gap-2 sm:flex-row sm:items-center sm:gap-3.5">
              <div className="min-w-0 flex-1">
                {/* Your role goes under the name where it doesn't fit beside it. */}
                <div className="flex flex-wrap items-center gap-x-2 text-[14px] text-ink">
                  <span className="truncate">{w.workspace.name}</span>
                  {w.require_two_factor && (
                    <ShieldCheck className="size-3.5 text-accent" aria-label="Requires two-factor sign-in" />
                  )}
                  {w.role && (
                    <span className="text-[12px] text-ink-4">
                      {w.role === "owner"
                        ? "you are its primary admin"
                        : w.role === "admin"
                          ? "you are an admin"
                          : "you are a member"}
                    </span>
                  )}
                </div>
                <div className="truncate text-[12px] text-ink-4">
                  {w.primary ? `Run by ${w.primary.name} · ` : ""}
                  {count(w.members, "member", "members")} · {count(w.projects, "project", "projects")} ·{" "}
                  {count(w.runners, "runner", "runners")}
                  {w.github.length > 0 && ` · ${w.github.join(", ")}`} · since {ago(w.created_at)}
                </div>
                {w.needs_primary && (
                  <div className="mt-1 text-[12px] text-amber">
                    {w.primary
                      ? `${w.primary.name}'s account is disabled, so nobody can hand its primary admin role over.`
                      : "It has no primary admin."}
                  </div>
                )}
              </div>
              <div
                className={cn(
                  "flex shrink-0 flex-wrap items-center gap-1 sm:ml-0 sm:gap-3.5",
                  // On a phone the first action lines up with the details: the button by its border,
                  // Open by its text, the delete icon by its glyph.
                  !w.needs_primary && (w.role ? "-ml-2.5" : "-ml-1.5"),
                )}
              >
                {w.needs_primary && (
                  <Button size="sm" icon={<Crown className="size-3.5" />} onClick={() => choose(w)}>
                    Choose a primary admin
                  </Button>
                )}
                {w.role && (
                  <Link to="/w/$slug" params={{ slug: w.workspace.slug }}>
                    <Button size="sm" variant="ghost">
                      Open
                    </Button>
                  </Link>
                )}
                <button
                  type="button"
                  aria-label={`Delete ${w.workspace.name}`}
                  className="rounded p-1.5 text-ink-4 hover:text-danger"
                  onClick={() => setDeleting(w)}
                >
                  <Trash2 className="size-3.5" />
                </button>
              </div>
            </div>
          </li>
        ))}
        {list.data?.workspaces.length === 0 && (
          <li className="py-6 text-[13px] text-ink-4">No workspaces yet.</li>
        )}
      </ul>
      <Dialog
        open={!!choosing}
        onClose={() => setChoosing(null)}
        title={`Who runs ${choosing?.workspace.name ?? ""} now?`}
        description="They become its primary admin: they manage its admins, can hand the role on, and can delete it. Its other admins stay admins."
      >
        {choosing && choosing.candidates.length === 0 ? (
          <div className="flex flex-col gap-4">
            <Banner tone="warn">
              None of its members has an active account. Enable one of them first, or delete the workspace.
            </Banner>
            <div className="flex justify-end">
              <Button variant="ghost" onClick={() => setChoosing(null)}>
                Close
              </Button>
            </div>
          </div>
        ) : (
          <form
            className="flex flex-col gap-4"
            onSubmit={(e) => {
              e.preventDefault();
              if (choosing && primary) assign.mutate({ workspace: choosing, user: primary });
            }}
          >
            <Field label="Its new primary admin" error={assign.error?.message}>
              {(id) => (
                <Select
                  id={id}
                  value={primary}
                  onChange={setPrimary}
                  options={(choosing?.candidates ?? []).map((c) => ({ value: c.id, label: c.name }))}
                />
              )}
            </Field>
            <div className="flex justify-end gap-2">
              <Button type="button" variant="ghost" onClick={() => setChoosing(null)}>
                Cancel
              </Button>
              <Button type="submit" variant="primary" loading={assign.isPending} disabled={!primary}>
                Make them primary admin
              </Button>
            </div>
          </form>
        )}
      </Dialog>
      <Dialog
        open={!!deleting}
        onClose={closeDelete}
        title={`Delete ${deleting?.workspace.name ?? ""}?`}
        description={deleting ? deletes(deleting) : undefined}
      >
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (deleting) remove.mutate(deleting);
          }}
        >
          <Field label={`Type ${deleting?.workspace.name ?? ""} to confirm`} error={remove.error?.message}>
            {(id) => (
              <Input id={id} value={typed} onChange={(e) => setTyped(e.target.value)} autoComplete="off" />
            )}
          </Field>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={closeDelete}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="danger"
              loading={remove.isPending}
              disabled={typed.trim() !== deleting?.workspace.name}
            >
              Delete workspace
            </Button>
          </div>
        </form>
      </Dialog>
    </div>
  );
}

/** The accounts the instance's GitHub App is installed on, and the workspaces that use them. */
function Github() {
  const list = useQuery({ queryKey: keys.adminGithub, queryFn: api.adminGithub });
  const [dropping, setDropping] = useState<{ account: string; workspace: G.WorkspaceRef } | null>(null);
  const disconnect = useMutation({
    mutationFn: (v: { account: string; workspace: string }) =>
      api.adminDisconnectGithub(v.account, v.workspace),
    onSuccess: (g) => {
      queryClient.setQueryData(keys.adminGithub, g);
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      void queryClient.invalidateQueries({ queryKey: ["workspace"] });
      setDropping(null);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const g = list.data;
  if (g && !g.configured)
    return (
      <Banner tone="info">
        This instance has no GitHub App. Set one up to let workspaces' projects use repositories (see the
        self-hosting guide).
      </Banner>
    );
  const scopeOf = (repos?: string[]) =>
    repos
      ? `Only ${repos.slice(0, 4).join(", ")}${repos.length > 4 ? ` and ${repos.length - 4} more` : ""}`
      : "Every repository";
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        The <span className="font-mono text-[13px]">{g?.app_slug}</span> app mints tokens for the accounts it
        is installed on. A workspace's admins connect the ones they may, signing in to GitHub, which says what
        they may give it: you can take an account from a workspace, and only its admins connect it again.
      </p>
      <ul className="mt-8 border-t border-line">
        {g?.installations.map((i) => (
          <li key={i.account} className="border-b border-line py-4">
            <div className="flex items-center gap-3.5">
              <GithubAvatar account={i.account} />
              <div className="min-w-0 flex-1">
                <a
                  href={`https://github.com/${encodeURIComponent(i.account)}`}
                  target="_blank"
                  rel="noreferrer"
                  className="inline-flex items-center gap-1.5 text-[14px] text-ink hover:underline"
                >
                  {i.account} <ExternalLink className="size-3 text-ink-4" />
                </a>
                <div className="text-[12px] text-ink-4">
                  {i.installation_id
                    ? `${i.account_type === "User" ? "Personal account" : "Organization"} · ${
                        i.repository_selection === "selected" ? "selected repositories" : "all repositories"
                      }${i.installed_at ? ` · installed ${ago(i.installed_at)}` : ""}`
                    : "The app isn't installed there (any more)"}
                </div>
              </div>
            </div>
            {i.workspaces.length > 0 ? (
              <ul className="mt-3 ml-11 flex flex-col gap-2.5 sm:gap-1.5">
                {i.workspaces.map((w) => (
                  <li
                    key={w.workspace.id}
                    className="grid grid-cols-[auto_minmax(0,1fr)_auto] items-center gap-x-2.5 text-[13px] text-ink-2"
                  >
                    <WorkspaceAvatar workspace={w.workspace} size={18} />
                    {/* The repositories go under the workspace's name on a phone; beside it, they
                        give way to it. */}
                    <div className="contents sm:flex sm:items-center sm:gap-2.5">
                      <span className="truncate sm:max-w-1/2 sm:shrink-0">{w.workspace.name}</span>
                      <span className="col-start-2 text-[12px] text-ink-4 sm:truncate">
                        {scopeOf(w.repos)}
                        {w.github_user && ` · by ${w.requested_by?.name ?? "an admin"} as @${w.github_user}`}
                      </span>
                    </div>
                    <button
                      type="button"
                      aria-label={`Disconnect ${w.workspace.name} from ${i.account}`}
                      className="col-start-3 row-start-1 rounded p-1 text-ink-4 hover:text-danger"
                      onClick={() => setDropping({ account: i.account, workspace: w.workspace })}
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  </li>
                ))}
              </ul>
            ) : (
              <p className="mt-1.5 ml-11 text-[12px] text-ink-4">No workspace works in it.</p>
            )}
          </li>
        ))}
        {g?.installations.length === 0 && (
          <li className="py-6 text-[13px] text-ink-4">The app isn't installed on any account yet.</li>
        )}
      </ul>
      <Dialog
        open={!!dropping}
        onClose={() => setDropping(null)}
        title={`Disconnect ${dropping?.workspace.name ?? ""} from ${dropping?.account ?? ""}?`}
        description={`Its projects can't use ${dropping?.account ?? "the account"}'s repositories any more, and its running sessions lose their tokens for it within the hour. Only its admins connect it again, signing in to GitHub.`}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDropping(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={disconnect.isPending}
            onClick={() =>
              dropping && disconnect.mutate({ account: dropping.account, workspace: dropping.workspace.id })
            }
          >
            Disconnect
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

function Runners() {
  const runners = useRunners();
  const [name, setName] = useState("");
  const [reg, setReg] = useState<G.RunnerRegistration | null>(null);
  // The workspaces you are an admin of, whose sessions a new runner may run.
  const admined = (useWorkspaces().data?.workspaces ?? []).filter(
    (w) => !w.blocked && (w.role === "owner" || w.role === "admin"),
  );
  // One of them, until you choose; without any, the runner waits for a workspace.
  const [chosen, setWorkspace] = useState<string>();
  const workspace = chosen ?? admined[0]?.id ?? "none";
  const create = useMutation({
    mutationFn: () => api.createRunner({ name, workspace: workspace === "none" ? undefined : workspace }),
    onSuccess: (r) => {
      setReg(r);
      setName("");
      void runners.refetch();
    },
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.deleteRunner(id),
    onSuccess: () => runners.refetch(),
  });
  const toggle = useMutation({
    mutationFn: (r: G.Runner) => api.updateRunner(r.id, { enabled: r.status === "disabled" }),
    onSuccess: () => runners.refetch(),
  });
  const drain = useMutation({
    mutationFn: (r: G.Runner) => api.updateRunner(r.id, { draining: !r.drain }),
    onSuccess: (r) => {
      toast(
        r.drain ? `Draining ${r.name}: no new sessions start on it` : `${r.name} takes new sessions again`,
      );
      void runners.refetch();
    },
    onError: (e) => toast(e.message, "error"),
  });
  // Disabling a runner that runs sessions, or removing one, asks first.
  const [confirm, setConfirm] = useState<{ runner: G.Runner; action: "disable" | "remove" } | null>(null);
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        Runners are the machines that run agents, every one on the instance. Each session gets its own sandbox
        — a lightweight VM with Apple's container on macOS 26, or Docker, Lima, or plain processes elsewhere —
        on a runner of its workspace. A runner belongs to whoever registered it, who adds it to the workspaces
        they run.
      </p>
      {/* The fields line up by their labels, so the hint or error under the name moves nothing.
          On a phone, the workspace goes under the name. */}
      <form
        className="mt-6 flex flex-col gap-3 sm:flex-row sm:items-start sm:gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <Field
          label="Register a machine"
          className="sm:flex-1"
          hint="A name you'll recognize, like “studio mac”."
          error={create.error?.message}
        >
          {(id) => (
            <Input id={id} value={name} onChange={(e) => setName(e.target.value)} placeholder="studio mac" />
          )}
        </Field>
        <Field label="For">
          {(id) => (
            <div className="flex gap-2">
              <Select
                id={id}
                value={workspace}
                onChange={setWorkspace}
                options={
                  admined.length > 0
                    ? admined.map((w) => ({
                        value: w.id,
                        label: w.name,
                        icon: <WorkspaceAvatar workspace={w} size={14} title={false} />,
                      }))
                    : [{ value: "none", label: "No workspace yet" }]
                }
                className="flex-1 sm:flex-none"
              />
              <Button
                type="submit"
                variant="primary"
                size="field"
                icon={<Plus className="size-3.5" />}
                loading={create.isPending}
                disabled={!name}
              >
                Register
              </Button>
            </div>
          )}
        </Field>
      </form>
      {runners.data?.runners.length === 0 ? (
        <Empty icon={<Cpu className="size-6" />} title="No machines yet">
          Register one above, then run two commands on it.
        </Empty>
      ) : (
        <ul className="mt-6 border-t border-line">
          {runners.data?.runners.map((r) => (
            <li key={r.id} className="flex items-start gap-4 border-b border-line py-4 sm:items-center">
              <span
                className={cn(
                  "mt-1.5 size-2 shrink-0 rounded-full sm:mt-0",
                  r.status === "disabled"
                    ? "bg-danger"
                    : r.drain || lowOnDisk(r)
                      ? "bg-amber"
                      : r.status === "online"
                        ? "bg-accent shadow-[0_0_10px_var(--c-accent)]"
                        : "bg-ink-4",
                )}
              />
              {/* On a phone the actions go under the runner's details. */}
              <div className="flex min-w-0 flex-1 flex-col gap-2 sm:flex-row sm:items-center sm:gap-4">
                <div className="min-w-0 flex-1">
                  <div className="text-[14px] text-ink">
                    {r.name} <span className="ml-1 text-[12px] text-ink-4">{runnerState(r)}</span>
                  </div>
                  <div className="mt-0.5 text-[12px] text-ink-4">
                    {r.hostname ? `${r.hostname} · ` : ""}
                    {r.os && r.arch ? `${r.os}/${r.arch} · ` : ""}
                    {r.driver ? `${r.driver} driver · ` : ""}
                    {r.active_sessions}/{r.capacity} sessions
                    {r.version ? ` · fctl ${r.version}` : ""}
                    {r.last_seen_at ? ` · seen ${ago(r.last_seen_at)}` : " · never connected"}
                  </div>
                  {/* Control planes before workspaces don't say which ones it runs. */}
                  {r.workspaces?.length ? (
                    <div className="mt-1 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-ink-3">
                      {r.workspaces.map((w) => (
                        <span key={w.id} className="inline-flex items-center gap-1.5">
                          <WorkspaceAvatar workspace={w} size={14} /> {w.name}
                        </span>
                      ))}
                      {r.created_by && <span className="text-ink-4">· {r.created_by.name}'s</span>}
                    </div>
                  ) : r.workspaces ? (
                    <div className="mt-0.5 text-[12px] text-amber">Runs no workspace's sessions yet</div>
                  ) : null}
                  {r.drain && r.status !== "disabled" && <DrainLine runner={r} />}
                  {r.status !== "disabled" && <LowDiskLine runner={r} />}
                  <RunnerResources runner={r} />
                  <RunnerImages runner={r} />
                </div>
                <div className="-ml-2.5 flex shrink-0 items-center gap-1 sm:ml-0">
                  {r.status !== "disabled" && (
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => drain.mutate(r)}
                      loading={drain.isPending && drain.variables?.id === r.id}
                    >
                      {r.drain ? "Stop draining" : "Drain"}
                    </Button>
                  )}
                  <Button
                    size="sm"
                    variant="ghost"
                    onClick={() =>
                      r.status !== "disabled" && r.active_sessions > 0
                        ? setConfirm({ runner: r, action: "disable" })
                        : toggle.mutate(r)
                    }
                  >
                    {r.status === "disabled" ? "Enable" : "Disable"}
                  </Button>
                  <button
                    type="button"
                    aria-label={`Remove ${r.name}`}
                    className="rounded p-1.5 text-ink-4 hover:text-danger"
                    onClick={() => setConfirm({ runner: r, action: "remove" })}
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
              </div>
            </li>
          ))}
        </ul>
      )}
      <ConfirmRunner
        confirm={confirm}
        onClose={() => setConfirm(null)}
        onDrain={(r) => {
          drain.mutate(r);
          setConfirm(null);
        }}
        onConfirm={(r, action) => {
          if (action === "disable") toggle.mutate(r);
          else remove.mutate(r.id);
          setConfirm(null);
        }}
      />
      <Dialog
        open={!!reg}
        onClose={() => setReg(null)}
        wide
        title={`Bring ${reg?.runner.name ?? "it"} online`}
        description="Run these on the machine. The token is shown once and lets the machine run sessions — keep it private."
      >
        {reg && (
          <div className="flex flex-col gap-3">
            <div className="text-[12.5px] text-ink-3">1. Install fctl</div>
            <CodeBox value={installCommand()} />
            <div className="text-[12.5px] text-ink-3">2. Register it as a runner and start the service</div>
            <CodeBox value={`FCTL_SERVER=${location.origin} fctl host install --token ${reg.token}`} />
            <p className="text-[12.5px] leading-relaxed text-ink-4">
              It shows up here as online within seconds and pulls the sandbox image in the background. With
              Lima, prepare the base VM once with `fctl host image build`. `fctl host doctor` checks
              everything, including whether the Mac is set to never sleep.
            </p>
          </div>
        )}
      </Dialog>
    </div>
  );
}

/** "online", or why it takes no new sessions: a drain ("draining" while sessions run, then "drained"), or its disk. */
function runnerState(r: G.Runner): string {
  if (r.status === "disabled") return r.status;
  if (r.drain) return r.active_sessions > 0 ? "draining" : "drained";
  return lowOnDisk(r) ? "low on disk" : r.status;
}

/** Who drained a runner, when, and what still runs on it. */
function DrainLine({ runner: r }: { runner: G.Runner }) {
  if (!r.drain) return null;
  const who = r.drain.by?.name ?? "Its host";
  const n = r.active_sessions;
  return (
    <div className="mt-0.5 text-[12px] text-amber">
      {who} started draining it {ago(r.drain.since)}: no new sessions start on it
      {n === 0
        ? ", and none runs."
        : n === 1
          ? ", and 1 session still runs."
          : `, and ${n} sessions still run.`}
    </div>
  );
}

/** How far a runner's free disk is below what its host keeps, and what happens meanwhile. */
function LowDiskLine({ runner: r }: { runner: G.Runner }) {
  if (!lowOnDisk(r) || r.disk_available_mb == null || r.min_free_disk_mb == null) return null;
  return (
    <div className="mt-0.5 text-[12px] text-amber">
      {diskBelow(r.disk_available_mb, r.min_free_disk_mb)}: its host removes stopped sandboxes, and no new
      sessions start on it until there is room.
    </div>
  );
}

/** Disabling a runner that runs sessions, or removing one, says what happens first. */
function ConfirmRunner({
  confirm,
  onClose,
  onDrain,
  onConfirm,
}: {
  confirm: { runner: G.Runner; action: "disable" | "remove" } | null;
  onClose: () => void;
  onDrain: (r: G.Runner) => void;
  onConfirm: (r: G.Runner, action: "disable" | "remove") => void;
}) {
  const r = confirm?.runner;
  const action = confirm?.action ?? "disable";
  const running = r?.active_sessions ?? 0;
  return (
    <Dialog
      open={!!confirm}
      onClose={onClose}
      title={`${action === "disable" ? "Disable" : "Remove"} ${r?.name ?? "the runner"}?`}
      description={
        action === "disable"
          ? "It disconnects at once and can't connect again until you enable it."
          : "Its token stops working: the machine has to be registered again to run sessions."
      }
    >
      <div className="flex flex-col gap-5">
        {running > 0 && (
          <Banner tone="warn">
            {running === 1
              ? "A session runs on it. Its agent keeps working in its sandbox without reaching fctl, and the session stops after ten minutes."
              : `${running} sessions run on it. Their agents keep working in their sandboxes without reaching fctl, and the sessions stop after ten minutes.`}
            {r?.drain
              ? ""
              : ` Drain it instead to let ${running === 1 ? "it" : "them"} finish: no new sessions start on it meanwhile.`}
          </Banner>
        )}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          {running > 0 && r && !r.drain && (
            <Button type="button" onClick={() => onDrain(r)}>
              Drain instead
            </Button>
          )}
          <Button type="button" variant="danger" onClick={() => r && onConfirm(r, action)}>
            {action === "disable" ? "Disable" : "Remove"}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/** What a runner gives sandboxes, what the machine has, and what is free. */
function RunnerResources({ runner: r }: { runner: G.Runner }) {
  const free = [
    r.memory_available_mb != null && `${memory(r.memory_available_mb)} of memory`,
    r.disk_available_mb != null && `${memory(r.disk_available_mb)} of disk`,
  ].filter(Boolean);
  const text = [
    r.sandbox && `Sandboxes: ${sandboxLimits(r.sandbox)}`,
    r.cpus && r.memory_mb && `machine: ${cpus(r.cpus)} and ${memory(r.memory_mb)}`,
    free.length > 0 && `${free.join(" and ")} free`,
  ]
    .filter(Boolean)
    .join(" · ");
  return text ? <div className="mt-0.5 text-[12px] text-ink-4">{text}</div> : null;
}

/** The image a runner gives sessions, and the others it runs and builds. */
function RunnerImages({ runner: r }: { runner: G.Runner }) {
  if (!r.image) return null;
  return (
    <div className="mt-0.5 text-[12px] text-ink-4">
      Image <span className="break-all font-mono">{r.image}</span> · {runsImages(r.allowed_images)} ·{" "}
      {buildsImages(r.allowed_builds)}
    </div>
  );
}

function InstanceSettings() {
  const settings = useQuery({ queryKey: keys.instanceSettings, queryFn: api.instanceSettings });
  const save = useMutation({
    mutationFn: (b: G.UpdateInstanceSettingsRequest) => api.updateInstanceSettings(b),
    onSuccess: (s, b) => {
      queryClient.setQueryData(keys.instanceSettings, s);
      void queryClient.invalidateQueries({ queryKey: keys.instance });
      void queryClient.invalidateQueries({ queryKey: keys.me });
      void queryClient.invalidateQueries({ queryKey: keys.users });
      toast(settingsSaved(s, b));
    },
    onError: (e) => toast(e.message, "error"),
  });
  const s = settings.data;
  return (
    <>
      <SignInSettings settings={s} save={(b) => save.mutate(b)} saving={save.isPending} />
      <SensitiveActions settings={s} save={(b) => save.mutate(b)} saving={save.isPending} />
      <Section title="Agents" description="What projects may let their agents do on this instance.">
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Published screenshots
            <span className="block text-[12.5px] text-ink-4">
              Projects that turn this on let their agents publish screenshots here, at links anyone who has
              one can open, so that pull requests show them: GitHub takes no uploads from a session. Links
              work for as long as each project chooses, 24 hours to 60 days. Off takes every published
              screenshot offline.
            </span>
          </span>
          <Switch
            checked={s?.publish_screenshots ?? false}
            onChange={(v) => save.mutate({ publish_screenshots: v })}
            label="Published screenshots"
          />
        </div>
      </Section>
      <ReservedSlugs />
    </>
  );
}

/**
 * Addresses admins hold for later: no new workspace or project takes one (one whose name makes it
 * gets a random ending instead), and nobody chooses one. fctl's own, like new and admin, aren't
 * listed: it reserves them itself.
 */
function ReservedSlugs() {
  const client = useQueryClient();
  const list = useQuery({ queryKey: keys.reservedSlugs, queryFn: api.reservedSlugs });
  const [slug, setSlug] = useState("");
  const shown = slug.trim().toLowerCase() || "…";
  const reserve = useMutation({
    mutationFn: () => api.reserveSlug({ slug: slug.trim() }),
    onSuccess: (r) => {
      client.setQueryData<G.ReservedSlugList>(keys.reservedSlugs, (l) => ({
        slugs: [...(l?.slugs ?? []).filter((x) => x.slug !== r.slug), r].sort((a, b) =>
          a.slug.localeCompare(b.slug),
        ),
      }));
      setSlug("");
      toast(`${r.slug} is reserved`);
    },
  });
  const unreserve = useMutation({
    mutationFn: (s: string) => api.unreserveSlug(s),
    onSuccess: (_, s) => {
      client.setQueryData<G.ReservedSlugList>(
        keys.reservedSlugs,
        (l) => l && { slugs: l.slugs.filter((x) => x.slug !== s) },
      );
      toast(`New workspaces and projects can take ${s} again`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const slugs = list.data?.slugs ?? [];
  return (
    <Section
      title="Reserved addresses"
      description="Addresses held for later. A new workspace or project whose name makes one gets a random ending instead, like home-829d34, and nobody can choose one. Those that have one already keep it."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (slug.trim()) reserve.mutate();
        }}
      >
        <Field
          label="Address"
          error={reserve.error?.message}
          hint={
            <>
              <span className="font-mono text-[11.5px]">
                {location.host}/w/{shown}
              </span>{" "}
              and <span className="font-mono text-[11.5px]">/p/{shown}</span>
            </>
          }
        >
          {(id) => (
            <div className="flex gap-2">
              <Input
                id={id}
                value={slug}
                onChange={(e) => {
                  setSlug(e.target.value);
                  if (reserve.isError) reserve.reset();
                }}
                placeholder="home"
                autoCapitalize="none"
                spellCheck={false}
                className="min-w-0 flex-1 font-mono"
              />
              <Button
                type="submit"
                variant="primary"
                size="field"
                loading={reserve.isPending}
                disabled={!slug.trim()}
              >
                Reserve
              </Button>
            </div>
          )}
        </Field>
      </form>
      {slugs.length > 0 && (
        <ul className="mt-5 border-t border-line">
          {slugs.map((r) => (
            <li key={r.slug} className="flex items-center gap-3 border-b border-line py-2.5">
              <div className="min-w-0 flex-1">
                <div className="font-mono text-[13px] text-ink [overflow-wrap:anywhere]">{r.slug}</div>
                <div className="text-[12px] text-ink-4">
                  reserved {ago(r.created_at)}
                  {r.created_by && ` by ${r.created_by.name}`}
                </div>
              </div>
              <IconButton
                label={`Stop reserving ${r.slug}`}
                className="size-7"
                disabled={unreserve.isPending && unreserve.variables === r.slug}
                onClick={() => unreserve.mutate(r.slug)}
              >
                <X className="size-4" />
              </IconButton>
            </li>
          ))}
        </ul>
      )}
    </Section>
  );
}

/** What changed, for the toast. */
function settingsSaved(s: G.InstanceSettings, b: G.UpdateInstanceSettingsRequest): string {
  if (b.verify_sensitive_actions !== undefined)
    return s.verify_sensitive_actions
      ? "Sensitive actions now ask people to confirm it's them"
      : "Sensitive actions no longer ask people to confirm it's them";
  if (b.require_two_factor !== undefined)
    return s.require_two_factor
      ? "Two-factor sign-in is required"
      : "Two-factor sign-in is optional: people choose";
  if (b.two_factor_google !== undefined)
    return s.two_factor_google
      ? "Google sign-ins ask for a code too"
      : "Google sign-ins ask only those who chose that";
  return s.publish_screenshots
    ? "Projects can now publish screenshots"
    : "Published screenshots are off and offline";
}

/** Two-factor sign-in for the whole instance: who needs it, and whether Google sign-ins ask for it. */
function SignInSettings({
  settings: s,
  save,
  saving,
}: {
  settings?: G.InstanceSettings;
  save: (b: G.UpdateInstanceSettingsRequest) => void;
  saving: boolean;
}) {
  const { data: me } = useMe();
  const users = useQuery({ queryKey: keys.users, queryFn: api.users });
  // A change that would stop people until they set two-factor up asks first, naming them.
  const [confirm, setConfirm] = useState<{ body: G.UpdateInstanceSettingsRequest; missing: G.User[] } | null>(
    null,
  );
  const current: G.InstanceSettings = s ?? {
    publish_screenshots: false,
    require_two_factor: false,
    two_factor_google: false,
    verify_sensitive_actions: false,
  };
  const active = users.data?.users.filter((u) => u.status === "active") ?? [];
  const using = active.filter(secured);
  const missingWith = (next: G.InstanceSettings) => active.filter((u) => lacksTwoFactor(u, next));
  const missingNow = missingWith(current);
  // Who requiring it would stop until they set it up.
  const missing = missingWith({ ...current, require_two_factor: true });
  const share = active.length ? using.length / active.length : 0;
  const change = (body: G.UpdateInstanceSettingsRequest) => {
    const next = { ...current, ...body } as G.InstanceSettings;
    const newly = missingWith(next).filter((u) => !missingNow.some((m) => m.id === u.id));
    if (newly.length > 0) setConfirm({ body, missing: newly });
    else save(body);
  };
  const google = confirm?.body.two_factor_google !== undefined;
  return (
    <Section
      title="Sign-in"
      description="How people prove it's them. Everyone can turn on two-factor sign-in for themselves in Settings."
    >
      <div className="flex flex-col gap-6">
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Require two-factor sign-in
            <span className="block text-[12.5px] leading-relaxed text-ink-4">
              Everyone who signs in with a password also needs a code from an authenticator app. People who
              haven't set one up can do nothing else until they have, in the browser and with their CLI and
              access tokens. People who sign in only with Google don't need one, unless Google sign-ins ask
              for codes too.
            </span>
          </span>
          <Switch
            checked={current.require_two_factor}
            disabled={!s || saving}
            onChange={(v) => change({ require_two_factor: v })}
            label="Require two-factor sign-in"
          />
        </div>
        {active.length > 0 && (
          <div className="flex flex-col gap-2.5 rounded-lg border border-line bg-sunken/40 px-4 py-3.5">
            <div className="flex items-baseline justify-between gap-3 text-[12.5px]">
              <span className="text-ink-2">
                <span className="font-medium text-ink tabular-nums">{using.length}</span> of {active.length}{" "}
                {active.length === 1 ? "person has" : "people have"} an authenticator app or a passkey
              </span>
              <span className="font-mono text-[11.5px] text-ink-4 tabular-nums">
                {Math.round(share * 100)}%
              </span>
            </div>
            <div className="h-1.5 overflow-hidden rounded-full bg-line" aria-hidden="true">
              <div
                className="h-full rounded-full bg-highlight transition-[width] duration-700 ease-[var(--ease-out)]"
                style={{ width: `${share * 100}%` }}
              />
            </div>
            {missing.length > 0 && (
              <div className="flex items-center gap-2.5 text-[12px] text-ink-4">
                <AvatarStack users={missing} max={6} size={20} />
                <span className="min-w-0">
                  {current.require_two_factor
                    ? `${missing.length === 1 ? "1 person sets" : `${missing.length} people set`} it up before anything else`
                    : `${missing.length === 1 ? "1 person" : `${missing.length} people`} would set it up first if you required it`}
                </span>
              </div>
            )}
          </div>
        )}
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Ask for codes on Google sign-in too
            <span className="block text-[12.5px] leading-relaxed text-ink-4">
              Google sign-ins then ask everyone who has two-factor for a code; otherwise only those who chose
              that in Settings. With two-factor required, people who sign in only with Google need it too.
            </span>
          </span>
          <Switch
            checked={current.two_factor_google}
            disabled={!s || saving}
            onChange={(v) => change({ two_factor_google: v })}
            label="Ask for codes on Google sign-in too"
          />
        </div>
      </div>
      <Dialog
        open={!!confirm}
        onClose={() => setConfirm(null)}
        title={google ? "Ask for codes on Google sign-in too?" : "Require two-factor sign-in?"}
        description={
          google
            ? "Google sign-ins will take a code too, and people who sign in only with Google will need two-factor."
            : "Everyone who signs in with a password will need a code from an authenticator app."
        }
      >
        {confirm && (
          <div className="flex flex-col gap-5">
            <div className="flex flex-col gap-2.5">
              <p className="text-[13.5px] leading-relaxed text-ink-2">
                {confirm.missing.length === 1
                  ? `${confirm.missing[0]?.name} hasn't`
                  : `These ${confirm.missing.length} people haven't`}{" "}
                set it up yet. Until they do, fctl asks them to before anything else, and their CLI and access
                tokens stop working.
              </p>
              <ul className="flex max-h-48 flex-col gap-1.5 overflow-y-auto">
                {confirm.missing.map((u) => (
                  <li key={u.id} className="flex items-center gap-2.5 text-[13px] text-ink">
                    <Avatar user={u} size={22} />
                    <span className="truncate">{u.name}</span>
                    <span className="truncate text-[12px] text-ink-4">{u.email}</span>
                  </li>
                ))}
              </ul>
              {confirm.missing.some((u) => u.id === me?.user.id) && (
                <Banner tone="info">
                  You haven't set it up either: you'll set yours up right after this.
                </Banner>
              )}
            </div>
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setConfirm(null)}>
                Cancel
              </Button>
              <Button
                variant="primary"
                onClick={() => {
                  save(confirm.body);
                  setConfirm(null);
                }}
              >
                {google ? "Ask for codes" : "Require it"}
              </Button>
            </div>
          </div>
        )}
      </Dialog>
    </Section>
  );
}

/**
 * Whether sensitive actions ask people to confirm it's them with a passkey or
 * a code: only the primary instance owner turns it on or off, and turning it
 * off asks them to confirm it's them (lib/api.ts opens the dialog).
 */
function SensitiveActions({
  settings: s,
  save,
  saving,
}: {
  settings?: G.InstanceSettings;
  save: (b: G.UpdateInstanceSettingsRequest) => void;
  saving: boolean;
}) {
  const { data: me } = useMe();
  const users = useQuery({ queryKey: keys.users, queryFn: api.users });
  const owner = me?.user.role === "owner";
  const on = !!s?.verify_sensitive_actions;
  const active = users.data?.users.filter((u) => u.status === "active") ?? [];
  // Who has nothing to confirm it with: the others, and you.
  const without = active.filter((u) => !secured(u) && u.id !== me?.user.id);
  const mine = !!me && (me.user.two_factor || me.passkeys.length > 0);
  return (
    <Section
      title="Sensitive actions"
      description="What people confirm with a passkey or a code from their authenticator app before they do it."
    >
      <div className="flex flex-col gap-4">
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Confirm sensitive actions
            <span className="block text-[12.5px] leading-relaxed text-ink-4">
              Inviting and promoting admins, turning two-factor sign-in off, adding a runner to a workspace,
              new access tokens and CLI sign-ins, connecting a Claude subscription, exporting data, and
              deleting accounts, projects and workspaces ask for a passkey or a code first. A browser or CLI
              that confirmed isn't asked again for 15 minutes.
            </span>
          </span>
          <Switch
            checked={on}
            disabled={!s || saving || !owner}
            onChange={(v) => save({ verify_sensitive_actions: v })}
            label="Confirm sensitive actions"
          />
        </div>
        {(!owner || (on && without.length > 0) || (on && !!me && !mine)) && (
          <ul className="flex flex-col gap-1.5 text-[12.5px] leading-relaxed text-ink-4">
            {!owner && (
              <li className="flex items-start gap-2">
                <Crown className="mt-[3px] size-3.5 shrink-0" />
                <span>Only the primary instance owner turns this on or off.</span>
              </li>
            )}
            {on && without.length > 0 && (
              <li className="flex items-start gap-2">
                <ShieldOff className="mt-[3px] size-3.5 shrink-0" />
                <span>
                  {without.length === 1 ? `${without[0]?.name} has` : `${without.length} people have`} neither
                  a passkey nor an authenticator app, so {without.length === 1 ? "isn't" : "aren't"} asked.
                  Requiring two-factor sign-in gives everyone one.
                </span>
              </li>
            )}
            {on && me && !mine && (
              <li className="flex items-start gap-2">
                <Fingerprint className="mt-[3px] size-3.5 shrink-0" />
                <span>
                  You have neither yourself, so it doesn't ask you either. Add a passkey in Settings.
                </span>
              </li>
            )}
          </ul>
        )}
      </div>
    </Section>
  );
}

function Audit() {
  return <AuditLog queryKey={keys.audit} load={(before) => api.audit(before)} ip />;
}

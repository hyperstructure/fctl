// Admin: who gets in, who is here, the machines doing the work, and what happened.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import { Cpu, Globe, Link2, MailPlus, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { Avatar, Banner, Button, CodeBox, Dialog, Empty, Field, Input, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago, cpus, memory, sandboxLimits } from "@/lib/format";
import { keys, queryClient, useMe, useRunners } from "@/lib/queries";

type Tab = "access" | "users" | "runners" | "audit";

export function Admin() {
  const { tab = "access" } = useSearch({ from: "/app/admin" });
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
      <nav className="mt-8 flex gap-1 border-b border-line">
        {(["access", "users", "runners", "audit"] as Tab[]).map((t) => (
          <Link
            key={t}
            to="/admin"
            search={{ tab: t === "access" ? undefined : t }}
            className={cn(
              "relative px-3 py-2.5 text-[13.5px] capitalize transition-colors",
              tab === t ? "text-ink" : "text-ink-3 hover:text-ink",
            )}
          >
            {t}
            {tab === t && <span className="absolute inset-x-2 -bottom-px h-[2px] rounded-full bg-accent" />}
          </Link>
        ))}
      </nav>
      <div className="mt-8">
        {tab === "access" && <Access />}
        {tab === "users" && <Users />}
        {tab === "runners" && <Runners />}
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
      toast(r.disabled.length ? `Removed. Disabled: ${r.disabled.join(", ")}` : "Removed");
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
        className="mt-6 flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          add.mutate();
        }}
      >
        <Field label="Email or domain" className="flex-1" error={add.error?.message}>
          {(id) => (
            <Input
              id={id}
              value={pattern}
              onChange={(e) => setPattern(e.target.value)}
              placeholder="ada@example.com or *@example.com"
            />
          )}
        </Field>
        {me?.user.role === "owner" && (
          <label className="mb-2.5 flex items-center gap-2 text-[13px] text-ink-3">
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
          icon={<MailPlus className="size-3.5" />}
          loading={add.isPending}
          disabled={!pattern}
        >
          Allow
        </Button>
      </form>
      <ul className="mt-6 border-t border-line">
        {list.data?.entries.map((e) => (
          <li key={e.id} className="flex items-center gap-3 border-b border-line py-3">
            <span className="text-ink-4">
              {e.pattern.startsWith("*@") ? <Globe className="size-4" /> : <MailPlus className="size-4" />}
            </span>
            <div className="min-w-0 flex-1">
              <div className="font-mono text-[13px] text-ink">{e.pattern}</div>
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
        description={
          invite?.emailed
            ? `We emailed ${invite.email}. You can also send them this link.`
            : `Send this link to ${invite?.email}. It works once, for 14 days.`
        }
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

function Users() {
  const { data: me } = useMe();
  const users = useQuery({ queryKey: keys.users, queryFn: api.users });
  const [reset, setReset] = useState<string | null>(null);
  const update = useMutation({
    mutationFn: (v: { id: string; body: G.UpdateUserRequest }) => api.updateUser(v.id, v.body),
    onSuccess: () => users.refetch(),
    onError: (e) => toast(e.message, "error"),
  });
  const link = useMutation({
    mutationFn: (id: string) => api.resetLink(id),
    onSuccess: (r) => setReset(r.url),
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <div>
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
            <div className="min-w-0 flex-1">
              <div className="text-[14px] text-ink">
                {u.name} {u.id === me?.user.id && <span className="text-[12px] text-ink-4">you</span>}
              </div>
              <div className="text-[12px] text-ink-4">
                {u.email} · {u.last_login_at ? `active ${ago(u.last_login_at)}` : "never signed in"}
              </div>
            </div>
            <select
              value={u.role}
              disabled={u.id === me?.user.id}
              onChange={(e) => update.mutate({ id: u.id, body: { role: e.target.value as G.InstanceRole } })}
              className="rounded-md border border-line bg-transparent px-2 py-1 text-[12.5px] text-ink-2"
            >
              <option value="member">member</option>
              <option value="admin">admin</option>
              {me?.user.role === "owner" && <option value="owner">owner</option>}
            </select>
            {u.id !== me?.user.id && (
              <>
                <Button
                  size="sm"
                  variant="ghost"
                  icon={<Link2 className="size-3.5" />}
                  onClick={() => link.mutate(u.id)}
                >
                  Reset link
                </Button>
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
          </li>
        ))}
      </ul>
      <Dialog
        open={!!reset}
        onClose={() => setReset(null)}
        title="Password reset link"
        description="Valid for one hour. Whoever opens it can set a new password, so send it privately."
      >
        {reset && <CodeBox value={reset} />}
      </Dialog>
    </div>
  );
}

function Runners() {
  const runners = useRunners();
  const [name, setName] = useState("");
  const [reg, setReg] = useState<G.RunnerRegistration | null>(null);
  const create = useMutation({
    mutationFn: () => api.createRunner({ name }),
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
  const site = location.hostname.startsWith("app.")
    ? `https://${location.hostname.slice(4)}`
    : location.origin;
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        Runners are the machines that run agents. Each session gets its own sandbox — a lightweight VM with
        Apple's container on macOS 26, or Docker, Lima, or plain processes elsewhere.
      </p>
      <form
        className="mt-6 flex items-end gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        <Field
          label="Register a machine"
          className="flex-1"
          hint="A name you'll recognize, like “studio mac”."
        >
          {(id) => (
            <Input id={id} value={name} onChange={(e) => setName(e.target.value)} placeholder="studio mac" />
          )}
        </Field>
        <Button
          type="submit"
          variant="primary"
          icon={<Plus className="size-3.5" />}
          loading={create.isPending}
          disabled={!name}
          className="mb-6"
        >
          Register
        </Button>
      </form>
      {runners.data?.runners.length === 0 ? (
        <Empty icon={<Cpu className="size-6" />} title="No machines yet">
          Register one above, then run two commands on it.
        </Empty>
      ) : (
        <ul className="border-t border-line">
          {runners.data?.runners.map((r) => (
            <li key={r.id} className="flex items-center gap-4 border-b border-line py-4">
              <span
                className={cn(
                  "size-2 shrink-0 rounded-full",
                  r.status === "online"
                    ? "bg-accent shadow-[0_0_10px_var(--c-accent)]"
                    : r.status === "disabled"
                      ? "bg-danger"
                      : "bg-ink-4",
                )}
              />
              <div className="min-w-0 flex-1">
                <div className="text-[14px] text-ink">
                  {r.name} <span className="ml-1 text-[12px] text-ink-4">{r.status}</span>
                </div>
                <div className="mt-0.5 text-[12px] text-ink-4">
                  {r.hostname ? `${r.hostname} · ` : ""}
                  {r.os && r.arch ? `${r.os}/${r.arch} · ` : ""}
                  {r.driver ? `${r.driver} driver · ` : ""}
                  {r.active_sessions}/{r.capacity} sessions
                  {r.version ? ` · fctl ${r.version}` : ""}
                  {r.last_seen_at ? ` · seen ${ago(r.last_seen_at)}` : " · never connected"}
                </div>
                <RunnerResources runner={r} />
              </div>
              <Button size="sm" variant="ghost" onClick={() => toggle.mutate(r)}>
                {r.status === "disabled" ? "Enable" : "Disable"}
              </Button>
              <button
                type="button"
                aria-label={`Remove ${r.name}`}
                className="rounded p-1.5 text-ink-4 hover:text-danger"
                onClick={() => remove.mutate(r.id)}
              >
                <Trash2 className="size-3.5" />
              </button>
            </li>
          ))}
        </ul>
      )}
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
            <CodeBox value={`curl -fsSL ${site}/install | sh`} />
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

function Audit() {
  const list = useQuery({ queryKey: keys.audit, queryFn: () => api.audit() });
  return (
    <ul className="border-t border-line">
      {list.data?.entries.map((e) => (
        <li key={e.id} className="flex items-baseline gap-3 border-b border-line py-2.5 text-[13px]">
          <span className="w-24 shrink-0 text-[12px] text-ink-4">{ago(e.ts)}</span>
          <span className="w-40 shrink-0 truncate text-ink-2">{e.actor?.name ?? "system"}</span>
          <span className="font-mono text-[12.5px] text-ink">{e.action}</span>
          <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink-4">{e.target}</span>
          {e.ip && <span className="shrink-0 font-mono text-[11.5px] text-ink-4">{e.ip}</span>}
        </li>
      ))}
      {list.data?.entries.length === 0 && <li className="py-6 text-[13px] text-ink-4">Nothing yet.</li>}
    </ul>
  );
}

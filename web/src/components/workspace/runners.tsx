// A workspace's own runners on its Runners tab: the ones that wait for its admins' approval, what
// its admins do with the runners that belong to it (they enrolled with one of its enrollment
// tokens), and the enrollment tokens themselves, with which machines that autoscaling creates
// enroll without anyone signing in on them (docs/architecture.md#enrollment-tokens).

import { useMutation, useQuery } from "@tanstack/react-query";
import {
  ArrowUpRight,
  Ban,
  Check,
  ChevronDown,
  Ellipsis,
  Hourglass,
  KeyRound,
  Pause,
  Pencil,
  Play,
  Plus,
  RotateCcw,
  Trash2,
} from "lucide-react";
import { useId, useState } from "react";
import {
  Banner,
  Button,
  CodeBox,
  Dialog,
  Field,
  IconButton,
  Input,
  Menu,
  MenuItem,
  MenuSeparator,
  Meta,
  Select,
  Switch,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { api, workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import {
  describeEnrollment,
  ENROLLMENT_EXPIRIES,
  type EnrollmentForm,
  enrollCommand,
  enrolledFrom,
  enrollmentReady,
  enrollmentRequest,
  limitOf,
  MAX_RUNNERS,
  machineFacts,
  minutesText,
  NEW_ENROLLMENT,
  OFFLINE_REMOVAL,
  tokenState,
} from "@/lib/enrollment";
import { ago } from "@/lib/format";
import { installCommand, siteUrl } from "@/lib/install";
import { keys, queryClient } from "@/lib/queries";

/** How enrollment works, on fctl.sh. */
const GUIDE = () => `${siteUrl()}/docs/getting-started/#many-machines-and-autoscaling`;

/** Refetches what a change to the workspace's runners touches. */
function refetch(w: G.Workspace) {
  for (const k of [keys.workspaceRunners(w.id), keys.enrollmentTokens(w.id), keys.runners, keys.workspaces])
    void queryClient.invalidateQueries({ queryKey: k });
}

// ───────────────────────────── waiting for approval ─────────────────────────────

/** The runners that wait for an admin of `w` to approve them, each with what it said of itself. */
export function WaitingRunners({ w, runners }: { w: G.Workspace; runners: G.Runner[] }) {
  const title = useId();
  const [rejecting, setRejecting] = useState<G.Runner | null>(null);
  const approve = useMutation({
    mutationFn: (r: G.Runner) => api.approveRunner(r.id),
    onSuccess: (r) => {
      toast(`${r.name} runs ${w.name}'s sessions now`);
      refetch(w);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const reject = useMutation({
    mutationFn: (r: G.Runner) => api.rejectRunner(r.id),
    onSuccess: (_, r) => {
      setRejecting(null);
      toast(`Rejected ${r.name}`);
      refetch(w);
    },
    onError: (e) => toast(e.message, "error"),
  });
  if (!runners.length) return null;
  const one = runners.length === 1;
  return (
    <section
      aria-labelledby={title}
      className="mt-6 animate-rise overflow-hidden rounded-xl border border-amber/30 bg-amber-soft/60"
    >
      <div className="flex items-start gap-3 px-4 pt-3.5 pb-1">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full bg-amber/15 text-amber">
          <Hourglass className="size-3.5" />
        </span>
        <div className="min-w-0">
          <h3 id={title} className="text-[14.5px] font-medium text-ink">
            {one ? "A runner waits for your approval" : `${runners.length} runners wait for your approval`}
          </h3>
          <p className="mt-0.5 max-w-2xl text-[12.5px] leading-relaxed text-ink-3">
            Until an admin approves {one ? "it" : "them"}, {one ? "it runs" : "they run"} nothing and{" "}
            {one ? "gets" : "get"} none of {w.name}'s code, secrets or tokens. Approve only machines you know:{" "}
            <code className="font-mono text-[11.5px]">fctl host status</code> on each shows its id.
          </p>
        </div>
      </div>
      <ul className="divide-y divide-amber/15">
        {runners.map((r) => {
          const from = enrolledFrom(r.enrollment);
          return (
            <li
              key={r.id}
              className="flex flex-col gap-3 px-4 py-3.5 sm:flex-row sm:items-center sm:gap-4 sm:pl-14"
            >
              <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
                <div className="flex flex-wrap items-center gap-x-2 text-[14px] text-ink">
                  {r.name}
                  <span className="inline-flex items-center gap-1.5 text-[12px] text-ink-4">
                    <span
                      className={cn(
                        "size-1.5 rounded-full",
                        r.status === "online" ? "bg-accent shadow-[0_0_8px_var(--c-accent)]" : "bg-ink-4",
                      )}
                    />
                    {r.status === "online"
                      ? "connected"
                      : r.last_seen_at
                        ? `offline, seen ${ago(r.last_seen_at)}`
                        : "not connected yet"}
                  </span>
                </div>
                <Meta
                  parts={[
                    ...machineFacts(r).map((f) => ({ key: f, node: f })),
                    {
                      key: "id",
                      node: (
                        <span
                          className="font-mono text-[11.5px]"
                          title="Its id, which fctl host status shows on it"
                        >
                          {r.id}
                        </span>
                      ),
                    },
                  ]}
                />
                <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
                  Enrolled {ago(r.created_at)}
                  {r.enrollment?.token_name ? <> with “{r.enrollment.token_name}”</> : null}
                  {from ? <> from {from}</> : null}.{" "}
                  {r.pending_until && (
                    <span className="text-ink-4">Removed {ago(r.pending_until)} unless approved.</span>
                  )}
                </p>
              </div>
              <div className="flex shrink-0 gap-2">
                <Button size="sm" variant="ghost" onClick={() => setRejecting(r)}>
                  Reject
                </Button>
                <Button
                  size="sm"
                  variant="primary"
                  icon={<Check className="size-3.5" />}
                  loading={approve.isPending && approve.variables?.id === r.id}
                  onClick={() => approve.mutate(r)}
                >
                  Approve
                </Button>
              </div>
            </li>
          );
        })}
      </ul>
      <Dialog
        open={!!rejecting}
        onClose={() => setRejecting(null)}
        title={`Reject ${rejecting?.name ?? ""}?`}
        description="It is removed, and its token stops working: the machine runs nothing for this workspace. It frees its place in its enrollment token's limit, and could enroll again while the token works."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setRejecting(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={reject.isPending}
            onClick={() => rejecting && reject.mutate(rejecting)}
          >
            Reject it
          </Button>
        </div>
      </Dialog>
    </section>
  );
}

// ───────────────────────────── a workspace's own runner ─────────────────────────────

/**
 * What a workspace's admins do with a runner that belongs to it, from its row's menu: rename,
 * drain, disable and remove it, and replace its token.
 */
export function ManageRunner({ w, runner: r }: { w: G.Workspace; runner: G.Runner }) {
  const [dialog, setDialog] = useState<"rename" | "token" | "remove" | "disable" | null>(null);
  const [name, setName] = useState(r.name);
  const [token, setToken] = useState<G.RunnerRegistration | null>(null);
  const close = () => {
    setDialog(null);
    setToken(null);
  };
  const update = useMutation({
    mutationFn: (b: G.UpdateRunnerRequest) => api.updateRunner(r.id, b),
    onSuccess: (x, b) => {
      close();
      if (b.draining !== undefined)
        toast(
          b.draining
            ? `Draining ${x.name}: no new sessions start on it`
            : `${x.name} takes new sessions again`,
        );
      else if (b.enabled !== undefined) toast(b.enabled ? `Enabled ${x.name}` : `Disabled ${x.name}`);
      refetch(w);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: () => api.deleteRunner(r.id),
    onSuccess: () => {
      close();
      toast(`Removed ${r.name}`);
      refetch(w);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const rotate = useMutation({
    mutationFn: () => api.rotateRunner(r.id),
    onSuccess: (x) => {
      setToken(x);
      refetch(w);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const disabled = r.status === "disabled";
  return (
    <>
      <Menu
        trigger={(toggle, open) => (
          <IconButton label={`Manage ${r.name}`} aria-haspopup="menu" aria-expanded={open} onClick={toggle}>
            <Ellipsis className="size-4" />
          </IconButton>
        )}
      >
        {(closeMenu) => {
          const pick = (f: () => void) => () => {
            closeMenu();
            f();
          };
          return (
            <>
              <MenuItem
                icon={<Pencil />}
                onClick={pick(() => {
                  setName(r.name);
                  setDialog("rename");
                })}
              >
                Rename…
              </MenuItem>
              {!disabled && (
                <MenuItem
                  icon={r.drain ? <Play /> : <Pause />}
                  onClick={pick(() => update.mutate({ draining: !r.drain }))}
                >
                  {r.drain ? "Stop draining" : "Drain"}
                </MenuItem>
              )}
              <MenuItem
                icon={disabled ? <Play /> : <Ban />}
                onClick={pick(() =>
                  !disabled && r.active_sessions > 0
                    ? setDialog("disable")
                    : update.mutate({ enabled: disabled }),
                )}
              >
                {disabled ? "Enable" : "Disable"}
              </MenuItem>
              <MenuItem icon={<RotateCcw />} onClick={pick(() => setDialog("token"))}>
                Replace token…
              </MenuItem>
              <MenuSeparator />
              <MenuItem danger icon={<Trash2 />} onClick={pick(() => setDialog("remove"))}>
                Remove…
              </MenuItem>
            </>
          );
        }}
      </Menu>
      <Dialog open={dialog === "rename"} onClose={close} title={`Rename ${r.name}`}>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (name.trim()) update.mutate({ name: name.trim() });
          }}
        >
          <Field label="Name" error={update.error?.message}>
            {(id) => <Input id={id} value={name} onChange={(e) => setName(e.target.value)} autoFocus />}
          </Field>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button type="submit" variant="primary" loading={update.isPending} disabled={!name.trim()}>
              Rename
            </Button>
          </div>
        </form>
      </Dialog>
      <Dialog
        open={dialog === "disable"}
        onClose={close}
        title={`Disable ${r.name}?`}
        description={`It disconnects, and takes no sessions until it is enabled. The ${r.active_sessions === 1 ? "session it runs stops" : `${r.active_sessions} sessions it runs stop`} within ten minutes; draining it first lets ${r.active_sessions === 1 ? "it" : "them"} finish.`}
      >
        <div className="flex justify-end gap-2">
          <Button
            variant="ghost"
            onClick={() => update.mutate({ draining: true })}
            loading={update.isPending}
          >
            Drain instead
          </Button>
          <Button
            variant="danger"
            onClick={() => update.mutate({ enabled: false })}
            loading={update.isPending}
          >
            Disable
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={dialog === "remove"}
        onClose={close}
        title={`Remove ${r.name}?`}
        description={`Its token stops working, and ${
          r.active_sessions
            ? `the ${r.active_sessions === 1 ? "session it runs stops" : `${r.active_sessions} sessions it runs stop`} and resume${r.active_sessions === 1 ? "s" : ""} on another runner`
            : "it runs nothing for this workspace any more"
        }. The machine can enroll again while an enrollment token works.`}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={close}>
            Cancel
          </Button>
          <Button variant="danger" loading={remove.isPending} onClick={() => remove.mutate()}>
            Remove it
          </Button>
        </div>
      </Dialog>
      <Dialog
        open={dialog === "token"}
        onClose={close}
        wide={!!token}
        title={token ? `${r.name}'s new token` : `Replace ${r.name}'s token?`}
        description={
          token
            ? "Its old token no longer works, and the machine is disconnected until it has this one. It is shown once: keep it private."
            : "Its old token stops working at once, and the machine disconnects until you put the new one in place on it."
        }
      >
        {token ? (
          <div className="flex flex-col gap-3">
            <div className="text-[12.5px] text-ink-3">On the machine, as the user the host runs as:</div>
            <CodeBox value={`FCTL_SERVER=${location.origin} fctl host install --token ${token.token}`} />
            <div className="flex justify-end">
              <Button variant="primary" onClick={close}>
                Done
              </Button>
            </div>
          </div>
        ) : (
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={close}>
              Cancel
            </Button>
            <Button variant="danger" loading={rotate.isPending} onClick={() => rotate.mutate()}>
              Replace it
            </Button>
          </div>
        )}
      </Dialog>
    </>
  );
}

// ───────────────────────────── enrollment tokens ─────────────────────────────

/** The workspace's enrollment tokens, for its admins: what each does, and how much it was used. */
export function EnrollmentTokens({ w }: { w: G.Workspace }) {
  const list = useQuery({
    queryKey: keys.enrollmentTokens(w.id),
    queryFn: () => workspaces.enrollmentTokens(w.id),
  });
  const [creating, setCreating] = useState(false);
  const [showEnded, setShowEnded] = useState(false);
  const [revoking, setRevoking] = useState<G.EnrollmentToken | null>(null);
  const revoke = useMutation({
    mutationFn: (t: G.EnrollmentToken) => workspaces.revokeEnrollmentToken(w.id, t.id),
    onSuccess: (t) => {
      setRevoking(null);
      toast(`Revoked ${t.name}: it enrolls no more runners`);
      void list.refetch();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const now = Date.now();
  const tokens = list.data?.tokens ?? [];
  const live = tokens.filter((t) => !["revoked", "expired"].includes(tokenState(t, now)));
  const ended = tokens.filter((t) => ["revoked", "expired"].includes(tokenState(t, now)));
  return (
    <section id="enrollment-tokens" className="mt-12 scroll-mt-6">
      <div className="flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between sm:gap-6">
        <div className="min-w-0">
          <h3 className="font-display text-xl text-ink">Enrollment tokens</h3>
          <p className="mt-1 max-w-2xl text-[14px] leading-relaxed text-ink-3">
            For machines that autoscaling creates, or that you set up without signing in on them: each enrolls
            itself with a token as one of {w.name}'s runners, with a token of its own. They belong to {w.name}
            , run everyone's sessions here, and its admins manage them.{" "}
            <a
              href={GUIDE()}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-0.5 text-accent hover:underline"
            >
              How enrollment works <ArrowUpRight className="size-3.5" />
            </a>
          </p>
        </div>
        <Button
          icon={<Plus className="size-3.5" />}
          onClick={() => setCreating(true)}
          className="shrink-0 self-start"
        >
          New enrollment token
        </Button>
      </div>
      {list.data && !tokens.length && <p className="mt-4 text-[13px] text-ink-4">None yet.</p>}
      {live.length > 0 && (
        <ul className="mt-5 divide-y divide-line border-y border-line">
          {live.map((t) => (
            <TokenRow key={t.id} token={t} now={now} onRevoke={() => setRevoking(t)} />
          ))}
        </ul>
      )}
      {ended.length > 0 && (
        <div className={cn(live.length ? "mt-3" : "mt-5")}>
          <button
            type="button"
            aria-expanded={showEnded}
            onClick={() => setShowEnded(!showEnded)}
            className="inline-flex items-center gap-1 text-[12.5px] text-ink-3 hover:text-ink"
          >
            <ChevronDown
              className={cn("size-3.5 transition-transform duration-200", showEnded && "rotate-180")}
            />
            {showEnded ? "Hide" : "Show"} {ended.length} revoked or expired
          </button>
          {showEnded && (
            <ul className="mt-2 animate-rise divide-y divide-line border-y border-line">
              {ended.map((t) => (
                <TokenRow key={t.id} token={t} now={now} />
              ))}
            </ul>
          )}
        </div>
      )}
      <NewEnrollmentToken
        w={w}
        open={creating}
        onClose={() => {
          setCreating(false);
          void list.refetch();
        }}
      />
      <Dialog
        open={!!revoking}
        onClose={() => setRevoking(null)}
        title={`Revoke ${revoking?.name ?? ""}?`}
        description={`It enrolls no more runners. ${
          revoking?.runners
            ? `The ${revoking.runners === 1 ? "runner it enrolled stays" : `${revoking.runners} runners it enrolled stay`}${revoking.pending ? `, and ${revoking.pending === 1 ? "the one that waits" : `the ${revoking.pending} that wait`} for approval still ${revoking.pending === 1 ? "waits" : "wait"}` : ""}: remove them on their own.`
            : "No runner it enrolled is left."
        }`}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setRevoking(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={revoke.isPending}
            onClick={() => revoking && revoke.mutate(revoking)}
          >
            Revoke it
          </Button>
        </div>
      </Dialog>
    </section>
  );
}

function TokenRow({
  token: t,
  now,
  onRevoke,
}: {
  token: G.EnrollmentToken;
  now: number;
  onRevoke?: () => void;
}) {
  const state = tokenState(t, now);
  const ended = state === "revoked" || state === "expired";
  return (
    <li className={cn("flex items-start gap-3 py-3.5", ended && "opacity-70")}>
      <span
        className={cn(
          "mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full",
          ended ? "bg-raised text-ink-4" : "bg-accent-soft text-accent",
        )}
      >
        <KeyRound className="size-3.5" />
      </span>
      <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
        <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[14px] text-ink">
          {t.name}
          {state !== "active" && (
            <span
              className={cn(
                "rounded-full border px-2 py-px text-[11.5px]",
                state === "full" ? "border-amber/30 text-amber" : "border-line text-ink-4",
              )}
            >
              {state === "full" ? "Full" : state === "revoked" ? "Revoked" : "Expired"}
            </span>
          )}
        </div>
        <p className="mt-0.5 text-[12.5px] leading-relaxed text-ink-3">
          {describeEnrollment(t.approval, t.max_runners, t.remove_offline_after_minutes)}
        </p>
        <Meta
          parts={[
            {
              key: "runners",
              node:
                t.max_runners !== undefined
                  ? `${t.runners} of ${t.max_runners} runners`
                  : `${t.runners} ${t.runners === 1 ? "runner" : "runners"}`,
            },
            t.pending > 0 && { key: "waiting", node: `${t.pending} waiting`, className: "text-amber" },
            t.last_used_at
              ? { key: "used", node: `last enrolled one ${ago(t.last_used_at)}` }
              : { key: "used", node: "not used yet" },
            state === "revoked"
              ? {
                  key: "ended",
                  node: `revoked ${ago(t.revoked_at)}${t.revoked_by ? ` by ${t.revoked_by.name}` : ""}`,
                }
              : t.expires_at
                ? {
                    key: "ended",
                    node: `${state === "expired" ? "expired" : "expires"} ${ago(t.expires_at)}`,
                  }
                : { key: "ended", node: "never expires" },
            { key: "made", node: `made by ${t.created_by?.name ?? "a former admin"}` },
          ]}
        />
      </div>
      {onRevoke && !ended && (
        <Button size="sm" variant="ghost" onClick={onRevoke}>
          Revoke
        </Button>
      )}
    </li>
  );
}

/** Makes an enrollment token, and shows it once with the commands that enroll a machine. */
function NewEnrollmentToken({ w, open, onClose }: { w: G.Workspace; open: boolean; onClose: () => void }) {
  const [form, setForm] = useState<EnrollmentForm>(NEW_ENROLLMENT);
  const [created, setCreated] = useState<G.CreatedEnrollmentToken | null>(null);
  const set = (f: Partial<EnrollmentForm>) => setForm((was) => ({ ...was, ...f }));
  const create = useMutation({
    mutationFn: () => workspaces.createEnrollmentToken(w.id, enrollmentRequest(form)),
    onSuccess: setCreated,
  });
  const close = () => {
    onClose();
    setForm(NEW_ENROLLMENT);
    setCreated(null);
    create.reset();
  };
  return (
    <Dialog
      open={open}
      onClose={close}
      wide
      title={created ? `${created.token.name} is ready` : "New enrollment token"}
      description={
        created
          ? describeEnrollment(
              created.token.approval,
              created.token.max_runners,
              created.token.remove_offline_after_minutes,
            )
          : `Machines that have it enroll themselves as ${w.name}'s runners, without signing in.`
      }
    >
      {created ? (
        <Created w={w} token={created} onDone={close} />
      ) : (
        <TokenForm w={w} form={form} set={set} create={create} />
      )}
    </Dialog>
  );
}

function TokenForm({
  w,
  form,
  set,
  create,
}: {
  w: G.Workspace;
  form: EnrollmentForm;
  set: (f: Partial<EnrollmentForm>) => void;
  create: { mutate: () => void; isPending: boolean; error: Error | null };
}) {
  const field = (create.error as { field?: string } | null)?.field;
  const err = (f: string) => (field === f ? create.error?.message : undefined);
  const limit = limitOf(form.limit);
  const expires = ENROLLMENT_EXPIRIES.find((e) => e.value === form.expires)?.hours;
  const offline = OFFLINE_REMOVAL.find((o) => o.value === form.offline)?.minutes;
  return (
    <form
      className="flex flex-col gap-5"
      onSubmit={(e) => {
        e.preventDefault();
        if (enrollmentReady(form)) create.mutate();
      }}
    >
      <Field label="Name" hint="What it's for, which the runners it enrolls show." error={err("name")}>
        {(id) => (
          <Input
            id={id}
            value={form.name}
            onChange={(e) => set({ name: e.target.value })}
            placeholder="Build fleet"
            autoFocus
          />
        )}
      </Field>
      <div className="grid items-start gap-4 sm:grid-cols-2">
        <Field label="Expires" error={err("expires_in_hours")}>
          {(id) => (
            <Select
              id={id}
              value={form.expires}
              onChange={(expires) => set({ expires })}
              options={ENROLLMENT_EXPIRIES.map((e) => ({ value: e.value, label: e.label }))}
            />
          )}
        </Field>
        <Field
          label="Runners at once"
          hint={Number.isNaN(limit) ? undefined : "Removed and rejected ones free their place."}
          error={
            err("max_runners") ??
            (Number.isNaN(limit) ? `A whole number from 1 to ${MAX_RUNNERS}, or none` : undefined)
          }
        >
          {(id) => (
            <Input
              id={id}
              inputMode="numeric"
              value={form.limit}
              onChange={(e) => set({ limit: e.target.value })}
              placeholder="No limit"
            />
          )}
        </Field>
      </div>
      <Field label="When they go offline" error={err("remove_offline_after_minutes")}>
        {(id) => (
          <Select
            id={id}
            value={form.offline}
            onChange={(offline) => set({ offline })}
            options={OFFLINE_REMOVAL.map((o) => ({
              value: o.value,
              label: o.label,
              description: o.description,
            }))}
          />
        )}
      </Field>
      <div className="flex items-start justify-between gap-4 rounded-lg border border-line px-4 py-3">
        <div className="min-w-0">
          <div className="text-[13.5px] text-ink">Approve each runner</div>
          <p className="mt-0.5 text-[12.5px] leading-relaxed text-ink-3">
            Runners it enrolls wait on this tab, with where they enrolled from, until an admin approves them;
            until then they run nothing. {w.name}'s admins get a notification.
          </p>
        </div>
        <Switch
          checked={form.approval}
          onChange={(approval) => set({ approval })}
          label="Approve each runner"
        />
      </div>
      <div
        aria-live="polite"
        className="rounded-lg border border-line bg-sunken/50 px-4 py-3 text-[13px] leading-relaxed text-ink-2"
      >
        {describeEnrollment(form.approval, Number.isNaN(limit) ? undefined : limit, offline)}{" "}
        {expires
          ? `It expires on ${new Date(Date.now() + expires * 3_600_000).toLocaleDateString(undefined, { dateStyle: "long" })}${expires < 24 ? ` at ${new Date(Date.now() + expires * 3_600_000).toLocaleTimeString(undefined, { timeStyle: "short" })}` : ""}.`
          : "It never expires: revoke it when you no longer need it."}
        {offline === undefined && " A runner it enrolls also goes when its host uninstalls."}
      </div>
      {create.error && !field && <Banner tone="error">{create.error.message}</Banner>}
      <div className="flex justify-end">
        <Button type="submit" variant="primary" loading={create.isPending} disabled={!enrollmentReady(form)}>
          Create token
        </Button>
      </div>
    </form>
  );
}

/** The new token, once, and how a machine enrolls with it. */
function Created({
  w,
  token,
  onDone,
}: {
  w: G.Workspace;
  token: G.CreatedEnrollmentToken;
  onDone: () => void;
}) {
  const t = token.token;
  return (
    <div className="flex flex-col gap-5">
      <div>
        <Banner tone="warn">
          Copy it now: it is shown once. Whoever has it enrolls machines in {w.name}
          {t.approval ? ", which wait for an admin's approval" : ", which run everyone's sessions there"}.
        </Banner>
        <CodeBox value={token.secret} className="mt-2" />
      </div>
      <ol className="flex flex-col gap-3 text-[13px] leading-relaxed text-ink-3">
        <li>
          <span className="text-ink-2">1. Install fctl on the machine</span>
          <CodeBox value={installCommand()} className="mt-1.5" />
        </li>
        <li>
          <span className="text-ink-2">
            2. Enroll it and start its host service, as the user the host runs as
          </span>
          <CodeBox value={enrollCommand(location.origin, token.secret)} className="mt-1.5" />
        </li>
      </ol>
      <p className="text-[12.5px] leading-relaxed text-ink-4">
        In a boot script (cloud-init's user data, a launch template) both run as they are; on Linux, where
        nobody logs in, run <code className="font-mono text-[11.5px] text-ink-3">loginctl enable-linger</code>{" "}
        for that user first. Run again, the install keeps the runner the machine enrolled.{" "}
        {t.remove_offline_after_minutes
          ? `A runner goes when its host uninstalls, or once it has been offline for ${minutesText(t.remove_offline_after_minutes)}.`
          : "A runner goes when its host uninstalls (fctl host uninstall)."}
      </p>
      <div className="flex justify-end">
        <Button variant="primary" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

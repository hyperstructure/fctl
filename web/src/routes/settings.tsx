// Settings: profile, email addresses, workspaces, Claude, notifications,
// sign-in methods, passkeys, two-factor sign-in, devices, access tokens,
// appearance, your data (export) and the danger zone (deleting the account).

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useSearch } from "@tanstack/react-router";
import {
  ArrowUpRight,
  Bell,
  BellOff,
  Download,
  KeyRound,
  Laptop,
  ListOrdered,
  LogOut,
  Mail,
  Monitor,
  Moon,
  Plus,
  Radio,
  Shield,
  ShieldCheck,
  Sun,
  Terminal,
  Trash2,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { OnRunnersDetails, OnRunnersSetup } from "@/components/claude/on-runners";
import { PasskeySettings } from "@/components/passkeys/settings";
import { PlanUsageList } from "@/components/plan-usage";
import { AccessTokens } from "@/components/tokens";
import { ConfirmWithCode } from "@/components/two-factor/confirm-code";
import { RecoveryCodes } from "@/components/two-factor/recovery-codes";
import { AuthenticatorSetup } from "@/components/two-factor/setup";
import {
  Avatar,
  Banner,
  Button,
  DangerZone,
  Dialog,
  Field,
  Input,
  LinearIcon,
  NameFields,
  nameParts,
  SecretInput,
  Section,
  Select,
  Skeleton,
  Spinner,
  Switch,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api, confirmFirst, emails, linearLinkUrl, workspaces as wsApi } from "@/lib/api";
import { tokenOnRunnersIn } from "@/lib/claude";
import { cn } from "@/lib/cn";
import { forgetDraft } from "@/lib/draft";
import { ago, browserName } from "@/lib/format";
import { disablePush, enablePush, forgetPush, permission, pushSupport } from "@/lib/push";
import { keys, queryClient, useInstance, useMe, useWorkspaces } from "@/lib/queries";
import { forgetSidebar } from "@/lib/sidebar";
import { useTheme } from "@/lib/theme";
import { type LinearOutcome, linearOutcome } from "@/lib/tickets";
import { FEW_RECOVERY_CODES } from "@/lib/two-factor";
import { claudeOptions, roleLabel } from "@/lib/workspaces";

const LINK_ERRORS: Record<string, string> = {
  google_linked_elsewhere: "That Google account is already linked to another user.",
  google_cancelled: "Linking was cancelled.",
  google_state: "Linking could not be verified. Try again.",
  google_expired: "Linking took too long. Try again.",
  google_reauth:
    "Linking Google takes a sign-in from the last 15 minutes. Sign out and in again, then link it.",
};

export function SettingsPage() {
  const { data: me } = useMe();
  const search = useSearch({ from: "/app/settings" });
  if (!me) return null;
  return (
    <div className="mx-auto max-w-[920px] px-6 pt-12 pb-24">
      <h1 className="font-display text-[34px] leading-tight font-medium tracking-[-0.02em]">Settings</h1>
      {search.linked === "google" && (
        <Banner tone="ok" className="mt-6">
          Google is linked. You can sign in with it from now on.
        </Banner>
      )}
      {search.error && (
        <Banner tone="error" className="mt-6">
          {LINK_ERRORS[search.error] ?? "That did not work. Try again."}
        </Banner>
      )}
      <div className="mt-6">
        <Profile me={me} />
        <Addresses me={me} />
        <Workspaces me={me} />
        <Claude me={me} />
        <LinearAccounts returned={search.linear} />
        <Notifications me={me} />
        <SignIn me={me} />
        <PasskeySettings me={me} />
        <TwoFactorSettings me={me} />
        <Devices />
        <AccessTokens />
        <Appearance />
        <YourData />
      </div>
      <AccountDangerZone me={me} />
    </div>
  );
}

function Profile({ me }: { me: G.Me }) {
  const [given, setGiven] = useState(() => nameParts(me.user).given);
  const [family, setFamily] = useState(() => nameParts(me.user).family);
  const [avatar, setAvatar] = useState(me.user.avatar_url ?? "");
  const save = useMutation({
    mutationFn: () =>
      api.updateMe({
        given_name: given,
        family_name: family,
        avatar_url: avatar,
      }),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      toast("Profile saved");
    },
  });
  const err = (f: string) =>
    (save.error as { field?: string } | null)?.field === f ? save.error?.message : undefined;
  return (
    <Section title="Profile" description="How you appear in sessions, in commits and to your team.">
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          save.mutate();
        }}
      >
        <div className="flex items-center gap-4">
          <Avatar user={{ ...me.user, avatar_url: avatar || undefined }} size={48} />
          <div className="text-[13px] text-ink-3">
            {me.user.email}
            <div className="text-[12px] text-ink-4">
              {me.user.role === "owner"
                ? "Primary instance owner · "
                : me.user.role === "admin"
                  ? "An instance admin · "
                  : ""}
              joined {ago(me.user.created_at)}
            </div>
          </div>
        </div>
        <NameFields given={given} family={family} onGiven={setGiven} onFamily={setFamily} error={err} />
        <Field
          label="Avatar URL"
          hint="An https:// image. Linking Google fills this in."
          error={err("avatar_url")}
        >
          {(id) => (
            <Input
              id={id}
              value={avatar}
              onChange={(e) => setAvatar(e.target.value)}
              placeholder="https://…"
            />
          )}
        </Field>
        <div>
          <Button type="submit" variant="primary" loading={save.isPending}>
            Save
          </Button>
        </div>
      </form>
    </Section>
  );
}

/** Your addresses: each signs in, has its own commit email, and is how workspaces know you. */
function Addresses({ me }: { me: G.Me }) {
  const [disconnecting, setDisconnecting] = useState<G.EmailAddress | null>(null);
  const set = (m: G.Me) => {
    queryClient.setQueryData(keys.me, m);
    void queryClient.invalidateQueries({ queryKey: keys.workspaces });
  };
  const primary = useMutation({
    mutationFn: (email: string) => emails.update(email, { primary: true }),
    onSuccess: (m) => {
      set(m);
      toast(`${m.user.email} is your primary address`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const disconnect = useMutation({
    mutationFn: (email: string) => emails.remove(email),
    onSuccess: (m, email) => {
      set(m);
      setDisconnecting(null);
      toast(`Disconnected ${email}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const list = me.emails.length
    ? me.emails
    : [{ email: me.user.email, primary: true, commit_email: me.commit_email, workspaces: [], added_at: 0 }];
  return (
    <Section
      title="Email addresses"
      description="Each signs in to this account. A workspace knows you by the address you joined it as, and your commits in its projects credit that address's commit email."
    >
      <ul className="flex flex-col divide-y divide-line rounded-xl border border-line">
        {list.map((e) => (
          <li key={e.email} className="flex flex-col gap-3 px-4 py-4">
            <div className="flex items-center gap-3">
              <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-raised text-ink-3 shadow-[inset_0_0_0_1px_var(--c-line)]">
                <Mail className="size-4" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-[14px] text-ink">
                  <span className="truncate">{e.email}</span>
                  {e.primary && (
                    <span className="rounded-full bg-accent-soft px-1.5 py-px text-[11px] text-ink-2">
                      Primary
                    </span>
                  )}
                </div>
                <div className="mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-ink-4">
                  {e.workspaces.length === 0 ? (
                    "No workspace knows you by it"
                  ) : (
                    <>
                      In
                      {e.workspaces.map((w) => (
                        <span key={w.id} className="inline-flex min-w-0 items-center gap-1 text-ink-3">
                          <WorkspaceAvatar workspace={w} size={13} />
                          <span className="truncate">{w.name}</span>
                        </span>
                      ))}
                    </>
                  )}
                </div>
              </div>
              {!e.primary && (
                <div className="flex shrink-0 gap-1">
                  <Button
                    size="sm"
                    variant="ghost"
                    loading={primary.isPending && primary.variables === e.email}
                    onClick={() => primary.mutate(e.email)}
                  >
                    Make primary
                  </Button>
                  <button
                    type="button"
                    aria-label={`Disconnect ${e.email}`}
                    className="rounded p-1.5 text-ink-4 hover:text-danger"
                    onClick={() => setDisconnecting(e)}
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
              )}
            </div>
            <CommitEmail address={e} onSaved={set} />
          </li>
        ))}
      </ul>
      <p className="mt-3 text-[12.5px] leading-relaxed text-ink-4">
        To add another address, ask an instance admin to invite it. Open the invitation while you are signed
        in here, and connect it to this account.
      </p>
      <Dialog
        open={!!disconnecting}
        onClose={() => setDisconnecting(null)}
        title={`Disconnect ${disconnecting?.email ?? ""}?`}
        description={
          disconnecting?.workspaces.length
            ? `It no longer signs you in, and you leave ${disconnecting.workspaces.map((w) => w.name).join(", ")}, which you joined as it. Your sessions there stop.`
            : "It no longer signs you in."
        }
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDisconnecting(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={disconnect.isPending}
            onClick={() => disconnecting && disconnect.mutate(disconnecting.email)}
          >
            Disconnect
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

/** The commit email of one address, saved on its own. */
function CommitEmail({ address, onSaved }: { address: G.EmailAddress; onSaved: (m: G.Me) => void }) {
  const [value, setValue] = useState(address.commit_email ?? "");
  useEffect(() => setValue(address.commit_email ?? ""), [address.commit_email]);
  const save = useMutation({
    mutationFn: () => emails.update(address.email, { commit_email: value.trim() }),
    onSuccess: (m) => {
      onSaved(m);
      toast(
        value.trim() ? `Commits credit you as ${value.trim()}` : "Those commits don't credit you any more",
      );
    },
  });
  const changed = value.trim() !== (address.commit_email ?? "");
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (changed) save.mutate();
      }}
    >
      <Field
        label="Commit email"
        error={save.error?.message}
        hint={
          <>
            Commits in the sessions you start
            {address.workspaces.length ? ` in ${address.workspaces.map((w) => w.name).join(", ")}` : ""}{" "}
            credit you as a co-author with it, which anyone who can read the repository sees. Use one your
            GitHub account has, like your{" "}
            <a
              href="https://github.com/settings/emails"
              target="_blank"
              rel="noreferrer"
              className="text-ink underline decoration-accent underline-offset-2"
            >
              noreply address
            </a>
            , so GitHub links the commits to you.
          </>
        }
      >
        {(id) => (
          <div className="flex gap-2">
            <Input
              id={id}
              type="email"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder="None: commits don't credit you"
              autoComplete="off"
              spellCheck={false}
              className="min-w-0 flex-1"
            />
            <Button type="submit" size="field" disabled={!changed} loading={save.isPending}>
              Save
            </Button>
          </div>
        )}
      </Field>
    </form>
  );
}

/** The workspaces you are in: how each knows you, and what your sessions there run on. */
function Workspaces({ me }: { me: G.Me }) {
  const list = useWorkspaces();
  const [leaving, setLeaving] = useState<G.Workspace | null>(null);
  const leave = useMutation({
    mutationFn: (w: G.Workspace) => wsApi.removeMember(w.id, me.user.id),
    onSuccess: (_, w) => {
      setLeaving(null);
      for (const k of [keys.workspaces, keys.projects, ["sessions"], keys.runners, keys.me])
        void queryClient.invalidateQueries({ queryKey: k });
      toast(`You left ${w.name}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const claude = useMutation({
    mutationFn: (v: { id: string; claude: string }) => wsApi.updateMembership(v.id, { claude: v.claude }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      toast("Saved: sessions take it when they next start or resume");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const data = list.data;
  return (
    <Section
      title="Workspaces"
      description="The ones you are in, each with its own people, projects, runners and GitHub accounts."
      actions={
        data && data.workspaces.length < data.limit ? (
          <Link to="/w/new">
            <Button size="sm" icon={<Plus className="size-3.5" />}>
              New workspace
            </Button>
          </Link>
        ) : undefined
      }
    >
      {!data ? (
        <Skeleton className="h-16" />
      ) : data.workspaces.length === 0 ? (
        <p className="text-[13.5px] text-ink-3">You aren't in a workspace yet.</p>
      ) : (
        <ul className="flex flex-col divide-y divide-line rounded-xl border border-line">
          {data.workspaces.map((w) => (
            <li key={w.id} className="flex flex-col gap-3 px-4 py-3.5 sm:flex-row sm:items-center">
              <Link
                to="/w/$slug"
                params={{ slug: w.slug }}
                className="flex min-w-0 flex-1 items-center gap-3 rounded-md"
              >
                <WorkspaceAvatar workspace={w} size={32} />
                <div className="min-w-0">
                  <div className="truncate text-[14px] text-ink">{w.name}</div>
                  <div className="truncate text-[12px] text-ink-4">
                    {roleLabel(w.role)} · as {w.email}
                    {w.blocked && <span className="text-amber"> · closed until you set up two-factor</span>}
                  </div>
                  {tokenOnRunnersIn(me, w) ? (
                    <div className="truncate text-[12px] text-ink-4">
                      Only on your runners that keep your Claude token
                    </div>
                  ) : (
                    w.own_runners && (
                      <div className="truncate text-[12px] text-ink-4">Only on your runners</div>
                    )
                  )}
                </div>
              </Link>
              <div className="flex items-center gap-2">
                <Select
                  size="sm"
                  aria-label={`Claude connection in ${w.name}`}
                  value={w.claude ?? "default"}
                  // Closed to you until you set up two-factor: so is choosing for it.
                  disabled={w.blocked}
                  onChange={(v) => claude.mutate({ id: w.id, claude: v === "default" ? "" : v })}
                  options={claudeOptions(me, w.claude, "Claude: ")}
                />
                {w.role !== "owner" && (
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<LogOut className="size-3.5" />}
                    onClick={() => setLeaving(w)}
                  >
                    Leave
                  </Button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
      {data && (
        <p className="mt-3 text-[12.5px] text-ink-4">
          {data.workspaces.length} of the {data.limit} workspaces anyone can be in.
        </p>
      )}
      <Dialog
        open={!!leaving}
        onClose={() => setLeaving(null)}
        title={`Leave ${leaving?.name ?? ""}?`}
        description="You lose access to its projects and sessions at once. Your sessions there stop, and so do its sessions on your runners, which leave it. Its admins can invite you again."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setLeaving(null)}>
            Cancel
          </Button>
          <Button variant="danger" loading={leave.isPending} onClick={() => leaving && leave.mutate(leaving)}>
            Leave
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

function Claude({ me }: { me: G.Me }) {
  const [token, setToken] = useState("");
  const [job, setJob] = useState<G.ClaudeLoginJob | null>(null);
  const [code, setCode] = useState("");
  const add = useMutation({
    mutationFn: () => api.claudeToken({ token }),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      setToken("");
      toast("Claude connected");
    },
  });
  const start = useMutation({ mutationFn: api.claudeLogin, onSuccess: setJob });
  const submit = useMutation({
    mutationFn: () => api.claudeLoginCode(job?.id ?? "", code),
    onSuccess: setJob,
  });
  // Follow the sign-in job (the feed also pushes updates).
  const live = useQuery({
    queryKey: ["claude-login", job?.id],
    queryFn: () => api.claudeLoginJob(job?.id ?? ""),
    enabled: !!job && job.state !== "succeeded" && job.state !== "failed",
    refetchInterval: 1500,
  });
  const current = live.data ?? job;
  useEffect(() => {
    if (current?.state === "succeeded") {
      void queryClient.invalidateQueries({ queryKey: keys.me });
      toast("Signed in with Claude: Remote Control is on");
    }
  }, [current?.state]);
  const remove = useMutation({
    mutationFn: (id: string) => api.claudeRemove(id),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
  });
  const makeDefault = useMutation({
    mutationFn: (id: string) => api.claudeDefault(id),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
  });

  return (
    <Section
      id="claude"
      title="Claude"
      description={
        <>
          Sessions you start run on your own Claude subscription. fctl refuses API keys and stops a session
          before it would draw on paid extra usage. Each connection shows how much of its plan's limits is
          used, as sessions last reported it. Keep the token on your runners, and you never give it to fctl.
        </>
      }
    >
      <div className="flex flex-col gap-6">
        {me.claude.length > 0 && (
          <ul className="divide-y divide-line border-y border-line">
            {me.claude.map((c) => (
              <li key={c.id} className="py-3">
                <div className="flex items-center gap-3">
                  <span
                    className={cn(
                      "flex size-8 items-center justify-center rounded-md border",
                      c.remote_control || c.on_runners
                        ? "border-accent/30 bg-accent-soft text-accent"
                        : "border-line bg-raised text-ink-3",
                    )}
                  >
                    {c.on_runners ? (
                      <ShieldCheck className="size-4" />
                    ) : c.remote_control ? (
                      <Radio className="size-4" />
                    ) : (
                      <KeyRound className="size-4" />
                    )}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13.5px] text-ink">
                      {c.account_email ??
                        c.label ??
                        (c.on_runners
                          ? "Token on your runners"
                          : c.kind === "oauth"
                            ? "Claude login"
                            : "Long-lived token")}
                      {c.is_default && (
                        <span className="ml-2 rounded-full bg-raised px-1.5 py-0.5 text-[10.5px] text-ink-3">
                          used for new sessions
                        </span>
                      )}
                    </div>
                    <div className="text-[12px] text-ink-4">
                      {c.on_runners
                        ? "Kept on your runners · never given to fctl · no Remote Control"
                        : c.kind === "oauth"
                          ? "Full login · Remote Control"
                          : "Inference only · no Remote Control"}
                      {c.subscription_type && ` · ${c.subscription_type}`}
                      {c.status === "invalid" && <span className="text-danger"> · needs reconnecting</span>}
                      {c.last_used_at && ` · used ${ago(c.last_used_at)}`}
                    </div>
                  </div>
                  {!c.is_default && (
                    <Button size="sm" variant="ghost" onClick={() => makeDefault.mutate(c.id)}>
                      Use by default
                    </Button>
                  )}
                  <button
                    type="button"
                    aria-label="Remove"
                    onClick={() => remove.mutate(c.id)}
                    className="rounded p-1.5 text-ink-4 hover:text-danger"
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
                <div className="mt-3 flex flex-col gap-4 pl-11">
                  {c.on_runners && <OnRunnersDetails />}
                  {c.usage?.length ? (
                    <PlanUsageList usage={c.usage} />
                  ) : (
                    <p className="text-[12px] text-ink-4">
                      Plan usage shows here once a session runs on this connection.
                    </p>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}

        <div>
          <div className="flex items-start justify-between gap-4">
            <div>
              <h3 className="flex items-center gap-2 text-[13.5px] font-medium text-ink">
                <Radio className="size-3.5 text-accent" /> Sign in with Claude
              </h3>
              <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
                The official Claude Code sign-in, run in a sandbox on one of your runners or, when none is
                online, on one that runs your sessions, whose machine sees the tokens. Sessions can then be
                followed and steered from claude.ai/code and the Claude apps.
              </p>
            </div>
            {(!current || current.state === "failed" || current.state === "succeeded") && (
              <Button size="sm" variant="primary" onClick={() => start.mutate()} loading={start.isPending}>
                Start
              </Button>
            )}
          </div>
          {start.error && (
            <Banner tone="error" className="mt-3">
              {start.error.message}
            </Banner>
          )}
          {current && (
            <div className="mt-4 animate-rise">
              {(current.state === "pending" || current.state === "starting") && (
                <p className="flex items-center gap-2 text-[13px] text-ink-3">
                  <Spinner className="size-3.5" /> Starting Claude Code in a sandbox…
                </p>
              )}
              {current.state === "awaiting_code" && (
                <form
                  className="flex flex-col gap-3"
                  onSubmit={(e) => {
                    e.preventDefault();
                    submit.mutate();
                  }}
                >
                  <a
                    href={current.url}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center gap-1.5 self-start rounded-md bg-ink px-3 py-1.5 text-[13px] font-medium text-canvas"
                  >
                    1. Approve on claude.ai <ArrowUpRight className="size-3.5" />
                  </a>
                  <Field label="2. Paste the code claude.ai shows">
                    {(id) => (
                      <Input
                        id={id}
                        value={code}
                        onChange={(e) => setCode(e.target.value)}
                        className="font-mono"
                        autoComplete="off"
                      />
                    )}
                  </Field>
                  <div>
                    <Button
                      type="submit"
                      variant="primary"
                      size="sm"
                      loading={submit.isPending}
                      disabled={!code.includes("#")}
                    >
                      Finish
                    </Button>
                  </div>
                </form>
              )}
              {current.state === "verifying" && (
                <p className="flex items-center gap-2 text-[13px] text-ink-3">
                  <Spinner className="size-3.5" /> Verifying with Claude…
                </p>
              )}
              {current.state === "succeeded" && (
                <Banner tone="ok">Connected. New sessions enable Remote Control.</Banner>
              )}
              {current.state === "failed" && <Banner tone="error">{current.error}</Banner>}
            </div>
          )}
        </div>

        <form
          className="border-t border-line pt-6"
          onSubmit={(e) => {
            e.preventDefault();
            add.mutate();
          }}
        >
          <h3 className="flex items-center gap-2 text-[13.5px] font-medium text-ink">
            <KeyRound className="size-3.5 text-ink-3" /> Or paste a long-lived token
          </h3>
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
            Run <code className="rounded bg-raised px-1 font-mono text-[11.5px]">claude setup-token</code> on
            any machine with Claude Code. Valid for a year; no Remote Control.
          </p>
          <div className="mt-3 flex gap-2">
            <SecretInput
              value={token}
              onChange={(e) => setToken(e.target.value)}
              placeholder="sk-ant-oat01-…"
              className="font-mono"
            />
            <Button type="submit" variant="secondary" size="field" loading={add.isPending} disabled={!token}>
              Connect
            </Button>
          </div>
          {add.error && <p className="mt-2 text-[12.5px] text-danger">{add.error.message}</p>}
        </form>

        {!me.claude.some((c) => c.on_runners) && <OnRunnersSetup />}
      </div>
    </Section>
  );
}

function Notifications({ me }: { me: G.Me }) {
  const list = useQuery({ queryKey: keys.notifications, queryFn: api.notifications });
  const [support] = useState(pushSupport);
  const [allowed, setAllowed] = useState(permission);
  const n = list.data;
  const set = (next: G.Notifications) => queryClient.setQueryData(keys.notifications, next);
  const update = useMutation({
    mutationFn: (b: G.UpdateNotificationsRequest) => api.updateNotifications(b),
    onSuccess: set,
    onError: (e) => {
      toast(e.message, "error");
      void list.refetch();
    },
  });
  const change = (b: G.UpdateNotificationsRequest) => {
    if (n) set({ ...n, settings: { ...n.settings, ...b } });
    update.mutate(b);
  };
  const on = useMutation({
    mutationFn: () => enablePush(n?.public_key ?? "", me.user.id),
    onSuccess: () => {
      toast("Notifications are on in this browser");
      void list.refetch();
    },
    onSettled: () => setAllowed(permission()),
  });
  const off = useMutation({
    mutationFn: (id?: string) => disablePush(id),
    onSuccess: () => {
      toast("Notifications are off in this browser");
      void list.refetch();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.removePushDevice(id),
    onSuccess: set,
    onError: (e) => toast(e.message, "error"),
  });
  const test = useMutation({
    mutationFn: api.testNotification,
    onSuccess: (r) =>
      r.failed
        ? toast(`Sent to ${r.sent} of ${r.sent + r.failed} browsers`, r.sent ? "info" : "error")
        : toast(r.sent === 1 ? "Sent. It should show in a moment." : `Sent to ${r.sent} browsers`),
    onError: (e) => toast(e.message, "error"),
  });
  const here = allowed === "granted" ? n?.devices.find((d) => d.current) : undefined;
  return (
    <Section
      title="Notifications"
      description="Sessions you follow notify you when they wait for your reply, pause on a usage limit or fail. You follow the sessions you start; follow others from their page."
    >
      {!n ? (
        <Skeleton className="h-28 w-full" />
      ) : (
        <div className="flex flex-col gap-6">
          {!n.public_key ? (
            <Banner tone="warn">
              This instance can't send notifications: it has no master key (FCTL_MASTER_KEY).
            </Banner>
          ) : support === "install" ? (
            <Banner tone="info">
              On iPhone and iPad, notifications come to fctl on the Home Screen. Tap Share, then Add to Home
              Screen, open fctl from there and turn them on.
            </Banner>
          ) : support === "unsupported" ? (
            <Banner tone="info">This browser can't show notifications.</Banner>
          ) : (
            <div>
              <div className="flex items-center gap-3 border-y border-line py-3">
                <span
                  className={cn(
                    "flex size-8 items-center justify-center rounded-md border",
                    here ? "border-accent/30 bg-accent-soft text-accent" : "border-line bg-raised text-ink-3",
                  )}
                >
                  {here ? <Bell className="size-4" /> : <BellOff className="size-4" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-[13.5px] text-ink">
                    {here ? "On in this browser" : "Off in this browser"}
                  </div>
                  <div className="text-[12px] text-ink-4">
                    {here
                      ? (here.name ?? "This browser")
                      : allowed === "denied"
                        ? "Notifications are blocked for this site. Allow them in your browser's site settings."
                        : browserName(navigator.userAgent)}
                  </div>
                </div>
                {here ? (
                  <>
                    <Button size="sm" variant="ghost" onClick={() => test.mutate()} loading={test.isPending}>
                      Send a test
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => off.mutate(here.id)}
                      loading={off.isPending}
                    >
                      Turn off
                    </Button>
                  </>
                ) : (
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() => on.mutate()}
                    loading={on.isPending}
                    disabled={allowed === "denied"}
                  >
                    Turn on
                  </Button>
                )}
              </div>
              {on.error && (
                <Banner tone="error" className="mt-3">
                  {on.error.message}
                </Banner>
              )}
            </div>
          )}

          <div>
            <h3 className="text-[13.5px] font-medium text-ink">Notify me when a session I follow</h3>
            <ul className="mt-2 divide-y divide-line border-y border-line">
              <SwitchRow
                label="Waits for my reply"
                checked={n.settings.waiting}
                onChange={(v) => change({ waiting: v })}
              />
              <SwitchRow
                label="Pauses on a usage limit"
                hint="It continues on its own when the limit resets."
                checked={n.settings.paused}
                onChange={(v) => change({ paused: v })}
              />
              <SwitchRow label="Fails" checked={n.settings.failed} onChange={(v) => change({ failed: v })} />
            </ul>
          </div>
          <ul className="divide-y divide-line border-y border-line">
            <SwitchRow
              label="Follow the sessions I start or resume"
              hint="Off: you follow only the sessions you choose."
              checked={n.settings.follow_started}
              onChange={(v) => change({ follow_started: v })}
            />
          </ul>

          {n.devices.length > 0 && (
            <div>
              <h3 className="text-[13.5px] font-medium text-ink">Browsers that get them</h3>
              <ul className="mt-2 divide-y divide-line border-y border-line">
                {n.devices.map((d) => (
                  <li key={d.id} className="flex items-center gap-3 py-2.5">
                    <Bell className="size-4 text-ink-4" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13px] text-ink">
                        {d.name ?? "Browser"}
                        {d.current && <span className="ml-2 text-[11.5px] text-accent">this browser</span>}
                      </div>
                      <div className="text-[11.5px] text-ink-4">
                        added {ago(d.created_at)}
                        {d.last_sent_at ? ` · last notified ${ago(d.last_sent_at)}` : " · nothing sent yet"}
                      </div>
                    </div>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => (d.current ? off.mutate(d.id) : remove.mutate(d.id))}
                    >
                      Remove
                    </Button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <p className="text-[12.5px] leading-relaxed text-ink-3">
            Several that arrive close together come as one, and at most one a minute. Sessions you're looking
            at don't notify you.
          </p>
        </div>
      )}
    </Section>
  );
}

function SwitchRow({
  label,
  hint,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <li className="flex items-center justify-between gap-4 py-2.5">
      <div className="min-w-0">
        <div className="text-[13.5px] text-ink">{label}</div>
        {hint && <div className="text-[12px] text-ink-4">{hint}</div>}
      </div>
      <Switch checked={checked} onChange={onChange} label={label} disabled={disabled} />
    </li>
  );
}

/**
 * Who fctl knows you as in Linear workspaces: what you delegate to fctl there runs as you, and your
 * replies there reach its agents. Linking takes a Linear sign-in, which fctl forgets at once.
 */
function LinearAccounts({ returned }: { returned?: LinearOutcome }) {
  const { data: instance } = useInstance();
  const mine = useQuery({
    queryKey: keys.myLinear,
    queryFn: api.myLinear,
    enabled: !!instance?.features.linear,
  });
  const [dropping, setDropping] = useState<G.LinearAccount | null>(null);
  const unlink = useMutation({
    mutationFn: (organization: string) => api.unlinkLinear(organization),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.myLinear, m);
      void queryClient.invalidateQueries({ queryKey: ["project-tickets"] });
      setDropping(null);
      toast("Unlinked");
    },
    onError: (e) => toast(e.message, "error"),
  });
  // Back from linking, or sent here for it: the browser's own jump to #linear came before this did.
  const ref = useRef<HTMLElement>(null);
  const loaded = !mine.isPending;
  useEffect(() => {
    if (loaded && (returned || location.hash === "#linear"))
      ref.current?.scrollIntoView({ block: "start", behavior: "instant" });
  }, [loaded, returned]);
  if (!instance?.features.linear) return null;
  const accounts = mine.data?.accounts ?? [];
  const outcome = returned && linearOutcome(returned);
  return (
    <section id="linear" ref={ref} className="scroll-mt-6">
      <Section
        title="Linear"
        description="Who fctl knows you as in Linear: issues you delegate to fctl there start sessions as you, on your Claude subscription, and your replies there reach its agents."
      >
        <div className="flex flex-col gap-4">
          {outcome && <Banner tone={outcome.tone}>{outcome.text}</Banner>}
          {mine.isPending ? (
            <Skeleton className="h-14" />
          ) : (
            <ul className="border-y border-line">
              {accounts.map((a) => (
                <li
                  key={a.organization}
                  className="flex items-center gap-3 border-b border-line py-3 last:border-b-0"
                >
                  <Avatar user={{ id: a.linear_user, name: a.name, avatar_url: a.avatar_url }} size={28} />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13.5px] text-ink">
                      {a.name}
                      <span className="text-ink-4"> in {a.organization_name}</span>
                    </div>
                    <div className="truncate text-[12px] text-ink-4">
                      {[a.email, a.url_key && `linear.app/${a.url_key}`].filter(Boolean).join(" · ")} · linked{" "}
                      {ago(a.linked_at)}
                    </div>
                  </div>
                  <Button size="sm" variant="ghost" onClick={() => setDropping(a)}>
                    Unlink
                  </Button>
                </li>
              ))}
              {accounts.length === 0 && (
                <li className="flex items-center gap-3 py-3">
                  <LinearIcon className="size-5 text-ink-3" />
                  <span className="text-[13.5px] text-ink-3">
                    fctl doesn't know your Linear account yet, so what you delegate to fctl there starts
                    nothing.
                  </span>
                </li>
              )}
            </ul>
          )}
          <div>
            <a href={linearLinkUrl()} className="inline-block">
              <Button icon={<LinearIcon className="size-3.5" />}>
                {accounts.length > 0 ? "Link another Linear account" : "Link your Linear account"}
              </Button>
            </a>
          </div>
        </div>
      </Section>
      <Dialog
        open={!!dropping}
        onClose={() => setDropping(null)}
        title={`Unlink your account in ${dropping?.organization_name ?? ""}?`}
        description="What you delegate to fctl there starts nothing, and your replies there don't reach its agents, until you link it again. Sessions already running carry on."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDropping(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={unlink.isPending}
            onClick={() => dropping && unlink.mutate(dropping.organization)}
          >
            Unlink
          </Button>
        </div>
      </Dialog>
    </section>
  );
}

function SignIn({ me }: { me: G.Me }) {
  const { data: instance } = useInstance();
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const change = useMutation({
    mutationFn: () =>
      api.changePassword({
        current_password: me.user.has_password ? current : undefined,
        new_password: next,
      }),
    onSuccess: () => {
      setCurrent("");
      setNext("");
      void queryClient.invalidateQueries({ queryKey: keys.me });
      toast("Password changed. Other sessions were signed out.");
    },
  });
  const unlink = useMutation({
    mutationFn: (id: string) => api.unlinkIdentity(id),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
    onError: (e) => toast(e.message, "error"),
  });
  const google = me.identities.find((i) => i.provider === "google");
  const err = (f: string) =>
    (change.error as { field?: string } | null)?.field === f ? change.error?.message : undefined;
  return (
    <Section title="Sign-in methods" description="Keep at least one way in.">
      <div className="flex flex-col gap-6">
        <div className="flex items-center gap-3 border-y border-line py-3">
          <svg viewBox="0 0 24 24" className="size-5" aria-hidden="true">
            <path
              fill="#4285F4"
              d="M23.5 12.3c0-.8-.1-1.6-.2-2.3H12v4.4h6.5a5.6 5.6 0 0 1-2.4 3.6v3h3.9c2.3-2.1 3.5-5.2 3.5-8.7z"
            />
            <path
              fill="#34A853"
              d="M12 24c3.2 0 6-1.1 8-2.9l-3.9-3c-1.1.7-2.5 1.2-4.1 1.2-3.1 0-5.8-2.1-6.7-5H1.3v3.1A12 12 0 0 0 12 24z"
            />
            <path fill="#FBBC05" d="M5.3 14.3a7.2 7.2 0 0 1 0-4.6V6.6H1.3a12 12 0 0 0 0 10.8l4-3.1z" />
            <path
              fill="#EA4335"
              d="M12 4.8c1.8 0 3.3.6 4.6 1.8l3.4-3.4A12 12 0 0 0 1.3 6.6l4 3.1c.9-2.9 3.6-4.9 6.7-4.9z"
            />
          </svg>
          <div className="min-w-0 flex-1">
            <div className="text-[13.5px] text-ink">Google</div>
            <div className="text-[12px] text-ink-4">
              {google
                ? `Linked as ${google.email}`
                : instance?.features.google
                  ? "Not linked"
                  : "Not available on this instance"}
            </div>
          </div>
          {google ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => unlink.mutate(google.id)}
              loading={unlink.isPending}
            >
              Unlink
            </Button>
          ) : (
            instance?.features.google && (
              <a
                href="/api/v1/auth/google/start?intent=link&return_to=/settings"
                className="rounded-md border border-line-strong px-3 py-1.5 text-[12.5px] font-medium text-ink hover:border-ink-4"
              >
                Link Google
              </a>
            )
          )}
        </div>
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            change.mutate();
          }}
        >
          {me.user.has_password && (
            <Field label="Current password" error={err("current_password")}>
              {(id) => (
                <Input
                  id={id}
                  type="password"
                  autoComplete="current-password"
                  value={current}
                  onChange={(e) => setCurrent(e.target.value)}
                />
              )}
            </Field>
          )}
          <Field
            label={me.user.has_password ? "New password" : "Set a password"}
            hint="At least 10 characters."
            error={err("password") ?? err("new_password")}
          >
            {(id) => (
              <Input
                id={id}
                type="password"
                autoComplete="new-password"
                value={next}
                onChange={(e) => setNext(e.target.value)}
              />
            )}
          </Field>
          <div>
            <Button type="submit" variant="secondary" loading={change.isPending} disabled={!next}>
              {me.user.has_password ? "Change password" : "Set password"}
            </Button>
          </div>
        </form>
      </div>
    </Section>
  );
}

type TwoFactorDialog = "setup" | "replace" | "off" | "codes";

function TwoFactorSettings({ me }: { me: G.Me }) {
  const { data: instance } = useInstance();
  const t = me.two_factor;
  const on = t.authenticator_since !== undefined;
  const few = on && t.recovery_codes <= FEW_RECOVERY_CODES;
  const [dialog, setDialog] = useState<TwoFactorDialog | null>(null);
  const [codes, setCodes] = useState<string[] | null>(null);
  // Replacing the app starts with a code from it.
  const [replacement, setReplacement] = useState<G.AuthenticatorSetup | null>(null);
  const close = () => {
    setDialog(null);
    setCodes(null);
    setReplacement(null);
  };
  const google = useMutation({
    mutationFn: (v: boolean) => api.updateTwoFactor({ google: v }),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
    onError: (e) => toast(e.message, "error"),
  });
  const linkedGoogle = me.identities.some((i) => i.provider === "google");
  return (
    <Section
      title="Two-factor sign-in"
      description={
        <>
          A code from an app on your phone after your password, so that a password alone can't sign anyone in.
          {t.required && " This instance requires it, or a passkey."}
        </>
      }
    >
      <div className="flex flex-col gap-6">
        <ul className="divide-y divide-line border-y border-line">
          <li className="flex flex-wrap items-center gap-3 py-3">
            <span
              className={cn(
                "flex size-8 shrink-0 items-center justify-center rounded-md border transition-colors",
                on ? "border-accent/30 bg-accent-soft text-accent" : "border-line bg-raised text-ink-3",
              )}
            >
              {on ? <ShieldCheck className="size-4" /> : <Shield className="size-4" />}
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-[13.5px] text-ink">
                Authenticator app
                {on && (
                  <span className="rounded-full bg-accent-soft px-1.5 py-0.5 text-[10.5px] font-medium text-ink">
                    on
                  </span>
                )}
              </div>
              <div className="text-[12px] text-ink-4">
                {on
                  ? `Set up ${ago(t.authenticator_since ?? 0)} · ${
                      t.authenticator_used_at
                        ? `last used ${ago(t.authenticator_used_at)}`
                        : "not used to sign in yet"
                    }`
                  : t.on
                    ? "Not set up: your passkeys are your second factor"
                    : "Off: your password alone signs you in"}
              </div>
            </div>
            {on ? (
              <div className="flex items-center gap-1">
                <Button size="sm" variant="ghost" onClick={() => setDialog("replace")}>
                  Replace
                </Button>
                {(!t.required || me.passkeys.length > 0) && (
                  <Button size="sm" variant="ghost" onClick={() => setDialog("off")}>
                    Turn off
                  </Button>
                )}
              </div>
            ) : (
              <Button size="sm" variant="primary" onClick={() => setDialog("setup")}>
                Set up
              </Button>
            )}
          </li>
          {on && (
            <li className="flex flex-wrap items-center gap-3 py-3">
              <span className="flex size-8 shrink-0 items-center justify-center rounded-md border border-line bg-raised text-ink-3">
                <ListOrdered className="size-4" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="text-[13.5px] text-ink">Recovery codes</div>
                <div className={cn("text-[12px]", few ? "text-amber" : "text-ink-4")}>
                  {t.recovery_codes === 0
                    ? "None left: get new ones, or a lost phone locks you out"
                    : `${t.recovery_codes} of 10 left${few ? ": get new ones before they run out" : ", each signs you in once"}`}
                </div>
              </div>
              <Button size="sm" variant={few ? "secondary" : "ghost"} onClick={() => setDialog("codes")}>
                Get new codes
              </Button>
            </li>
          )}
        </ul>
        {(instance?.features.google || linkedGoogle) && (
          <ul className="divide-y divide-line border-y border-line">
            <SwitchRow
              label="Ask for a code when I sign in with Google too"
              hint={
                t.google_required
                  ? "This instance asks everyone who has two-factor."
                  : !on && t.google
                    ? "From when you set up an authenticator app."
                    : "Off: Google's own sign-in is enough."
              }
              checked={t.google || t.google_required}
              disabled={t.google_required || google.isPending}
              onChange={(v) => google.mutate(v)}
            />
          </ul>
        )}
      </div>

      <Dialog
        open={dialog === "setup" || dialog === "replace"}
        onClose={close}
        wide
        title={dialog === "replace" ? "Replace your authenticator app" : "Set up an authenticator app"}
        description={
          dialog === "replace"
            ? replacement
              ? "Set up the new app or phone. Codes from the old one stop working; your recovery codes stay."
              : "First, confirm it's you with a code from your current app, or a recovery code if the phone is gone."
            : "Then signing in with your password also takes the code the app shows."
        }
      >
        {dialog === "replace" && !replacement ? (
          <ConfirmWithCode
            action="Continue"
            onCancel={close}
            submit={(code) => api.startAuthenticator({ code })}
            onDone={setReplacement}
          />
        ) : (
          <AuthenticatorSetup
            me={me}
            started={replacement ?? undefined}
            onRestart={dialog === "replace" ? () => setReplacement(null) : undefined}
            onConfirmed={(m) => queryClient.setQueryData(keys.me, m)}
            onCancel={close}
            onDone={(r) => {
              close();
              toast(
                r.codes
                  ? "Two-factor sign-in is on"
                  : dialog === "replace"
                    ? "The new authenticator app is in use. Your recovery codes still work."
                    : "Two-factor sign-in is on. Your recovery codes still work.",
              );
            }}
          />
        )}
      </Dialog>

      <Dialog
        open={dialog === "off"}
        onClose={close}
        title={t.required ? "Remove your authenticator app?" : "Turn off two-factor sign-in?"}
        description={
          t.required
            ? "Your passkeys stay your second factor, and your recovery codes stop working. Enter a code to confirm it's you."
            : "Signing in will take your password alone, and your recovery codes stop working. Enter a code to confirm it's you."
        }
      >
        {dialog === "off" && (
          <ConfirmWithCode
            action="Turn off"
            danger
            onCancel={close}
            submit={(code) => api.removeAuthenticator({ code })}
            onDone={(m) => {
              queryClient.setQueryData(keys.me, m);
              close();
              toast("Two-factor sign-in is off");
            }}
          />
        )}
      </Dialog>

      <Dialog
        open={dialog === "codes"}
        onClose={close}
        wide={!!codes}
        title={codes ? "Your new recovery codes" : "Get new recovery codes?"}
        description={
          codes
            ? "Each signs you in once if you lose your phone. Keep them somewhere safe: they are shown only now."
            : "Your current recovery codes stop working. Enter a code from your authenticator app, or a recovery code, to confirm."
        }
      >
        {codes ? (
          <div className="flex flex-col gap-5">
            <RecoveryCodes codes={codes} email={me.user.email} />
            <div className="flex justify-end">
              <Button variant="primary" onClick={close}>
                Done
              </Button>
            </div>
          </div>
        ) : (
          dialog === "codes" && (
            <ConfirmWithCode
              action="Get new codes"
              onCancel={close}
              submit={(code) => api.newRecoveryCodes({ code })}
              onDone={(r) => {
                queryClient.setQueryData(keys.me, r.me);
                setCodes(r.codes);
              }}
            />
          )
        )}
      </Dialog>
    </Section>
  );
}

/** Where you are signed in: browsers and the CLI. Access tokens have a section of their own. */
function Devices() {
  const list = useQuery({ queryKey: keys.tokens, queryFn: api.tokens });
  const revoke = useMutation({
    mutationFn: (id: string) => api.deleteToken(id),
    onSuccess: () => list.refetch(),
  });
  const devices = list.data?.tokens.filter((t) => t.kind !== "pat") ?? [];
  return (
    <Section
      title="Devices"
      description="Everywhere you are signed in. Revoking signs that device out at once."
    >
      <div className="flex flex-col gap-5">
        <ul className="divide-y divide-line border-y border-line">
          {devices.map((t) => (
            <li key={t.id} className="flex items-center gap-3 py-2.5">
              <span className="text-ink-4">
                {t.kind === "web" ? <Monitor className="size-4" /> : <Terminal className="size-4" />}
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] text-ink">
                  {t.name ?? (t.kind === "web" ? browserName(t.user_agent) : "CLI")}
                  {t.current && <span className="ml-2 text-[11.5px] text-accent">this device</span>}
                </div>
                <div className="text-[11.5px] text-ink-4">
                  {t.kind === "cli" ? "fctl" : "Browser"} · active {ago(t.last_used_at ?? t.created_at)}
                </div>
              </div>
              {!t.current && (
                <Button size="sm" variant="ghost" onClick={() => revoke.mutate(t.id)}>
                  Revoke
                </Button>
              )}
            </li>
          ))}
        </ul>
        <div className="flex items-center gap-3 text-[12.5px] text-ink-3">
          <Laptop className="size-4 text-ink-4" />
          <span className="flex-1">
            Sign in the CLI with{" "}
            <code className="rounded bg-raised px-1 font-mono text-[11.5px]">
              fctl login {location.origin}
            </code>
          </span>
        </div>
      </div>
    </Section>
  );
}

function Appearance() {
  const [theme, setTheme] = useTheme();
  return (
    <Section title="Appearance">
      <div className="flex gap-2">
        {(["dark", "light"] as const).map((t) => (
          <button
            key={t}
            type="button"
            onClick={() => setTheme(t)}
            className={cn(
              "flex items-center gap-2 rounded-lg border px-4 py-2.5 text-[13px] capitalize transition-colors",
              theme === t
                ? "border-accent/50 bg-accent-soft text-ink"
                : "border-line-strong text-ink-3 hover:text-ink",
            )}
          >
            {t === "dark" ? <Moon className="size-3.5" /> : <Sun className="size-3.5" />}
            {t}
          </button>
        ))}
      </div>
    </Section>
  );
}

function YourData() {
  return (
    <Section
      title="Your data"
      description="A copy of everything this instance keeps about you, as one JSON file."
    >
      <div className="flex flex-col items-start gap-4">
        <p className="max-w-xl text-[13.5px] leading-relaxed text-ink-3">
          Your account, sign-in methods, passkeys and two-factor sign-in, devices and tokens, Claude
          connections (without their secrets), notification settings and browsers, projects, the schedules you
          set up, the sessions you started with their full transcripts, the sessions you follow, and your
          activity log.
        </p>
        <ExportLink className="inline-flex h-8.5 items-center gap-2 rounded-md border border-line-strong bg-raised px-3.5 text-[13.5px] font-medium text-ink transition-colors hover:border-ink-4">
          <Download className="size-3.5" />
          Export your data
        </ExportLink>
      </div>
    </Section>
  );
}

const EXPORT = "/api/v1/me/export";

/**
 * Downloads your data. Where sensitive actions ask you to confirm it's you,
 * it asks first: the browser makes the download, so it can't ask itself.
 */
function ExportLink({ className, children }: { className: string; children: ReactNode }) {
  const [busy, setBusy] = useState(false);
  return (
    <a
      href={EXPORT}
      download
      aria-busy={busy || undefined}
      className={className}
      onClick={(e) => {
        e.preventDefault();
        if (busy) return;
        setBusy(true);
        confirmFirst()
          .then((confirmed) => {
            if (!confirmed) return;
            const a = document.createElement("a");
            a.href = EXPORT;
            a.download = "";
            a.click();
          })
          .catch((err: Error) => toast(err.message, "error"))
          .finally(() => setBusy(false));
      }}
    >
      {children}
    </a>
  );
}

function AccountDangerZone({ me }: { me: G.Me }) {
  const [open, setOpen] = useState(false);
  return (
    <DangerZone
      title="Delete your account"
      description="Deletes your account, addresses, sign-in methods, devices, tokens and Claude connections, and the workspaces only you are in, with their projects and sessions. This cannot be undone."
      action="Delete account…"
      onAction={() => setOpen(true)}
    >
      <DeleteAccount me={me} open={open} onClose={() => setOpen(false)} />
    </DangerZone>
  );
}

const sameEmail = (typed: string, email: string) => typed.trim().toLowerCase() === email.trim().toLowerCase();

function DeleteAccount({ me, open, onClose }: { me: G.Me; open: boolean; onClose: () => void }) {
  const [typed, setTyped] = useState("");
  const plan = useQuery({ queryKey: keys.accountDeletion, queryFn: api.accountDeletion, enabled: open });
  const remove = useMutation({
    mutationFn: () => api.deleteAccount({ email: typed }),
    onSuccess: async () => {
      await forgetPush();
      await forgetDraft();
      queryClient.clear();
      forgetSidebar(); // last, so that no list that comes is kept again
      window.location.assign("/login?deleted=1");
    },
  });
  useEffect(() => {
    if (!open) {
      setTyped("");
      remove.reset();
    }
  }, [open, remove.reset]);
  const p = plan.data;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Delete your account?"
      description="This deletes your data for good. It cannot be undone."
    >
      {plan.isPending ? (
        <div className="flex justify-center py-6">
          <Spinner className="size-5" />
        </div>
      ) : plan.error ? (
        <Banner tone="error">{plan.error.message}</Banner>
      ) : p?.blocked ? (
        <div className="flex flex-col gap-5">
          <Banner tone="warn">{p.blocked}</Banner>
          <div className="flex justify-end">
            <Button onClick={onClose}>Close</Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-5"
          onSubmit={(e) => {
            e.preventDefault();
            if (sameEmail(typed, me.user.email)) remove.mutate();
          }}
        >
          <ul className="flex list-disc flex-col gap-1.5 pl-5 text-[13.5px] leading-relaxed text-ink-2 marker:text-ink-4">
            <li>
              Your account <span className="text-ink">{me.user.email}</span>
              {me.emails.length > 1 && " and its other addresses"}, sign-in methods, devices and tokens, and
              Claude connections are deleted.
            </li>
            {p?.workspaces.map((w) => (
              <li key={w.id}>
                <span className="text-ink">{w.name}</span> is deleted: no one else is in it.
              </li>
            ))}
            {p?.projects.map((project) => (
              <li key={project.slug}>
                Its project <span className="text-ink">{project.name}</span> goes
                {project.sessions === 0
                  ? ""
                  : project.sessions === 1
                    ? " with its session and transcript"
                    : ` with its ${project.sessions} sessions and their transcripts`}
                .
              </li>
            ))}
            {p?.handovers.map((h) => (
              <li key={h.workspace.id}>
                <span className="text-ink">{h.to.name}</span> becomes the primary admin of{" "}
                <span className="text-ink">{h.workspace.name}</span>.
              </li>
            ))}
            {p && p.kept_sessions > 0 && (
              <li>
                {p.kept_sessions === 1 ? "The session" : `The ${p.kept_sessions} sessions`} you started in
                shared workspaces {p.kept_sessions === 1 ? "stays" : "stay"} there, shown as started by a
                deleted user.
              </li>
            )}
            <li>Repositories on GitHub are not touched.</li>
          </ul>
          <p className="text-[13px] leading-relaxed text-ink-3">
            Want a copy first?{" "}
            <ExportLink className="text-ink underline decoration-accent underline-offset-2">
              Export your data
            </ExportLink>
            .
          </p>
          <Field
            label={
              <>
                Type <span className="font-mono text-ink">{me.user.email}</span> to confirm
              </>
            }
            error={remove.error?.message}
          >
            {(id) => (
              <Input
                id={id}
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
              />
            )}
          </Field>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="danger"
              loading={remove.isPending}
              disabled={!sameEmail(typed, me.user.email)}
            >
              Delete my account
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

// Settings: profile, Claude, sign-in methods, devices and tokens, appearance.

import { useMutation, useQuery } from "@tanstack/react-query";
import { useSearch } from "@tanstack/react-router";
import { ArrowUpRight, KeyRound, Laptop, Monitor, Moon, Radio, Sun, Terminal, Trash2 } from "lucide-react";
import { useEffect, useState } from "react";
import { Avatar, Banner, Button, CodeBox, Field, Input, Section, Spinner, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago } from "@/lib/format";
import { keys, queryClient, useInstance, useMe } from "@/lib/queries";
import { useTheme } from "@/lib/theme";

const LINK_ERRORS: Record<string, string> = {
  google_linked_elsewhere: "That Google account is already linked to another user.",
  google_cancelled: "Linking was cancelled.",
  google_state: "Linking could not be verified. Try again.",
  google_expired: "Linking took too long. Try again.",
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
        <Claude me={me} />
        <SignIn me={me} />
        <Tokens />
        <Appearance />
      </div>
    </div>
  );
}

function Profile({ me }: { me: G.Me }) {
  const [name, setName] = useState(me.user.name);
  const [avatar, setAvatar] = useState(me.user.avatar_url ?? "");
  const save = useMutation({
    mutationFn: () => api.updateMe({ name, avatar_url: avatar }),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      toast("Profile saved");
    },
  });
  return (
    <Section title="Profile" description="How you appear in sessions and to your team.">
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
              {me.user.role} · joined {ago(me.user.created_at)}
            </div>
          </div>
        </div>
        <Field
          label="Name"
          error={
            (save.error as { field?: string } | null)?.field === "name" ? save.error?.message : undefined
          }
        >
          {(id) => <Input id={id} value={name} onChange={(e) => setName(e.target.value)} />}
        </Field>
        <Field
          label="Avatar URL"
          hint="An https:// image. Linking Google fills this in."
          error={
            (save.error as { field?: string } | null)?.field === "avatar_url"
              ? save.error?.message
              : undefined
          }
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
      title="Claude"
      description={
        <>
          Sessions you start run on your own Claude subscription. fctl refuses API keys and stops a session
          before it would draw on paid extra usage.
        </>
      }
    >
      <div className="flex flex-col gap-6">
        {me.claude.length > 0 && (
          <ul className="divide-y divide-line border-y border-line">
            {me.claude.map((c) => (
              <li key={c.id} className="flex items-center gap-3 py-3">
                <span
                  className={cn(
                    "flex size-8 items-center justify-center rounded-md border",
                    c.remote_control
                      ? "border-accent/30 bg-accent-soft text-accent"
                      : "border-line bg-raised text-ink-3",
                  )}
                >
                  {c.remote_control ? <Radio className="size-4" /> : <KeyRound className="size-4" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13.5px] text-ink">
                    {c.account_email ?? c.label ?? (c.kind === "oauth" ? "Claude login" : "Long-lived token")}
                    {c.is_default && (
                      <span className="ml-2 rounded-full bg-raised px-1.5 py-0.5 text-[10.5px] text-ink-3">
                        used for new sessions
                      </span>
                    )}
                  </div>
                  <div className="text-[12px] text-ink-4">
                    {c.kind === "oauth"
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
                The official Claude Code sign-in, run on one of your hosts. Sessions can then be followed and
                steered from claude.ai/code and the Claude apps.
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
            <Input
              value={token}
              onChange={(e) => setToken(e.target.value)}
              type="password"
              placeholder="sk-ant-oat01-…"
              autoComplete="off"
              className="font-mono"
            />
            <Button type="submit" variant="secondary" loading={add.isPending} disabled={!token}>
              Connect
            </Button>
          </div>
          {add.error && <p className="mt-2 text-[12.5px] text-danger">{add.error.message}</p>}
        </form>
      </div>
    </Section>
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

function Tokens() {
  const list = useQuery({ queryKey: keys.tokens, queryFn: api.tokens });
  const [name, setName] = useState("");
  const [created, setCreated] = useState<string | null>(null);
  const create = useMutation({
    mutationFn: () => api.createToken({ name }),
    onSuccess: (t) => {
      setCreated(t.secret);
      setName("");
      void list.refetch();
    },
  });
  const revoke = useMutation({
    mutationFn: (id: string) => api.deleteToken(id),
    onSuccess: () => list.refetch(),
  });
  return (
    <Section
      title="Devices and tokens"
      description="Everywhere you are signed in. Revoking signs that device out at once."
    >
      <div className="flex flex-col gap-5">
        <ul className="divide-y divide-line border-y border-line">
          {list.data?.tokens.map((t) => (
            <li key={t.id} className="flex items-center gap-3 py-2.5">
              <span className="text-ink-4">
                {t.kind === "web" ? (
                  <Monitor className="size-4" />
                ) : t.kind === "cli" ? (
                  <Terminal className="size-4" />
                ) : (
                  <KeyRound className="size-4" />
                )}
              </span>
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] text-ink">
                  {t.name ?? (t.kind === "web" ? browser(t.user_agent) : "CLI")}
                  {t.current && <span className="ml-2 text-[11.5px] text-accent">this device</span>}
                </div>
                <div className="text-[11.5px] text-ink-4">
                  {t.kind === "pat" ? "Access token" : t.kind === "cli" ? "fctl" : "Browser"} · active{" "}
                  {ago(t.last_used_at ?? t.created_at)}
                  {t.expires_at && t.kind === "pat" && ` · expires ${ago(t.expires_at)}`}
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
        <form
          onSubmit={(e) => {
            e.preventDefault();
            create.mutate();
          }}
        >
          <Field label="New access token" hint="For scripts and CI: use it as FCTL_TOKEN.">
            {(id) => (
              <div className="flex gap-2">
                <Input
                  id={id}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="deploy bot"
                />
                <Button type="submit" variant="secondary" loading={create.isPending} disabled={!name}>
                  Create
                </Button>
              </div>
            )}
          </Field>
        </form>
        {created && (
          <div className="animate-rise">
            <Banner tone="warn">Copy it now: it is shown once.</Banner>
            <CodeBox value={created} className="mt-2" />
          </div>
        )}
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

function browser(ua?: string): string {
  if (!ua) return "Browser";
  const name = /Edg\//.test(ua)
    ? "Edge"
    : /Chrome\//.test(ua)
      ? "Chrome"
      : /Firefox\//.test(ua)
        ? "Firefox"
        : /Safari\//.test(ua)
          ? "Safari"
          : "Browser";
  const os = /Mac OS X/.test(ua)
    ? "macOS"
    : /Windows/.test(ua)
      ? "Windows"
      : /iPhone|iPad/.test(ua)
        ? "iOS"
        : /Android/.test(ua)
          ? "Android"
          : /Linux/.test(ua)
            ? "Linux"
            : "";
  return os ? `${name} on ${os}` : name;
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

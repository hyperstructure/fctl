// Sign-in, setup, invitations and password resets, and the second step of
// a sign-in with two-factor.

import { type FoundryHandle, foundry } from "@hyperstructure/design/gl/foundry";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { ArrowRight, Fingerprint, Sparkles } from "lucide-react";
import { type FormEvent, type ReactNode, useEffect, useState } from "react";
import { usePasskeySignIn } from "@/components/passkeys/sign-in";
import { challengeCopy, SecondFactorStep } from "@/components/two-factor/challenge";
import { Avatar, Banner, Button, Field, GLCanvas, Input, Logo, NameFields, Spinner } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { ApiFailure, api, emails } from "@/lib/api";
import { cn } from "@/lib/cn";
import { saveSent } from "@/lib/early-access";
import { OFFER_PASSKEY } from "@/lib/passkeys";
import { appPath } from "@/lib/paths";
import { keys, queryClient, useInstance, useMe } from "@/lib/queries";
import { RECOVERY_LEFT } from "@/lib/two-factor";

const GOOGLE_ERRORS: Record<string, string> = {
  google_expired: "The Google sign-in took too long. Try again.",
  google_state: "The Google sign-in could not be verified. Try again.",
  google_cancelled: "Google sign-in was cancelled.",
  google_exchange: "Google did not confirm the sign-in. Try again.",
  google_invalid: "Google returned an invalid sign-in.",
  google_unverified: "Your Google account's email is not verified.",
  google_link_required:
    "An account with this email exists. Sign in with your password, then link Google in Settings.",
  not_allowed: "This email is not allowed on this instance. Ask an admin for access.",
  disabled: "Your account is disabled. Ask an admin for access.",
};

/** sessionStorage: an invitation's token while its address signs in to connect it. */
const PENDING_INVITE = "fctl:invite";

/**
 * The invitation's token: in the link's fragment, which never reaches a
 * server, or kept in this tab while its address signs in to connect it.
 */
function inviteToken(): string {
  const fromLink = new URLSearchParams(window.location.hash.slice(1)).get("token");
  if (fromLink) return fromLink;
  try {
    return sessionStorage.getItem(PENDING_INVITE) ?? "";
  } catch {
    return "";
  }
}

function forgetInvite() {
  try {
    sessionStorage.removeItem(PENDING_INVITE);
  } catch {}
}

/** The invitation made or joined an account: the request for early access it answered is done. */
function used() {
  forgetInvite();
  saveSent(undefined);
}

export function AuthFrame({
  title,
  subtitle,
  children,
  footer,
  wide,
  below,
  onFoundry,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  /** Room for more than a form (setting up two-factor). */
  wide?: boolean;
  /** Under the card, before the footer. */
  below?: ReactNode;
  /** The scene behind the page, once it is mounted, for pages that light its threads. */
  onFoundry?: (scene: FoundryHandle) => void;
}) {
  const { data: instance } = useInstance();
  return (
    <div className="relative flex min-h-dvh flex-col overflow-hidden bg-canvas">
      <div className="absolute inset-0">
        <GLCanvas
          scene={() => {
            // Dim stars that fade rather than blink: a calm sky behind the form.
            const scene = foundry({ planet: [1.02, -0.75, 1.15], density: 0.6, stars: 0.3 });
            onFoundry?.(scene);
            return scene.scene;
          }}
          options={{ pixel: 3, fps: 30 }}
        />
      </div>
      <div className="relative z-10 flex items-center justify-between gap-4 px-6 py-5">
        <Logo />
        <span className="min-w-0 truncate text-[12.5px] text-ink-4">{instance?.name}</span>
      </div>
      <div className="relative z-10 flex flex-1 items-center justify-center px-5 pb-16">
        {/* An address or a name with no spaces wraps on a phone rather than running off it. */}
        <div
          className={cn(
            "w-full animate-rise [overflow-wrap:anywhere]",
            wide ? "max-w-[720px]" : "max-w-[400px]",
          )}
        >
          <h1 className="font-display text-[34px] leading-[1.1] font-medium tracking-[-0.02em] text-ink">
            {title}
          </h1>
          {subtitle && <p className="mt-3 text-[15px] leading-relaxed text-ink-2">{subtitle}</p>}
          <div className="mt-8 rounded-xl border border-line-strong bg-surface/90 p-6 shadow-[0_24px_80px_rgb(0_0_0/0.35)] backdrop-blur-2xl">
            {children}
          </div>
          {below}
          {footer && <div className="mt-5 text-center text-[13px] text-ink-3">{footer}</div>}
        </div>
      </div>
    </div>
  );
}

function useSubmit<T>(fn: () => Promise<T>, done: (r: T) => void) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiFailure | null>(null);
  const submit = async (e: FormEvent) => {
    e.preventDefault();
    setBusy(true);
    setError(null);
    try {
      done(await fn());
    } catch (err) {
      setError(err instanceof ApiFailure ? err : new ApiFailure(0, { message: String(err) }));
    } finally {
      setBusy(false);
    }
  };
  return { busy, error, submit, fieldError: (f: string) => (error?.field === f ? error.message : undefined) };
}

/** Into the app. Unless a passkey did it, the app then offers to make one (`components/passkeys/offer`). */
function signedIn(me: G.Me, next?: string, { passkey = false } = {}) {
  queryClient.setQueryData(keys.me, me);
  if (!passkey) sessionStorage.setItem(OFFER_PASSKEY, "1");
  window.location.assign(appPath(next));
}

/** Signs in with what the second step answered, and remembers to say how many recovery codes are left. */
function finished(r: G.AuthResponse, next?: string) {
  if (r.recovery_codes_left !== undefined) {
    sessionStorage.setItem(RECOVERY_LEFT, String(r.recovery_codes_left));
  }
  if (r.me) signedIn(r.me, r.next ?? next);
}

function GoogleButton({ label, next }: { label: string; next?: string }) {
  const back = appPath(next) !== "/" ? `&return_to=${encodeURIComponent(appPath(next))}` : "";
  return (
    <a
      href={`/api/v1/auth/google/start?intent=login${back}`}
      className="flex h-10 w-full items-center justify-center gap-2.5 rounded-lg border border-line-strong bg-raised/80 text-[14px] font-medium text-ink transition-colors hover:border-ink-4"
    >
      <svg viewBox="0 0 24 24" className="size-4" aria-hidden="true">
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
      {label}
    </a>
  );
}

/** On the sign-in page, where the instance takes requests: the way in for people without an account. */
function EarlyAccessLink() {
  return (
    <span className="inline-flex flex-wrap items-center justify-center gap-x-2 gap-y-1.5">
      No account yet?
      <Link
        to="/early-access"
        className="group inline-flex h-10 items-center gap-2 rounded-full border border-line-strong bg-surface/85 pr-3.5 pl-3 text-[14px] font-medium text-ink backdrop-blur-xl transition-colors hover:border-ink-4"
      >
        <Sparkles className="size-4 fill-highlight text-accent" />
        Request early access
        <ArrowRight className="size-4 text-ink-3 transition-transform duration-200 group-hover:translate-x-0.5 group-hover:text-ink motion-reduce:transition-none" />
      </Link>
    </span>
  );
}

export function LoginPage() {
  const search = useSearch({ from: "/login" });
  const { data: instance } = useInstance();
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const [forgot, setForgot] = useState(false);
  const [sent, setSent] = useState(false);
  const [challenge, setChallenge] = useState<G.TwoFactorChallenge | null>(null);
  const { busy, error, submit } = useSubmit(
    () => api.login({ email, password }),
    (r) => {
      if (r.two_factor) setChallenge(r.two_factor);
      else if (r.me) signedIn(r.me, search.next);
    },
  );
  const reset = useSubmit(
    () => api.forgotPassword({ email }),
    () => {
      setForgot(false);
      setSent(true);
    },
  );
  const passkey = usePasskeySignIn((r) => r.me && signedIn(r.me, search.next, { passkey: true }));
  const external = search.error
    ? (GOOGLE_ERRORS[search.error] ?? "The sign-in did not work. Try again.")
    : null;
  if (challenge) {
    const copy = challengeCopy(challenge, instance?.name);
    return (
      <AuthFrame title={copy.title} subtitle={copy.subtitle}>
        <SecondFactorStep
          key={challenge.expires_at}
          challenge={challenge}
          onSignedIn={(r) => finished(r, search.next)}
          onRestart={() => {
            setChallenge(null);
            setPassword("");
          }}
        />
      </AuthFrame>
    );
  }
  return (
    <AuthFrame
      title={<>Welcome back.</>}
      subtitle="Agents are building while you're away. Sign in to see what they've done."
      footer={
        instance?.features.early_access ? (
          <EarlyAccessLink />
        ) : instance && !instance.features.email ? (
          "Need an account or a new password? Ask an admin of this instance."
        ) : undefined
      }
    >
      {search.deleted && !external && (
        <Banner tone="ok" className="mb-4">
          Your account was deleted.
        </Banner>
      )}
      {external && (
        <Banner tone="error" className="mb-4">
          {external}
        </Banner>
      )}
      {(instance?.features.google || passkey.supported) && (
        <>
          <div className="flex flex-col gap-2.5">
            {instance?.features.google && <GoogleButton label="Continue with Google" next={search.next} />}
            {passkey.supported && (
              <button
                type="button"
                onClick={passkey.start}
                disabled={!passkey.ready || passkey.busy}
                className="flex h-10 w-full items-center justify-center gap-2.5 rounded-lg border border-line-strong bg-raised/80 text-[14px] font-medium text-ink transition-colors hover:border-ink-4 disabled:opacity-60"
              >
                <Fingerprint className={cn("size-4", passkey.busy && "animate-pulse")} />
                Sign in with a passkey
              </button>
            )}
          </div>
          {passkey.error && (
            <Banner tone="error" className="mt-3">
              {passkey.error}
            </Banner>
          )}
          <div className="my-5 flex items-center gap-3 text-[12px] text-ink-4">
            <div className="h-px flex-1 bg-line" /> or <div className="h-px flex-1 bg-line" />
          </div>
        </>
      )}
      <form onSubmit={submit} className="flex flex-col gap-4">
        <Field label="Email">
          {(id) => (
            <Input
              id={id}
              type="email"
              autoComplete="username webauthn"
              required
              value={email}
              onChange={(e) => setEmail(e.target.value)}
              autoFocus
            />
          )}
        </Field>
        <Field
          label="Password"
          action={
            // Without email, where the footer is the way to early access, it says whom to ask.
            instance?.features.email || instance?.features.early_access ? (
              <button
                type="button"
                className="text-[12px] text-ink-3 hover:text-ink"
                onClick={() => setForgot(true)}
              >
                Forgot?
              </button>
            ) : undefined
          }
        >
          {(id) => (
            <Input
              id={id}
              type="password"
              autoComplete="current-password"
              required
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          )}
        </Field>
        {error && <Banner tone="error">{error.message}</Banner>}
        {sent && <Banner tone="ok">If {email} has an account, a reset link is on its way.</Banner>}
        {forgot && !instance?.features.email && (
          <Banner tone="info">Ask an admin of this instance for a link to choose a new password.</Banner>
        )}
        {forgot && instance?.features.email && (
          <Banner tone="info">
            We'll email a reset link to {email || "your address"}.{" "}
            <button
              type="button"
              className="font-medium text-ink underline underline-offset-2"
              onClick={(e) => void reset.submit(e as unknown as FormEvent)}
            >
              Send it
            </button>
          </Banner>
        )}
        <Button
          type="submit"
          variant="primary"
          size="lg"
          loading={busy}
          trailing={<ArrowRight className="size-4" />}
        >
          Sign in
        </Button>
      </form>
    </AuthFrame>
  );
}

export function SetupPage() {
  const navigate = useNavigate();
  const token = inviteToken();
  const [given, setGiven] = useState("");
  const [family, setFamily] = useState("");
  const [email, setEmail] = useState("");
  const [password, setPassword] = useState("");
  const { busy, error, submit, fieldError } = useSubmit(
    () => api.setup({ token, given_name: given, family_name: family, email, password }),
    (r) => r.me && signedIn(r.me),
  );
  const { data: instance } = useInstance();
  if (instance && !instance.setup_required) {
    return (
      <AuthFrame title="Already set up." subtitle="This instance has an owner.">
        <Button
          variant="primary"
          size="lg"
          className="w-full"
          onClick={() =>
            navigate({ to: "/login", search: { next: undefined, error: undefined, deleted: undefined } })
          }
        >
          Sign in
        </Button>
      </AuthFrame>
    );
  }
  return (
    <AuthFrame title="Start your factory." subtitle="Create the owner account. You decide who else gets in.">
      {!token && (
        <Banner tone="warn" className="mb-4">
          Open the setup link from your deployment (it ends in #token=…).
        </Banner>
      )}
      <form onSubmit={submit} className="flex flex-col gap-4">
        <NameFields
          given={given}
          family={family}
          onGiven={setGiven}
          onFamily={setFamily}
          error={fieldError}
          autoFocus
        />
        <Field label="Email" error={fieldError("email")}>
          {(id) => (
            <Input
              id={id}
              type="email"
              required
              autoComplete="email"
              value={email}
              onChange={(e) => setEmail(e.target.value)}
            />
          )}
        </Field>
        <Field label="Password" hint="At least 10 characters." error={fieldError("password")}>
          {(id) => (
            <Input
              id={id}
              type="password"
              required
              autoComplete="new-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          )}
        </Field>
        {error && !error.field && <Banner tone="error">{error.message}</Banner>}
        <Button
          type="submit"
          variant="primary"
          size="lg"
          loading={busy}
          disabled={!token}
          trailing={<ArrowRight className="size-4" />}
        >
          Create the owner account
        </Button>
      </form>
    </AuthFrame>
  );
}

export function InvitePage() {
  const token = inviteToken();
  const preview = useQuery({
    queryKey: ["invite", token],
    queryFn: () => api.invitePreview(token),
    enabled: !!token,
    retry: false,
  });
  // Signed in already: the address can join this account instead.
  const { data: me } = useMe();
  const [separate, setSeparate] = useState(false);
  const [given, setGiven] = useState("");
  const [family, setFamily] = useState("");
  const [password, setPassword] = useState("");
  const email = preview.data?.email ?? "";
  const { busy, error, submit, fieldError } = useSubmit(
    () => api.signup({ email, given_name: given, family_name: family, password, invite: token }),
    (r) => {
      used();
      if (r.me) signedIn(r.me);
    },
  );
  const connect = useMutation({
    mutationFn: () => emails.connectInvite(token),
    onSuccess: (m) => {
      used();
      signedIn(m);
    },
  });
  const { data: instance } = useInstance();
  useEffect(() => {
    if (preview.isError) forgetInvite();
  }, [preview.isError]);
  if (!token || preview.isError) {
    return (
      <AuthFrame
        title="This invitation has expired."
        subtitle="Ask the person who invited you for a new link."
      >
        <Button
          variant="secondary"
          size="lg"
          className="w-full"
          onClick={() => window.location.assign("/login")}
        >
          Go to sign-in
        </Button>
      </AuthFrame>
    );
  }
  const joins = preview.data?.workspaces ?? [];
  const workspacesLine = joins.length > 0 && (
    <div className="mb-6 flex flex-col gap-2 rounded-xl border border-line bg-sunken/40 px-4 py-3">
      <div className="text-[12.5px] text-ink-3">
        You join {joins.length === 1 ? "this workspace" : "these workspaces"} with it
      </div>
      <ul className="flex flex-col gap-2">
        {joins.map((w) => (
          <li key={w.id} className="flex items-center gap-2.5 text-[14px] text-ink">
            <WorkspaceAvatar workspace={w} size={22} /> {w.name}
          </li>
        ))}
      </ul>
    </div>
  );
  const subtitle = preview.data ? (
    <>
      {preview.data.invited_by ?? "An admin"} invited {email} to {preview.data.instance}, where Claude agents
      build software.
    </>
  ) : (
    "Checking your invitation…"
  );
  // The answer to their request for early access.
  const title = preview.data?.early_access ? (
    // As the page where they asked said it (routes/early-access.tsx).
    <>
      Your <span className="title-hot">early access</span> is here<span className="title-dot">.</span>
    </>
  ) : (
    <>You're invited.</>
  );
  if (me && !separate) {
    return (
      <AuthFrame title={title} subtitle={subtitle}>
        {workspacesLine}
        <div className="flex flex-col gap-4">
          <div className="flex items-center gap-3 rounded-xl border border-line px-4 py-3">
            <Avatar user={me.user} size={32} />
            <div className="min-w-0 text-[13px]">
              <div className="truncate text-ink">{me.user.name}</div>
              <div className="truncate text-ink-4">Signed in as {me.user.email}</div>
            </div>
          </div>
          <p className="text-[13.5px] leading-relaxed text-ink-3">
            Connect <span className="text-ink">{email}</span> to this account: it signs in to it too, has its
            own commit email, and the workspaces you join as it see only it. Connect it only if the address is
            yours: whoever has it can reset this account's password.
          </p>
          {connect.error && <Banner tone="error">{connect.error.message}</Banner>}
          <Button
            variant="primary"
            size="lg"
            loading={connect.isPending}
            disabled={!preview.data}
            onClick={() => connect.mutate()}
            trailing={<ArrowRight className="size-4" />}
          >
            Connect it to my account
          </Button>
          <button
            type="button"
            onClick={() => setSeparate(true)}
            className="text-[13px] text-ink-3 underline-offset-2 hover:text-ink hover:underline"
          >
            Create a separate account for it instead
          </button>
        </div>
      </AuthFrame>
    );
  }
  return (
    <AuthFrame
      title={title}
      subtitle={subtitle}
      footer={
        !me && (
          <span>
            Have an account already?{" "}
            <a
              // The token waits in this tab, not in the sign-in page's address.
              href={`/login?next=${encodeURIComponent("/invite")}`}
              onClick={() => {
                try {
                  sessionStorage.setItem(PENDING_INVITE, token);
                } catch {}
              }}
              className="text-ink underline underline-offset-2"
            >
              Sign in to connect this address to it
            </a>
          </span>
        )
      }
    >
      {workspacesLine}
      {instance?.features.google && (
        <>
          <GoogleButton label={`Continue with Google as ${email}`} />
          <div className="my-5 flex items-center gap-3 text-[12px] text-ink-4">
            <div className="h-px flex-1 bg-line" /> or set a password <div className="h-px flex-1 bg-line" />
          </div>
        </>
      )}
      <form onSubmit={submit} className="flex flex-col gap-4">
        <Field label="Email">{(id) => <Input id={id} value={email} readOnly className="text-ink-3" />}</Field>
        <NameFields
          given={given}
          family={family}
          onGiven={setGiven}
          onFamily={setFamily}
          error={fieldError}
          autoFocus
        />
        <Field label="Password" hint="At least 10 characters." error={fieldError("password")}>
          {(id) => (
            <Input
              id={id}
              type="password"
              required
              autoComplete="new-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
            />
          )}
        </Field>
        {error && !error.field && <Banner tone="error">{error.message}</Banner>}
        <Button
          type="submit"
          variant="primary"
          size="lg"
          loading={busy}
          disabled={!preview.data}
          trailing={<ArrowRight className="size-4" />}
        >
          Create account
        </Button>
      </form>
    </AuthFrame>
  );
}

export function ResetPage() {
  const token = inviteToken();
  const { data: instance } = useInstance();
  const [password, setPassword] = useState("");
  const [challenge, setChallenge] = useState<G.TwoFactorChallenge | null>(null);
  const { busy, error, submit, fieldError } = useSubmit(
    () => api.resetPassword({ token, new_password: password }),
    (r) => {
      if (r.two_factor) setChallenge(r.two_factor);
      else if (r.me) signedIn(r.me);
    },
  );
  if (challenge) {
    const copy = challengeCopy(challenge, instance?.name);
    return (
      <AuthFrame title={copy.title} subtitle={copy.subtitle}>
        <SecondFactorStep
          key={challenge.expires_at}
          challenge={challenge}
          onSignedIn={(r) => finished(r)}
          onRestart={() => setChallenge(null)}
        />
      </AuthFrame>
    );
  }
  return (
    <AuthFrame
      title="Choose a new password."
      subtitle="You'll be signed in here, and signed out everywhere else."
    >
      <form onSubmit={submit} className="flex flex-col gap-4">
        <Field
          label="New password"
          hint="At least 10 characters."
          error={fieldError("new_password") ?? fieldError("password")}
        >
          {(id) => (
            <Input
              id={id}
              type="password"
              required
              autoComplete="new-password"
              value={password}
              onChange={(e) => setPassword(e.target.value)}
              autoFocus
            />
          )}
        </Field>
        {error && !error.field && <Banner tone="error">{error.message}</Banner>}
        <Button
          type="submit"
          variant="primary"
          size="lg"
          loading={busy}
          disabled={!token}
          trailing={<ArrowRight className="size-4" />}
        >
          Save and sign in
        </Button>
      </form>
    </AuthFrame>
  );
}

/** After Google, for accounts whose Google sign-ins ask for a code: the second step. */
export function TwoFactorPage() {
  const { data: instance } = useInstance();
  const challenge = useQuery({
    queryKey: ["two-factor-challenge"],
    queryFn: api.twoFactorChallenge,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const restart = () => window.location.assign("/login");
  if (challenge.isPending)
    return (
      <AuthFrame title="One more step." subtitle="Checking your sign-in…">
        <div className="flex justify-center py-6">
          <Spinner className="size-5 text-ink-3" />
        </div>
      </AuthFrame>
    );
  if (!challenge.data)
    return (
      <AuthFrame title="This sign-in took too long." subtitle="Sign in again: it takes a new code.">
        <Button variant="primary" size="lg" className="w-full" onClick={restart}>
          Go to sign-in
        </Button>
      </AuthFrame>
    );
  const copy = challengeCopy(challenge.data, instance?.name);
  return (
    <AuthFrame title={copy.title} subtitle={copy.subtitle}>
      <SecondFactorStep challenge={challenge.data} onSignedIn={(r) => finished(r)} onRestart={restart} />
    </AuthFrame>
  );
}

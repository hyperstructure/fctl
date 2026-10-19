// Sign-in, setup, invitations and password resets, and the second step of
// a sign-in with two-factor.

import { foundry } from "@hyperstructure/design/gl/foundry";
import { useQuery } from "@tanstack/react-query";
import { useNavigate, useSearch } from "@tanstack/react-router";
import { ArrowRight, Fingerprint } from "lucide-react";
import { type FormEvent, type ReactNode, useState } from "react";
import { usePasskeySignIn } from "@/components/passkeys/sign-in";
import { challengeCopy, SecondFactorStep } from "@/components/two-factor/challenge";
import { Banner, Button, Field, GLCanvas, Input, Logo, NameFields, Spinner } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { OFFER_PASSKEY } from "@/lib/passkeys";
import { keys, queryClient, useInstance } from "@/lib/queries";
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

function hashToken(): string {
  return new URLSearchParams(window.location.hash.slice(1)).get("token") ?? "";
}

export function AuthFrame({
  title,
  subtitle,
  children,
  footer,
  wide,
}: {
  title: ReactNode;
  subtitle?: ReactNode;
  children: ReactNode;
  footer?: ReactNode;
  /** Room for more than a form (setting up two-factor). */
  wide?: boolean;
}) {
  const { data: instance } = useInstance();
  return (
    <div className="relative flex min-h-dvh flex-col overflow-hidden bg-canvas">
      <div className="absolute inset-0">
        <GLCanvas
          scene={() => foundry({ planet: [1.02, -0.75, 1.15], density: 0.6 }).scene}
          options={{ pixel: 3, fps: 30 }}
        />
      </div>
      <div className="relative z-10 flex items-center justify-between px-6 py-5">
        <Logo />
        <span className="text-[12.5px] text-ink-4">{instance?.name}</span>
      </div>
      <div className="relative z-10 flex flex-1 items-center justify-center px-5 pb-16">
        <div className={cn("w-full animate-rise", wide ? "max-w-[720px]" : "max-w-[400px]")}>
          <h1 className="font-display text-[34px] leading-[1.1] font-medium tracking-[-0.02em] text-ink">
            {title}
          </h1>
          {subtitle && <p className="mt-3 text-[15px] leading-relaxed text-ink-2">{subtitle}</p>}
          <div className="mt-8 rounded-xl border border-line-strong bg-surface/90 p-6 shadow-[0_24px_80px_rgb(0_0_0/0.35)] backdrop-blur-2xl">
            {children}
          </div>
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

/** Where a sign-in goes: `next` when it is a path of the app, else the floor. */
function safeNext(next?: string) {
  return next?.startsWith("/") && !next.startsWith("//") ? next : "/";
}

/** Into the app. Unless a passkey did it, the app then offers to make one (`components/passkeys/offer`). */
function signedIn(me: G.Me, next?: string, { passkey = false } = {}) {
  queryClient.setQueryData(keys.me, me);
  if (!passkey) sessionStorage.setItem(OFFER_PASSKEY, "1");
  window.location.assign(safeNext(next));
}

/** Signs in with what the second step answered, and remembers to say how many recovery codes are left. */
function finished(r: G.AuthResponse, next?: string) {
  if (r.recovery_codes_left !== undefined) {
    sessionStorage.setItem(RECOVERY_LEFT, String(r.recovery_codes_left));
  }
  if (r.me) signedIn(r.me, r.next ?? next);
}

function GoogleButton({ label }: { label: string }) {
  return (
    <a
      href="/api/v1/auth/google/start?intent=login"
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
        instance && !instance.features.email
          ? "Need an account or a new password? Ask an admin of this instance."
          : undefined
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
            {instance?.features.google && <GoogleButton label="Continue with Google" />}
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
            instance?.features.email ? (
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
        {forgot && (
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
  const token = hashToken();
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
  const token = hashToken();
  const preview = useQuery({
    queryKey: ["invite", token],
    queryFn: () => api.invitePreview(token),
    enabled: !!token,
    retry: false,
  });
  const [given, setGiven] = useState("");
  const [family, setFamily] = useState("");
  const [password, setPassword] = useState("");
  const email = preview.data?.email ?? "";
  const { busy, error, submit, fieldError } = useSubmit(
    () => api.signup({ email, given_name: given, family_name: family, password, invite: token }),
    (r) => r.me && signedIn(r.me),
  );
  const { data: instance } = useInstance();
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
  return (
    <AuthFrame
      title={<>You're invited.</>}
      subtitle={
        preview.data ? (
          <>
            {preview.data.invited_by ?? "An admin"} invited you to {preview.data.instance}, where Claude
            agents build software.
          </>
        ) : (
          "Checking your invitation…"
        )
      }
    >
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
  const token = hashToken();
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

// The second step of a sign-in: a code from the authenticator app, a
// passkey, or a recovery code. The login and reset pages show it in place of
// their form, and /two-factor shows it after Google.

import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowRight, Clock, Fingerprint } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { usePasskeySupport } from "@/components/passkeys/settings";
import { Avatar, Banner, Button, CodeInput, Input } from "@/components/ui";
import type * as G from "@/gen";
import { type ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { cancelled, getPasskey, passkeyError, rpId, signalUnknown } from "@/lib/passkeys";
import { clock, formatRecoveryCode, isRecoveryCode } from "@/lib/two-factor";

type Mode = "app" | "passkey" | "recovery";

/** The page's title and subtitle while a sign-in waits for its second factor. */
export function challengeCopy(c: G.TwoFactorChallenge | null | undefined, instance?: string) {
  if (c && !c.methods.includes("totp") && c.methods.includes("passkey")) {
    switch (c.reason) {
      case "password_reset":
        return {
          title: "Confirm it's you.",
          subtitle: "Your new password takes effect once you confirm it's you with your passkey.",
        };
      case "google":
        return {
          title: "One more step.",
          subtitle: "Google signed you in. Now confirm it's you with your passkey.",
        };
      default:
        return { title: "One more step.", subtitle: "Confirm it's you with your passkey." };
    }
  }
  const it = instance ? `the code it shows for ${instance}` : "the code it shows";
  switch (c?.reason) {
    case "password_reset":
      return {
        title: "Confirm it's you.",
        subtitle: `Your new password takes effect once you enter the code your authenticator app shows${instance ? ` for ${instance}` : ""}.`,
      };
    case "google":
      return {
        title: "One more step.",
        subtitle: `Google signed you in. Now open your authenticator app and enter ${it}.`,
      };
    default:
      return { title: "Enter your code.", subtitle: `Open your authenticator app and enter ${it}.` };
  }
}

/** Re-renders every second while `until` is in the future; the time now. */
function useNow(until?: number) {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!until || until <= Date.now()) return;
    const t = setInterval(() => {
      setNow(Date.now());
      if (Date.now() >= until) clearInterval(t);
    }, 250);
    return () => clearInterval(t);
  }, [until]);
  return now;
}

export function SecondFactorStep({
  challenge,
  onSignedIn,
  onRestart,
}: {
  challenge: G.TwoFactorChallenge;
  onSignedIn: (r: G.AuthResponse) => void;
  /** Back to the first step: the sign-in is forgotten. */
  onRestart: () => void;
}) {
  const has = (m: G.SecondFactor) => challenge.methods.includes(m);
  const [mode, setMode] = useState<Mode>(has("totp") ? "app" : has("passkey") ? "passkey" : "recovery");
  const support = usePasskeySupport();
  const passkeys = has("passkey") && !!support?.supported;
  // Fetched ahead, so that the click opens the browser's passkey sheet at once.
  const passkeyOptions = useQuery({
    queryKey: ["two-factor-passkey", challenge.expires_at],
    queryFn: api.twoFactorPasskeyOptions,
    enabled: passkeys,
    retry: false,
    staleTime: 8 * 60_000,
    refetchOnWindowFocus: false,
  });
  const [passkeyFailure, setPasskeyFailure] = useState<string | null>(null);
  // The passkey that answered last, for its provider to forget if fctl doesn't know it.
  const answered = useRef<string | null>(null);
  const [code, setCode] = useState("");
  const [recovery, setRecovery] = useState("");
  const [errors, setErrors] = useState(0);
  const [lockedUntil, setLockedUntil] = useState(challenge.locked_until);
  const [expired, setExpired] = useState(false);
  const now = useNow(lockedUntil);
  const locked = !!lockedUntil && lockedUntil > now;
  const codeRef = useRef<HTMLInputElement>(null);
  const recoveryRef = useRef<HTMLInputElement>(null);
  const hintId = useId();
  const verify = useMutation({
    mutationFn: (b: G.VerifyTwoFactorRequest) => api.verifyTwoFactor(b),
    onSuccess: (r) => setTimeout(() => onSignedIn(r), 420),
    onError: (e: ApiFailure) => {
      if (e.code === "rate_limited" && e.retryAfter) {
        setLockedUntil(Date.now() + e.retryAfter * 1000);
        setCode("");
        setRecovery("");
      } else if (e.code === "unauthorized") setExpired(true);
      else if (e.field === "passkey" || e.status === 404) {
        if (e.status === 404 && answered.current) signalUnknown(rpId(), answered.current);
        setPasskeyFailure(e.message);
      } else if (e.field === "code") {
        setErrors((n) => n + 1);
        setCode("");
        setRecovery("");
      }
    },
  });
  // Ten minutes after the password, the sign-in is gone.
  useEffect(() => {
    const t = setTimeout(() => setExpired(true), Math.max(0, challenge.expires_at - Date.now()));
    return () => clearTimeout(t);
  }, [challenge.expires_at]);
  // Back from a wait: the field takes codes again.
  useEffect(() => {
    if (!locked) (mode === "app" ? codeRef : mode === "recovery" ? recoveryRef : null)?.current?.focus();
  }, [locked, mode]);
  const error = verify.error as ApiFailure | null;
  const cancel = useMutation({ mutationFn: api.cancelTwoFactor, onSettled: onRestart });
  // The wait holds back codes from the app; recovery codes still work.
  const send = (c: string) => {
    if (!verify.isPending && !verify.isSuccess && !(locked && mode === "app")) verify.mutate({ code: c });
  };
  const usePasskey = () => {
    const options = passkeyOptions.data;
    if (!options || verify.isPending) return;
    setPasskeyFailure(null);
    verify.reset();
    getPasskey(options)
      .then((passkey) => {
        answered.current = passkey.id;
        // Each challenge works once: another try takes a new one.
        verify.mutate({ passkey }, { onError: () => void passkeyOptions.refetch() });
      })
      .catch((e) => {
        if (!cancelled(e)) setPasskeyFailure(passkeyError(e));
        void passkeyOptions.refetch();
      });
  };
  const switchTo = (m: Mode) => {
    setMode(m);
    setPasskeyFailure(null);
    verify.reset();
  };

  if (expired)
    return (
      <div className="flex flex-col gap-4">
        <Banner tone="warn">
          This sign-in took too long. Start again: it takes your password, then a new code.
        </Banner>
        <Button variant="primary" size="lg" onClick={() => cancel.mutate()} loading={cancel.isPending}>
          Start again
        </Button>
      </div>
    );
  if (!has("totp") && !has("recovery_code") && support && !support.supported)
    return (
      <div className="flex flex-col gap-4">
        <Banner tone="warn">
          Your account's second factor is a passkey, and this browser can't use passkeys. Sign in in another
          browser, or ask an admin to reset your two-factor sign-in.
        </Banner>
        <Button variant="secondary" size="lg" onClick={() => cancel.mutate()} loading={cancel.isPending}>
          Start again
        </Button>
      </div>
    );

  const state = verify.isSuccess
    ? "ok"
    : verify.isPending
      ? "busy"
      : error?.field === "code"
        ? "error"
        : "idle";
  return (
    <div className="flex animate-rise flex-col gap-5">
      <div className="flex items-center gap-3 rounded-lg border border-line bg-sunken/50 px-3 py-2.5">
        <Avatar
          user={{ id: challenge.email, name: challenge.name, avatar_url: challenge.avatar_url }}
          size={30}
        />
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13.5px] text-ink">{challenge.name}</div>
          <div className="truncate text-[12px] text-ink-4">{challenge.email}</div>
        </div>
        <button
          type="button"
          className="shrink-0 rounded px-1.5 py-1 text-[12px] text-ink-3 hover:bg-hover hover:text-ink"
          onClick={() => cancel.mutate()}
        >
          Not you?
        </button>
      </div>

      {mode === "passkey" ? (
        <div className="flex flex-col items-center gap-3 py-1">
          <span
            className={cn(
              "flex size-14 items-center justify-center rounded-full bg-accent-soft text-accent transition-transform duration-300",
              verify.isPending && "animate-pulse",
              verify.isSuccess && "scale-110",
            )}
          >
            <Fingerprint className="size-7" />
          </span>
          <Button
            variant="primary"
            size="lg"
            className="w-full"
            onClick={usePasskey}
            loading={verify.isPending}
            disabled={!passkeyOptions.data || verify.isSuccess}
            icon={verify.isPending ? undefined : <Fingerprint className="size-4" />}
          >
            Use your passkey
          </Button>
          <p
            className={cn("text-center text-[12.5px]", passkeyFailure ? "text-danger" : "text-ink-4")}
            role={passkeyFailure ? "alert" : undefined}
          >
            {passkeyFailure ??
              passkeyOptions.error?.message ??
              "Your browser asks for your fingerprint, face or screen lock."}
          </p>
        </div>
      ) : mode === "app" ? (
        <form
          className="flex flex-col items-center gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (code.length === 6) send(code);
          }}
        >
          <CodeInput
            ref={codeRef}
            value={code}
            onChange={setCode}
            onComplete={send}
            state={state}
            errors={errors}
            disabled={locked || verify.isSuccess}
            label="The six-digit code from your authenticator app"
            describedBy={hintId}
            autoFocus
          />
          <Hint
            id={hintId}
            error={error}
            errors={errors}
            locked={locked}
            lockedUntil={lockedUntil}
            now={now}
            onRecovery={has("recovery_code") ? () => switchTo("recovery") : undefined}
          />
          <button type="submit" className="sr-only">
            Sign in
          </button>
        </form>
      ) : (
        <form
          className="flex flex-col gap-3"
          onSubmit={(e) => {
            e.preventDefault();
            if (isRecoveryCode(recovery)) send(recovery);
          }}
        >
          <label htmlFor={`${hintId}-r`} className="text-[12.5px] font-medium text-ink-2">
            Recovery code
          </label>
          <Input
            ref={recoveryRef}
            id={`${hintId}-r`}
            value={recovery}
            onChange={(e) => setRecovery(formatRecoveryCode(e.target.value))}
            placeholder="XXXXX-XXXXX"
            autoComplete="off"
            autoCapitalize="characters"
            spellCheck={false}
            disabled={verify.isSuccess}
            aria-invalid={error?.field === "code" || undefined}
            aria-describedby={hintId}
            className="h-11 text-center font-mono text-[17px] tracking-[0.12em] uppercase"
          />
          <Hint
            id={hintId}
            error={error}
            errors={errors}
            locked={locked}
            lockedUntil={lockedUntil}
            now={now}
            recovery
          />
          <Button
            type="submit"
            variant="primary"
            size="lg"
            loading={verify.isPending}
            disabled={!isRecoveryCode(recovery) || verify.isSuccess}
            trailing={<ArrowRight className="size-4" />}
          >
            Sign in
          </Button>
        </form>
      )}

      {error && !error.field && error.code !== "rate_limited" && error.code !== "unauthorized" && (
        <Banner tone="error">{error.message}</Banner>
      )}

      <div className="flex flex-col items-center gap-1.5 text-center text-[12.5px] text-ink-3">
        {mode !== "passkey" && passkeys && (
          <button
            type="button"
            className="group inline-flex items-center gap-1.5 hover:text-ink"
            onClick={() => switchTo("passkey")}
          >
            <Fingerprint className="size-3.5" />
            <span className="text-ink-2 underline underline-offset-2 group-hover:text-ink">
              Use a passkey instead
            </span>
          </button>
        )}
        {mode !== "app" && has("totp") && (
          <button type="button" className="hover:text-ink" onClick={() => switchTo("app")}>
            <span className="text-ink-2 underline underline-offset-2">Use your authenticator app</span>{" "}
            instead
          </button>
        )}
        {mode !== "recovery" && has("recovery_code") && (
          <button type="button" className="hover:text-ink" onClick={() => switchTo("recovery")}>
            {mode === "app" ? "Lost your phone? " : ""}
            <span className="text-ink-2 underline underline-offset-2">Use a recovery code</span>
          </button>
        )}
      </div>
    </div>
  );
}

function Hint({
  id,
  error,
  errors,
  locked,
  lockedUntil,
  now,
  recovery,
  onRecovery,
}: {
  id: string;
  error: ApiFailure | null;
  errors: number;
  locked: boolean;
  lockedUntil?: number;
  now: number;
  recovery?: boolean;
  onRecovery?: () => void;
}) {
  if (locked && lockedUntil && !recovery)
    return (
      <p id={id} className="flex items-center gap-1.5 text-[12.5px] text-amber" role="status">
        <Clock className="size-3.5 shrink-0" />
        <span>
          Too many wrong codes. Try again in{" "}
          <span className="font-mono tabular-nums">{clock(lockedUntil - now)}</span>, or{" "}
          <button type="button" className="underline underline-offset-2 hover:text-ink" onClick={onRecovery}>
            use a recovery code
          </button>
          .
        </span>
      </p>
    );
  if (error?.field === "code")
    return (
      <p id={id} className="max-w-[340px] text-center text-[12.5px] leading-relaxed text-danger" role="alert">
        {recovery
          ? "That recovery code doesn't work: it's wrong, or it was used before."
          : errors > 1 && !error.message.includes("just used")
            ? "That isn't right either. Codes depend on the time: check that your phone sets its clock automatically."
            : error.message}
      </p>
    );
  return (
    <p id={id} className={cn("text-[12.5px] text-ink-4", !recovery && "text-center")}>
      {recovery
        ? "One of the ten codes you saved when you set up two-factor. Each works once."
        : "It changes every 30 seconds."}
    </p>
  );
}

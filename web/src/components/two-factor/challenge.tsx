// The second step of a sign-in: a code from the authenticator app, or a
// recovery code. The login and reset pages show it in place of their form,
// and /two-factor shows it after Google.

import { useMutation } from "@tanstack/react-query";
import { ArrowRight, Clock } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import { Avatar, Banner, Button, CodeInput, Input } from "@/components/ui";
import type * as G from "@/gen";
import { type ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { clock, formatRecoveryCode, isRecoveryCode } from "@/lib/two-factor";

/** The page's title and subtitle while a sign-in waits for its code. */
export function challengeCopy(c: G.TwoFactorChallenge | null | undefined, instance?: string) {
  const app = instance
    ? `the code your authenticator app shows for ${instance}`
    : "the code your authenticator app shows";
  switch (c?.reason) {
    case "password_reset":
      return {
        title: "Confirm it's you.",
        subtitle: `Your new password takes effect once you enter ${app}.`,
      };
    case "google":
      return { title: "One more step.", subtitle: `Google signed you in. Now enter ${app}.` };
    default:
      return { title: "Enter your code.", subtitle: `Open your authenticator app and enter ${app}.` };
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
  const [mode, setMode] = useState<"app" | "recovery">("app");
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
    mutationFn: (c: string) => api.verifyTwoFactor({ code: c }),
    onSuccess: (r) => setTimeout(() => onSignedIn(r), 420),
    onError: (e: ApiFailure) => {
      if (e.code === "rate_limited" && e.retryAfter) setLockedUntil(Date.now() + e.retryAfter * 1000);
      else if (e.code === "unauthorized") setExpired(true);
      else if (e.field === "code") {
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
    if (!locked) (mode === "app" ? codeRef : recoveryRef).current?.focus();
  }, [locked, mode]);
  const error = verify.error as ApiFailure | null;
  const cancel = useMutation({ mutationFn: api.cancelTwoFactor, onSettled: onRestart });
  const send = (c: string) => {
    if (!verify.isPending && !verify.isSuccess && !locked) verify.mutate(c);
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

  const state = verify.isSuccess
    ? "ok"
    : verify.isPending
      ? "busy"
      : error?.field === "code"
        ? "error"
        : "idle";
  return (
    <div className="flex flex-col gap-5">
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

      {mode === "app" ? (
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
            disabled={locked || verify.isSuccess}
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
            disabled={!isRecoveryCode(recovery) || locked || verify.isSuccess}
            trailing={<ArrowRight className="size-4" />}
          >
            Sign in
          </Button>
        </form>
      )}

      {error && !error.field && error.code !== "rate_limited" && error.code !== "unauthorized" && (
        <Banner tone="error">{error.message}</Banner>
      )}

      <div className="text-center text-[12.5px] text-ink-3">
        {mode === "app" ? (
          <button
            type="button"
            className="hover:text-ink"
            onClick={() => {
              setMode("recovery");
              verify.reset();
            }}
          >
            Lost your phone?{" "}
            <span className="text-ink-2 underline underline-offset-2">Use a recovery code</span>
          </button>
        ) : (
          <button
            type="button"
            className="hover:text-ink"
            onClick={() => {
              setMode("app");
              verify.reset();
            }}
          >
            <span className="text-ink-2 underline underline-offset-2">Use your authenticator app</span>{" "}
            instead
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
}: {
  id: string;
  error: ApiFailure | null;
  errors: number;
  locked: boolean;
  lockedUntil?: number;
  now: number;
  recovery?: boolean;
}) {
  if (locked && lockedUntil)
    return (
      <p id={id} className="flex items-center gap-1.5 text-[12.5px] text-amber" role="status">
        <Clock className="size-3.5" />
        Too many wrong codes. Try again in{" "}
        <span className="font-mono tabular-nums">{clock(lockedUntil - now)}</span>
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

// Confirming it's you before sensitive actions, where the instance asks for
// it: a request the server answers with `verification_required` opens this
// dialog (lib/api.ts), and goes again once the person confirmed with their
// passkey, a code from their authenticator app or a recovery code.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Fingerprint } from "lucide-react";
import { useCallback, useEffect, useId, useRef, useState } from "react";
import { usePasskeySupport } from "@/components/passkeys/settings";
import { Banner, Button, CodeInput, Dialog, Input, Spinner } from "@/components/ui";
import type * as G from "@/gen";
import { type ApiFailure, api, setConfirmer } from "@/lib/api";
import { cn } from "@/lib/cn";
import { cancelled, getPasskey, passkeyError, rpId, signalUnknown } from "@/lib/passkeys";
import { formatRecoveryCode, isRecoveryCode } from "@/lib/two-factor";
import { Hint, useNow } from "./challenge";

type Mode = "passkey" | "app" | "recovery";

/** The dialog every request that needs it waits for, mounted once (main.tsx). */
export function VerifyDialog() {
  const [open, setOpen] = useState(false);
  // Requests that wait for it at once share one dialog, and its answer.
  const waiting = useRef<((confirmed: boolean) => void)[]>([]);
  useEffect(
    () =>
      setConfirmer(
        () =>
          new Promise<boolean>((resolve) => {
            waiting.current.push(resolve);
            setOpen(true);
          }),
      ),
    [],
  );
  const finish = useCallback((confirmed: boolean) => {
    const answer = waiting.current;
    waiting.current = [];
    setOpen(false);
    for (const resolve of answer) resolve(confirmed);
  }, []);
  return (
    <Dialog open={open} onClose={() => finish(false)} title="Confirm it's you">
      {open && <VerifyStep onDone={() => finish(true)} onCancel={() => finish(false)} />}
    </Dialog>
  );
}

/** What confirming takes, by what the person uses for it. */
function intro(mode: Mode): string {
  const then = "This browser then won't ask again for 15 minutes.";
  switch (mode) {
    case "passkey":
      return `This takes your passkey. ${then}`;
    case "app":
      return `This takes the code your authenticator app shows. ${then}`;
    case "recovery":
      return `This takes one of your recovery codes. ${then}`;
  }
}

export function VerifyStep({ onDone, onCancel }: { onDone: () => void; onCancel: () => void }) {
  const status = useQuery({
    queryKey: ["verification"],
    queryFn: api.verification,
    staleTime: 0,
    gcTime: 0,
    retry: false,
  });
  const support = usePasskeySupport();
  const methods = status.data?.methods ?? [];
  const has = (m: G.SecondFactor) => methods.includes(m);
  const passkeys = has("passkey") && !!support?.supported;
  const [chosen, setMode] = useState<Mode>();
  // The passkey first, where this browser can use it.
  const mode: Mode | undefined =
    chosen ??
    (!status.data || !support
      ? undefined
      : passkeys
        ? "passkey"
        : has("totp")
          ? "app"
          : has("recovery_code")
            ? "recovery"
            : undefined);
  // Fetched ahead, so that the click opens the browser's passkey sheet at once.
  const passkeyOptions = useQuery({
    queryKey: ["verify-passkey"],
    queryFn: api.verifyPasskeyOptions,
    enabled: passkeys,
    retry: false,
    staleTime: 8 * 60_000,
    gcTime: 0,
    refetchOnWindowFocus: false,
  });
  const [passkeyFailure, setPasskeyFailure] = useState<string | null>(null);
  // The passkey that answered last, for its provider to forget if fctl doesn't know it.
  const answered = useRef<string | null>(null);
  const [code, setCode] = useState("");
  const [recovery, setRecovery] = useState("");
  const [errors, setErrors] = useState(0);
  const [lockedUntil, setLockedUntil] = useState<number>();
  const now = useNow(lockedUntil ?? status.data?.locked_until);
  const until = lockedUntil ?? status.data?.locked_until;
  const locked = !!until && until > now;
  const hintId = useId();
  const verify = useMutation({
    mutationFn: (b: G.VerifyRequest) => api.verify(b),
    onSuccess: () => setTimeout(onDone, 420),
    onError: (e: ApiFailure) => {
      if (e.code === "rate_limited" && e.retryAfter) {
        setLockedUntil(Date.now() + e.retryAfter * 1000);
        setCode("");
        setRecovery("");
      } else if (e.field === "passkey" || e.status === 404) {
        if (e.status === 404 && answered.current) signalUnknown(rpId(), answered.current);
        setPasskeyFailure(e.message);
      } else if (e.field === "code") {
        setErrors((n) => n + 1);
        setCode("");
        setRecovery("");
      }
    },
  });
  const error = verify.error as ApiFailure | null;
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

  if (status.isError)
    return (
      <div className="flex flex-col gap-5">
        <Banner tone="error">{status.error.message}</Banner>
        <div className="flex justify-end">
          <Button onClick={onCancel}>Close</Button>
        </div>
      </div>
    );
  if (!status.data || !support)
    return (
      <div className="flex justify-center py-8">
        <Spinner className="size-5" />
      </div>
    );
  if (!mode)
    return (
      <div className="flex flex-col gap-5">
        <Banner tone="warn">
          {has("passkey")
            ? "Your passkey is how you confirm it's you, and this browser can't use passkeys. Do this in another browser, or set up an authenticator app in Settings."
            : "You have neither a passkey nor an authenticator app to confirm it's you with."}
        </Banner>
        <div className="flex justify-end">
          <Button onClick={onCancel}>Close</Button>
        </div>
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
      <p className="-mt-3 text-[13.5px] leading-relaxed text-ink-3">{intro(mode)}</p>
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
            autoFocus
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
            lockedUntil={until}
            now={now}
            onRecovery={has("recovery_code") ? () => switchTo("recovery") : undefined}
          />
          <button type="submit" className="sr-only">
            Confirm
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
            autoFocus
          />
          <Hint
            id={hintId}
            error={error}
            errors={errors}
            locked={locked}
            lockedUntil={until}
            now={now}
            recovery
          />
          <Button
            type="submit"
            variant="primary"
            size="lg"
            loading={verify.isPending}
            disabled={!isRecoveryCode(recovery) || verify.isSuccess}
          >
            Confirm
          </Button>
        </form>
      )}

      {error && !error.field && error.code !== "rate_limited" && error.status !== 404 && (
        <Banner tone="error">{error.message}</Banner>
      )}

      <div className="flex items-end justify-between gap-4">
        <div className="flex flex-col items-start gap-1.5 text-[12.5px] text-ink-3">
          {mode !== "passkey" && passkeys && (
            <button
              type="button"
              className="group inline-flex items-center gap-1.5 hover:text-ink"
              onClick={() => switchTo("passkey")}
            >
              <Fingerprint className="size-3.5" />
              <span className="text-ink-2 underline underline-offset-2 group-hover:text-ink">
                Use your passkey instead
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
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
      </div>
    </div>
  );
}

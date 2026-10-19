import { useMutation } from "@tanstack/react-query";
import { Clock } from "lucide-react";
import { useEffect, useId, useState } from "react";
import { Banner, Button, CodeInput, Input } from "@/components/ui";
import type { ApiFailure } from "@/lib/api";
import { CODE_LENGTH, clock, formatRecoveryCode, isRecoveryCode } from "@/lib/two-factor";

/**
 * Confirms a change to two-factor sign-in with a current code: one from the
 * authenticator app, or a recovery code.
 */
export function ConfirmWithCode<T>({
  submit,
  onDone,
  action,
  danger,
  onCancel,
}: {
  submit: (code: string) => Promise<T>;
  onDone: (r: T) => void;
  /** The button's label. */
  action: string;
  danger?: boolean;
  onCancel: () => void;
}) {
  const [mode, setMode] = useState<"app" | "recovery">("app");
  const [code, setCode] = useState("");
  const [errors, setErrors] = useState(0);
  const [lockedUntil, setLockedUntil] = useState<number>();
  const [now, setNow] = useState(Date.now);
  const id = useId();
  const run = useMutation({
    mutationFn: submit,
    onSuccess: onDone,
    onError: (e: ApiFailure) => {
      if (e.code === "rate_limited" && e.retryAfter) {
        setLockedUntil(Date.now() + e.retryAfter * 1000);
        setCode("");
      }
      if (e.field === "code") {
        setErrors((n) => n + 1);
        setCode("");
      }
    },
  });
  // The wait holds back codes from the app; recovery codes still work.
  const locked = !!lockedUntil && lockedUntil > now && mode === "app";
  useEffect(() => {
    if (!locked) return;
    const t = setInterval(() => setNow(Date.now()), 250);
    return () => clearInterval(t);
  }, [locked]);
  const error = run.error as ApiFailure | null;
  const ready = mode === "app" ? code.length === CODE_LENGTH : isRecoveryCode(code);
  const send = (c: string) => {
    if (!run.isPending && !locked) run.mutate(c);
  };
  return (
    <form
      className="flex flex-col gap-4"
      onSubmit={(e) => {
        e.preventDefault();
        if (ready) send(code);
      }}
    >
      {mode === "app" ? (
        <div className="flex flex-col gap-2">
          <label htmlFor={id} className="text-[12.5px] font-medium text-ink-2">
            Code from your authenticator app
          </label>
          <CodeInput
            id={id}
            value={code}
            onChange={setCode}
            onComplete={send}
            state={run.isPending ? "busy" : error?.field === "code" ? "error" : "idle"}
            errors={errors}
            disabled={locked}
            autoFocus
          />
        </div>
      ) : (
        <div className="flex flex-col gap-2">
          <label htmlFor={id} className="text-[12.5px] font-medium text-ink-2">
            Recovery code
          </label>
          <Input
            id={id}
            value={code}
            onChange={(e) => setCode(formatRecoveryCode(e.target.value))}
            placeholder="XXXXX-XXXXX"
            autoComplete="off"
            spellCheck={false}
            autoFocus
            className="font-mono tracking-[0.1em] uppercase"
          />
        </div>
      )}
      {locked && lockedUntil ? (
        <p className="flex items-center gap-1.5 text-[12.5px] text-amber" role="status">
          <Clock className="size-3.5 shrink-0" />
          <span>
            Too many wrong codes. Try again in{" "}
            <span className="font-mono tabular-nums">{clock(lockedUntil - now)}</span>, or use a recovery
            code.
          </span>
        </p>
      ) : error?.field === "code" ? (
        <p className="text-[12.5px] text-danger" role="alert">
          {error.message}
        </p>
      ) : error && error.code !== "rate_limited" ? (
        <Banner tone="error">{error.message}</Banner>
      ) : null}
      <button
        type="button"
        className="self-start text-[12.5px] text-ink-3 underline decoration-line-strong underline-offset-2 hover:text-ink"
        onClick={() => {
          setMode(mode === "app" ? "recovery" : "app");
          setCode("");
          run.reset();
        }}
      >
        {mode === "app" ? "Use a recovery code instead" : "Use your authenticator app instead"}
      </button>
      <div className="flex justify-end gap-2">
        <Button type="button" variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          type="submit"
          variant={danger ? "danger" : "primary"}
          loading={run.isPending}
          disabled={!ready || locked}
        >
          {action}
        </Button>
      </div>
    </form>
  );
}

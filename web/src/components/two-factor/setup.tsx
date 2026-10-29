// Setting up an authenticator app: scan the QR code (or open the link on a
// phone, or type the key), enter the code the app shows, and keep the
// recovery codes. Settings and the page that requires it both use it.

import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowRight, ArrowUpRight, Smartphone } from "lucide-react";
import { useId, useRef, useState } from "react";
import { Banner, Button, CodeInput, CopyButton, Field, Input, Skeleton, Spinner } from "@/components/ui";
import type * as G from "@/gen";
import { type ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { CODE_LENGTH, groupKey, touchDevice } from "@/lib/two-factor";
import { QrCode } from "./qr-code";
import { RecoveryCodes } from "./recovery-codes";

/** The fields whose errors the form shows beside them; others (a sign-in that's too old) show as a banner. */
const INLINE = ["code", "password", "setup"];

const APPS =
  "Any authenticator app works: 1Password, Google Authenticator, Microsoft Authenticator, Authy, or the passwords app on your phone.";

/** What the flow ends with: the account as it is now, and whether recovery codes were shown. */
export type SetupResult = { me: G.Me; codes: boolean };

export function AuthenticatorSetup({
  me,
  started,
  onRestart,
  onConfirmed,
  onDone,
  onCancel,
}: {
  me: G.Me;
  /** A setup already started: replacing an app, which took a code from it. */
  started?: G.AuthenticatorSetup;
  /** Starts over when the setup took too long (by default, a new one here). */
  onRestart?: () => void;
  /** The app's code checked out: two-factor is on (the recovery codes may still show). */
  onConfirmed?: (me: G.Me) => void;
  onDone: (r: SetupResult) => void;
  onCancel?: () => void;
}) {
  const [attempt, setAttempt] = useState(0);
  const fetched = useQuery({
    queryKey: ["authenticator-setup", attempt],
    queryFn: () => api.startAuthenticator(),
    enabled: !started,
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
    retry: false,
    refetchOnWindowFocus: false,
  });
  const setup = started ? { data: started, error: null } : fetched;
  const replacing = !!started;
  const restart = onRestart ?? (() => setAttempt((n) => n + 1));
  const [codes, setCodes] = useState<{ codes: string[]; me: G.Me } | null>(null);
  if (codes)
    return (
      <SaveCodes codes={codes.codes} me={codes.me} onDone={() => onDone({ me: codes.me, codes: true })} />
    );
  if (setup.error)
    return (
      <div className="flex flex-col gap-4">
        <Banner tone="error">{setup.error.message}</Banner>
        <div className="flex gap-2">
          <Button onClick={restart}>Try again</Button>
          {onCancel && (
            <Button variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          )}
        </div>
      </div>
    );
  if (!setup.data) return <ScanSkeleton />;
  return (
    <Scan
      key={setup.data.setup}
      setup={setup.data}
      me={me}
      replacing={replacing}
      onRestart={restart}
      onCancel={onCancel}
      onConfirmed={(r) => {
        onConfirmed?.(r.me);
        if (r.recovery_codes?.length) setCodes({ codes: r.recovery_codes, me: r.me });
        else onDone({ me: r.me, codes: false });
      }}
    />
  );
}

function ScanSkeleton() {
  return (
    <div className="grid gap-6 sm:grid-cols-[200px_1fr]">
      <Skeleton className="aspect-square w-full max-w-[200px]" />
      <div className="flex flex-col gap-3">
        <Skeleton className="h-5 w-3/4" />
        <Skeleton className="h-4 w-full" />
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="mt-4 h-12 w-72" />
      </div>
    </div>
  );
}

function Step({ n, title, children }: { n: number; title: string; children: React.ReactNode }) {
  return (
    <div className="flex gap-3">
      <span className="mt-px flex size-5 shrink-0 items-center justify-center rounded-full border border-line-strong font-mono text-[11px] text-ink-3">
        {n}
      </span>
      <div className="min-w-0 flex-1">
        <h3 className="text-[13.5px] font-medium text-ink">{title}</h3>
        <div className="mt-1.5">{children}</div>
      </div>
    </div>
  );
}

function Scan({
  setup,
  me,
  replacing,
  onConfirmed,
  onRestart,
  onCancel,
}: {
  setup: G.AuthenticatorSetup;
  me: G.Me;
  replacing?: boolean;
  onConfirmed: (r: G.AuthenticatorAdded) => void;
  onRestart: () => void;
  onCancel?: () => void;
}) {
  const [code, setCode] = useState("");
  const [password, setPassword] = useState("");
  const [errors, setErrors] = useState(0);
  const [showKey, setShowKey] = useState(false);
  const [touch] = useState(touchDevice);
  const codeRef = useRef<HTMLInputElement>(null);
  const hintId = useId();
  const confirm = useMutation({
    mutationFn: (c: string) =>
      api.confirmAuthenticator({
        setup: setup.setup,
        code: c,
        password: setup.password_required ? password : undefined,
      }),
    onSuccess: (r) => setTimeout(() => onConfirmed(r), 380),
    onError: (e: ApiFailure) => {
      if (e.field === "code") {
        setErrors((n) => n + 1);
        setCode("");
        codeRef.current?.focus();
      }
    },
  });
  const error = confirm.error as ApiFailure | null;
  const expired = error?.field === "setup";
  const submit = (c = code) => {
    if (c.length !== CODE_LENGTH || confirm.isPending || confirm.isSuccess) return;
    if (setup.password_required && !password) return;
    confirm.mutate(c);
  };
  const state = confirm.isSuccess
    ? "ok"
    : confirm.isPending
      ? "busy"
      : error?.field === "code"
        ? "error"
        : "idle";
  if (expired)
    return (
      <div className="flex flex-col items-start gap-4">
        <Banner tone="warn">This setup took too long, so its QR code no longer works.</Banner>
        <Button variant="primary" onClick={onRestart}>
          Start again
        </Button>
      </div>
    );
  return (
    <form
      className="grid gap-x-8 gap-y-6 sm:grid-cols-[200px_1fr]"
      onSubmit={(e) => {
        e.preventDefault();
        submit();
      }}
    >
      <div className="flex flex-col items-center gap-3 sm:items-start">
        <QrCode
          rows={setup.qr}
          label={`A QR code that adds ${me.user.email} to an authenticator app`}
          className="w-full max-w-[200px]"
        />
        {touch && (
          <a
            href={setup.uri}
            className="inline-flex h-8.5 w-full max-w-[200px] items-center justify-center gap-2 rounded-md border border-line-strong bg-raised text-[13px] font-medium text-ink transition-colors hover:border-ink-4"
          >
            <Smartphone className="size-3.5" /> Open in your app{" "}
            <ArrowUpRight className="size-3.5 text-ink-3" />
          </a>
        )}
      </div>
      <div className="flex min-w-0 flex-col gap-6">
        <Step
          n={1}
          title={
            touch ? "Add fctl to your authenticator app" : "Scan the QR code with your authenticator app"
          }
        >
          <p className="text-[12.5px] leading-relaxed text-ink-3">
            {touch ? "Open the link on this phone, or scan the code from another one. " : ""}
            {APPS}
          </p>
          {showKey ? (
            <div className="mt-2.5 animate-rise">
              <div className="text-[12px] text-ink-3">Type in this key, as a time-based code:</div>
              <div className="mt-1.5 flex items-center gap-1 rounded-md border border-line-strong bg-sunken py-1 pr-1 pl-3">
                <code className="min-w-0 flex-1 font-mono text-[13px] tracking-[0.06em] break-all text-ink">
                  {groupKey(setup.secret)}
                </code>
                <CopyButton text={setup.secret} />
              </div>
            </div>
          ) : (
            <button
              type="button"
              className="mt-1.5 text-[12.5px] text-ink-2 underline decoration-line-strong underline-offset-2 hover:text-ink hover:decoration-ink-4"
              onClick={() => setShowKey(true)}
            >
              Can't scan it? Type in a key instead
            </button>
          )}
        </Step>
        <Step n={2} title="Enter the code the app shows">
          <div className="flex flex-col gap-3">
            <CodeInput
              ref={codeRef}
              value={code}
              onChange={setCode}
              onComplete={(c) => {
                if (!setup.password_required || password) submit(c);
              }}
              state={state}
              errors={errors}
              disabled={confirm.isSuccess}
              label="The six-digit code from your authenticator app"
              describedBy={hintId}
              autoFocus={!touch && !setup.password_required}
            />
            <p
              id={hintId}
              className={cn(
                "text-[12.5px] leading-relaxed",
                error?.field === "code" ? "text-danger" : "text-ink-3",
              )}
            >
              {error?.field === "code"
                ? errors > 1
                  ? "That code doesn't match either. Codes depend on the time: check that your phone sets its clock automatically."
                  : "That code doesn't match. Enter the one the app shows for this account now."
                : "It changes every 30 seconds; any current one works."}
            </p>
          </div>
        </Step>
        {setup.password_required && (
          <Field
            label="Your password"
            hint="You signed in a while ago, so confirm it's you."
            error={error?.field === "password" ? error.message : undefined}
          >
            {(id) => (
              <Input
                id={id}
                type="password"
                autoComplete="current-password"
                value={password}
                onChange={(e) => setPassword(e.target.value)}
              />
            )}
          </Field>
        )}
        {error && !INLINE.includes(error.field ?? "") && <Banner tone="error">{error.message}</Banner>}
        <div className="flex items-center gap-2">
          <Button
            type="submit"
            variant="primary"
            loading={confirm.isPending}
            disabled={
              code.length !== CODE_LENGTH || (setup.password_required && !password) || confirm.isSuccess
            }
            trailing={confirm.isPending ? undefined : <ArrowRight className="size-4" />}
          >
            {replacing ? "Use this app" : "Turn on two-factor"}
          </Button>
          {onCancel && (
            <Button type="button" variant="ghost" onClick={onCancel}>
              Cancel
            </Button>
          )}
          {confirm.isSuccess && <Spinner className="size-3.5 text-ink-3" />}
        </div>
      </div>
    </form>
  );
}

/** A check that draws itself in a ring: two-factor is on. */
export function OnMark({ className }: { className?: string }) {
  return (
    <span
      className={cn(
        "flex size-10 animate-pop items-center justify-center rounded-full bg-highlight text-highlight-ink shadow-[0_0_0_6px_var(--c-accent-soft)]",
        className,
      )}
    >
      <svg viewBox="0 0 24 24" className="size-5" fill="none" aria-hidden="true">
        <path
          d="M5 12.5l4.2 4.2L19 7"
          stroke="currentColor"
          strokeWidth="2.4"
          strokeLinecap="round"
          strokeLinejoin="round"
          pathLength={1}
          className="animate-draw [stroke-dasharray:1]"
        />
      </svg>
    </span>
  );
}

function SaveCodes({ codes, me, onDone }: { codes: string[]; me: G.Me; onDone: () => void }) {
  const [saved, setSaved] = useState(false);
  return (
    <div className="flex flex-col gap-5">
      <div className="flex items-center gap-3.5">
        <OnMark />
        <div>
          <h3 className="text-[15px] font-medium text-ink">Two-factor sign-in is on</h3>
          <p className="text-[13px] text-ink-3">Signing in with your password now also takes a code.</p>
        </div>
      </div>
      <div>
        <h4 className="text-[13.5px] font-medium text-ink">Save your recovery codes</h4>
        <p className="mt-1 max-w-xl text-[12.5px] leading-relaxed text-ink-3">
          If you lose your phone, each of these signs you in once in place of a code. Keep them somewhere
          safe, such as your password manager: they are shown only now.
        </p>
        <div className="mt-3">
          <RecoveryCodes codes={codes} email={me.user.email} />
        </div>
      </div>
      <label className="flex cursor-pointer items-center gap-2.5 text-[13.5px] text-ink-2 select-none">
        <input
          type="checkbox"
          checked={saved}
          onChange={(e) => setSaved(e.target.checked)}
          className="size-4 accent-[var(--c-highlight)]"
        />
        I saved my recovery codes
      </label>
      <div>
        <Button
          variant="primary"
          disabled={!saved}
          onClick={onDone}
          trailing={<ArrowRight className="size-4" />}
        >
          Done
        </Button>
      </div>
    </div>
  );
}

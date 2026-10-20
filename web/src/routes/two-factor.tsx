// The signed-in layout's door: an account the instance requires two-factor
// sign-in of sees the page that sets it up, until it has, instead of the app.
// After a sign-in without a passkey, the app offers to make one.

import { ArrowLeft, ArrowRight, Fingerprint, Smartphone, X } from "lucide-react";
import { type ReactNode, useEffect, useState } from "react";
import { AddPasskeyProof, useAddPasskey } from "@/components/passkeys/add";
import { usePasskeySupport } from "@/components/passkeys/settings";
import { AppShell } from "@/components/shell/app-shell";
import { AuthenticatorSetup } from "@/components/two-factor/setup";
import { Banner, Button, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api, TWO_FACTOR_REQUIRED } from "@/lib/api";
import { cn } from "@/lib/cn";
import { forgetDraft } from "@/lib/draft";
import { OFFER_PASSKEY } from "@/lib/passkeys";
import { forgetPush } from "@/lib/push";
import { keys, queryClient, useInstance, useMe } from "@/lib/queries";
import { RECOVERY_LEFT } from "@/lib/two-factor";
import { AuthFrame } from "@/routes/auth";

export function SignedInLayout() {
  const { data: me } = useMe();
  // A request refused for want of two-factor: ask who this is again, so that
  // the page that sets it up shows at once.
  useEffect(() => {
    const again = () => void queryClient.invalidateQueries({ queryKey: keys.me });
    addEventListener(TWO_FACTOR_REQUIRED, again);
    return () => removeEventListener(TWO_FACTOR_REQUIRED, again);
  }, []);
  useRecoveryNotice();
  if (me?.two_factor.setup_required) return <TwoFactorRequired me={me} />;
  return (
    <>
      <AppShell />
      {me && <PasskeyOffer me={me} />}
    </>
  );
}

/** After a sign-in with a recovery code, say once how many are left. */
function useRecoveryNotice() {
  useEffect(() => {
    const left = sessionStorage.getItem(RECOVERY_LEFT);
    if (left === null) return;
    sessionStorage.removeItem(RECOVERY_LEFT);
    const n = Number(left);
    toast(
      n === 0
        ? "That was your last recovery code. Get new ones in Settings."
        : n === 1
          ? "Signed in with a recovery code. 1 is left: get new ones in Settings."
          : `Signed in with a recovery code. ${n} are left.`,
      "info",
    );
  }, []);
}

function TwoFactorRequired({ me }: { me: G.Me }) {
  const { data: instance } = useInstance();
  const support = usePasskeySupport();
  const [way, setWay] = useState<"choose" | "app">("choose");
  const add = useAddPasskey({
    enabled: !!support?.supported,
    onAdded: () => toast("Your passkey is in: it's your second factor. Welcome in."),
  });
  const signOut = async () => {
    await forgetPush();
    await forgetDraft();
    await api.logout().catch(() => {});
    queryClient.clear();
    window.location.assign("/login");
  };
  // Without passkeys there is nothing to choose.
  const app = way === "app" || support?.supported === false;
  return (
    <AuthFrame
      wide
      title={
        <>
          Set up two-factor <span className="whitespace-nowrap">sign-in.</span>
        </>
      }
      subtitle={`${instance?.name ?? "This instance"} asks everyone to confirm their sign-ins with a second factor. ${
        app ? "It takes about a minute." : "A passkey takes a moment; an authenticator app, about a minute."
      }`}
      footer={
        <>
          Signed in as {me.user.email} ·{" "}
          <button
            type="button"
            className="text-ink-2 underline underline-offset-2 hover:text-ink"
            onClick={signOut}
          >
            Sign out
          </button>
        </>
      }
    >
      {app ? (
        <div className="flex flex-col gap-5">
          {support?.supported && (
            <button
              type="button"
              className="inline-flex items-center gap-1.5 self-start text-[12.5px] text-ink-3 hover:text-ink"
              onClick={() => setWay("choose")}
            >
              <ArrowLeft className="size-3.5" /> Other ways
            </button>
          )}
          <AuthenticatorSetup
            me={me}
            onDone={(r) => {
              queryClient.setQueryData(keys.me, r.me);
              toast("Two-factor sign-in is on. Welcome in.");
            }}
          />
        </div>
      ) : (
        <div className="flex flex-col gap-4">
          <div className="grid gap-3 sm:grid-cols-2">
            <Way
              icon={<Fingerprint className="size-5" />}
              title="Add a passkey"
              text="Your fingerprint, face or screen lock confirms it's you. Nothing to type, and nothing a fake site can catch."
              note={support?.platform ? "This device can keep one" : "On your phone or a security key"}
              recommended
              busy={add.busy}
              onClick={add.start}
            />
            <Way
              icon={<Smartphone className="size-5" />}
              title="Use an authenticator app"
              text="A six-digit code from an app on your phone at each sign-in, with recovery codes for when it's gone."
              note="1Password, Google Authenticator and others"
              onClick={() => setWay("app")}
            />
          </div>
          {add.error && !add.proof && <Banner tone="error">{add.error}</Banner>}
          <AddPasskeyProof add={add} />
        </div>
      )}
    </AuthFrame>
  );
}

function Way({
  icon,
  title,
  text,
  note,
  recommended,
  busy,
  onClick,
}: {
  icon: ReactNode;
  title: string;
  text: string;
  note: string;
  recommended?: boolean;
  busy?: boolean;
  onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      disabled={busy}
      className={cn(
        "group flex flex-col items-start gap-3 rounded-lg border p-4 text-left transition-[border-color,background-color,transform] duration-150 active:scale-[0.99]",
        recommended
          ? "border-accent/40 bg-accent-soft/40 hover:border-accent/70"
          : "border-line-strong bg-sunken/40 hover:border-ink-4",
      )}
    >
      <span className="flex w-full items-center justify-between">
        <span
          className={cn(
            "flex size-10 items-center justify-center rounded-full",
            recommended ? "bg-highlight text-highlight-ink" : "bg-raised text-ink-2",
            busy && "animate-pulse",
          )}
        >
          {icon}
        </span>
        {recommended && (
          <span className="rounded-full bg-accent-soft px-2 py-0.5 text-[11px] text-ink-2">Quickest</span>
        )}
      </span>
      <span>
        <span className="flex items-center gap-1.5 text-[14.5px] font-medium text-ink">
          {title}
          <ArrowRight className="size-3.5 text-ink-4 transition-transform group-hover:translate-x-0.5" />
        </span>
        <span className="mt-1 block text-[12.5px] leading-relaxed text-ink-3">{text}</span>
        <span className="mt-2 block text-[11.5px] text-ink-4">{note}</span>
      </span>
    </button>
  );
}

/** localStorage: when the offer of a passkey was turned down in this browser. It comes back after 30 days. */
const DISMISSED = "fctl:passkey-offer-dismissed";
const ASK_AGAIN_MS = 30 * 24 * 60 * 60 * 1000;

/** After a sign-in without a passkey, on a device that can keep one: a card that offers to make one. */
function PasskeyOffer({ me }: { me: G.Me }) {
  const support = usePasskeySupport();
  const [open, setOpen] = useState(() => {
    const offered = sessionStorage.getItem(OFFER_PASSKEY) === "1";
    sessionStorage.removeItem(OFFER_PASSKEY);
    const dismissed = Number(localStorage.getItem(DISMISSED) ?? 0);
    return offered && Date.now() - dismissed > ASK_AGAIN_MS;
  });
  const show = open && support?.platform && me.passkeys.length === 0;
  const add = useAddPasskey({
    enabled: !!show,
    onAdded: (m) => {
      setOpen(false);
      toast(`${m.passkeys.at(-1)?.name ?? "Your passkey"} is in. Next time, skip the password.`);
    },
  });
  if (!show) return null;
  const dismiss = () => {
    localStorage.setItem(DISMISSED, String(Date.now()));
    setOpen(false);
  };
  return (
    <aside
      aria-label="Add a passkey"
      className="fixed right-[max(1rem,env(safe-area-inset-right))] bottom-[max(1rem,env(safe-area-inset-bottom))] z-40 w-[min(360px,calc(100vw-2rem))] animate-rise rounded-xl border border-line-strong bg-surface/95 p-4 shadow-[0_18px_60px_rgb(0_0_0/0.35)] backdrop-blur-xl"
    >
      <div className="flex gap-3">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-highlight text-highlight-ink">
          <Fingerprint className="size-4.5" />
        </span>
        <div className="min-w-0 flex-1">
          <div className="flex items-start justify-between gap-2">
            <h2 className="text-[14px] font-medium text-ink">Sign in faster next time</h2>
            <button
              type="button"
              aria-label="Not now"
              className="-mt-1 -mr-1 rounded p-1 text-ink-4 hover:bg-hover hover:text-ink"
              onClick={dismiss}
            >
              <X className="size-3.5" />
            </button>
          </div>
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
            Add a passkey, and your fingerprint, face or screen lock signs you in on this device instead of
            your password.
          </p>
          {add.error && !add.proof && <p className="mt-2 text-[12.5px] text-danger">{add.error}</p>}
          <div className="mt-3 flex items-center gap-2">
            <Button size="sm" variant="primary" loading={add.busy} onClick={add.start}>
              Add a passkey
            </Button>
            <Button size="sm" variant="ghost" onClick={dismiss}>
              Not now
            </Button>
          </div>
        </div>
      </div>
      <AddPasskeyProof add={add} />
    </aside>
  );
}

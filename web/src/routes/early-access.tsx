// /early-access, where the instance takes requests (as app.fctl.sh does):
// people without an account ask for early access with their email address.
// A thread of the foundry behind the page is theirs: it shows while they
// type an address, and lights up, with a packet running into the planet, once
// they are on the list; the threads make way for the pointer. Then the page
// says what to have ready for when the invitation comes. This browser
// remembers the request (`lib/early-access`), so the page says so when they
// come back, and can withdraw it.

import { prefersReducedMotion } from "@hyperstructure/design/gl/core";
import type { FoundryHandle, FoundryLane } from "@hyperstructure/design/gl/foundry";
import { Link } from "@tanstack/react-router";
import {
  ArrowRight,
  ArrowUpRight,
  Container,
  GitPullRequestArrow,
  type LucideIcon,
  MessageSquareText,
} from "lucide-react";
import { type FormEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { Banner, Button, Dialog, Field, Input } from "@/components/ui";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { loadSent, type Sent, saveSent } from "@/lib/early-access";
import { day } from "@/lib/format";
import { siteUrl } from "@/lib/install";
import { AuthFrame } from "@/routes/auth";

/** The thread that is theirs. */
const LANE = 21;
/** What could be an address, for their thread to show while they type it. */
const ADDRESS = /^[^\s@]+@[^\s@.]+\.[^\s@]+$/;

const POINTS: { icon: LucideIcon; lead: string; rest: string }[] = [
  {
    icon: MessageSquareText,
    lead: "Tell agents what to build",
    rest: "from the app, the CLI or your phone.",
  },
  {
    icon: Container,
    lead: "They work in sandboxes on your machines,",
    rest: "on your Claude plan, never on API billing.",
  },
  {
    icon: GitPullRequestArrow,
    lead: "You review their pull requests.",
    rest: "Each session works on a branch of its own.",
  },
];

const READY: { lead: string; rest: string }[] = [
  { lead: "A Claude Pro or Max plan.", rest: "Agents run on your subscription." },
  { lead: "A machine for their sandboxes:", rest: "a Mac with Apple silicon, or Linux with Docker." },
  { lead: "Your code on GitHub.", rest: "Agents open their pull requests there." },
];

export function EarlyAccessPage() {
  const [sent, setSent] = useState<Sent | undefined>(() => loadSent());
  /** Sent from this page just now, rather than remembered from before. */
  const [fresh, setFresh] = useState(false);
  /** Their thread runs fast for a few seconds after they're on the list. */
  const [burst, setBurst] = useState(false);
  /** They withdrew their request: the form says so. */
  const [withdrawn, setWithdrawn] = useState(false);
  const [email, setEmail] = useState("");
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<ApiFailure | null>(null);
  const scene = useRef<FoundryHandle | null>(null);
  const input = useRef<HTMLInputElement>(null);
  const site = siteUrl();

  // The threads make way for the pointer, over the whole page.
  useEffect(() => {
    const move = (e: PointerEvent) => {
      const page = document.documentElement;
      scene.current?.pointer(e.pageX / page.scrollWidth, e.pageY / page.scrollHeight);
    };
    const leave = () => scene.current?.pointerLeave();
    window.addEventListener("pointermove", move, { passive: true });
    document.documentElement.addEventListener("pointerleave", leave);
    return () => {
      window.removeEventListener("pointermove", move);
      document.documentElement.removeEventListener("pointerleave", leave);
    };
  }, []);

  const typed = !sent && ADDRESS.test(email.trim());
  useEffect(() => {
    // The foundry shows the first lanes it gets at once and eases later ones in, one frame
    // per change when motion is reduced: then their thread shows only once it is lit.
    if (!sent && prefersReducedMotion()) return;
    const lanes = new Map<number, FoundryLane>();
    if (sent) lanes.set(LANE, { lit: 1, color: 1, energy: burst ? 1 : 0.25 });
    else if (typed) lanes.set(LANE, { lit: 0.6, color: 2, energy: 0.35 });
    scene.current?.setLanes(lanes);
  }, [sent, typed, burst]);
  useEffect(() => {
    if (!burst) return;
    const t = setTimeout(() => setBurst(false), 4000);
    return () => clearTimeout(t);
  }, [burst]);

  const submit = async (e: FormEvent) => {
    e.preventDefault();
    if (!email.trim()) {
      setError(
        new ApiFailure(400, { code: "validation", field: "email", message: "Enter your email address" }),
      );
      input.current?.focus();
      return;
    }
    setBusy(true);
    setError(null);
    try {
      const { token } = await api.requestAccess({ email: email.trim() });
      const s = { email: email.trim().toLowerCase(), at: Date.now(), token };
      saveSent(s);
      setSent(s);
      setFresh(true);
      setWithdrawn(false);
      setBurst(true);
      scene.current?.pulse(LANE, 1000);
    } catch (err) {
      setError(err instanceof ApiFailure ? err : new ApiFailure(0, { message: String(err) }));
      input.current?.focus();
    } finally {
      setBusy(false);
    }
  };

  const another = () => {
    saveSent(undefined);
    setSent(undefined);
    setFresh(false);
    setEmail("");
    setError(null);
    setWithdrawn(false);
  };
  const withdraw = async (token: string) => {
    await api.withdrawAccess({ token });
    another();
    setWithdrawn(true);
  };

  const signIn = (
    <Link
      to="/login"
      search={{ next: undefined, error: undefined, deleted: undefined }}
      className="text-ink underline underline-offset-2"
    >
      Sign in
    </Link>
  );
  const keep = (s: FoundryHandle) => {
    scene.current = s;
  };

  if (sent) {
    return (
      <AuthFrame
        title={
          <span key="listed" className="inline-block animate-rise">
            You're <span className="title-hot">on the list</span>
            <span className="title-dot">.</span>
          </span>
        }
        subtitle={
          <span key="listed" className="inline-block animate-rise [animation-delay:60ms]">
            We'll send your invitation to <span className="text-ink">{sent.email}</span> when there's room. It
            sets up your account.
          </span>
        }
        onFoundry={keep}
        footer={
          <Frosted>
            <button type="button" onClick={another} className="text-ink underline underline-offset-2">
              Ask with another address
            </button>
            <span aria-hidden="true" className="text-ink-4">
              ·
            </span>
            {signIn}
          </Frosted>
        }
      >
        <p className="sr-only" aria-live="polite">
          {fresh ? `You're on the list. We'll send your invitation to ${sent.email}.` : ""}
        </p>
        <div className="flex animate-rise flex-col gap-6">
          <Ticket sent={sent} fresh={fresh} onWithdraw={withdraw} />
          <div>
            <h2 className="text-[13.5px] font-medium text-ink">While you wait, have these ready</h2>
            <ol className="mt-3 flex flex-col gap-3">
              {READY.map((r, i) => (
                <li
                  key={r.lead}
                  className="flex animate-rise gap-3 text-[13.5px] leading-snug"
                  style={{ animationDelay: `${160 + i * 80}ms` }}
                >
                  <span className="mt-px inline-flex size-5 shrink-0 items-center justify-center rounded-full border border-line-strong font-mono text-[10.5px] text-ink-2">
                    {i + 1}
                  </span>
                  <span>
                    <span className="text-ink">{r.lead}</span> <span className="text-ink-3">{r.rest}</span>
                  </span>
                </li>
              ))}
            </ol>
          </div>
          <a
            href={`${site}/docs/getting-started/`}
            target="_blank"
            rel="noreferrer"
            className="group -mt-1 inline-flex items-center gap-1.5 self-start text-[13px] text-ink-2 hover:text-ink"
          >
            Read how to get started
            <ArrowUpRight className="size-3.5 text-ink-4 transition-transform duration-200 group-hover:translate-x-px group-hover:-translate-y-px group-hover:text-ink" />
          </a>
        </div>
      </AuthFrame>
    );
  }

  return (
    <AuthFrame
      title={
        <>
          Request <span className="title-hot">early access</span>
          <span className="title-dot">.</span>
        </>
      }
      subtitle="Describe the work, and Claude agents do it in sandboxes on your own machines."
      onFoundry={keep}
      below={<Points />}
      footer={<Frosted>Have an account? {signIn}</Frosted>}
    >
      <form onSubmit={submit} className="flex flex-col gap-4" noValidate>
        {withdrawn && (
          // The control plane's answer is the same either way (api::early_access::withdraw).
          <Banner tone="ok">
            This browser forgot your address. Your request is withdrawn, unless another browser asked with it
            first.
          </Banner>
        )}
        <Field label="Email" error={error?.field === "email" ? error.message : undefined}>
          {(id) => (
            <Input
              ref={input}
              id={id}
              type="email"
              inputMode="email"
              autoComplete="email"
              enterKeyHint="send"
              autoCapitalize="off"
              spellCheck={false}
              required
              autoFocus
              placeholder="you@example.com"
              aria-invalid={error?.field === "email" || undefined}
              value={email}
              onChange={(e) => {
                setEmail(e.target.value);
                if (error) setError(null);
              }}
            />
          )}
        </Field>
        {error && !error.field && <Banner tone="error">{error.message}</Banner>}
        <Button
          type="submit"
          variant="primary"
          size="lg"
          loading={busy}
          trailing={<ArrowRight className="size-4" />}
        >
          Request early access
        </Button>
        <p className="text-center text-[12.5px] leading-relaxed text-ink-4">
          It's used only to send your invitation.{" "}
          <a
            href={`${site}/privacy`}
            target="_blank"
            rel="noreferrer"
            className="whitespace-nowrap underline underline-offset-2 hover:text-ink-2"
          >
            Privacy policy
          </a>
        </p>
      </form>
    </AuthFrame>
  );
}

/** What fctl does, under the form. */
function Points() {
  return (
    <ul className="mt-5 flex flex-col gap-3.5 rounded-xl bg-canvas/60 px-3 py-3 backdrop-blur-md">
      {POINTS.map(({ icon: Icon, lead, rest }, i) => (
        <li
          key={lead}
          className="flex animate-rise gap-3 text-[13.5px] leading-snug"
          style={{ animationDelay: `${140 + i * 70}ms` }}
        >
          <span className="inline-flex size-6 shrink-0 items-center justify-center rounded-md border border-line-strong bg-surface/80 text-ink-2 backdrop-blur-xl">
            <Icon className="size-3.5" />
          </span>
          <span className="pt-0.5">
            <span className="text-ink">{lead}</span> <span className="text-ink-2">{rest}</span>
          </span>
        </li>
      ))}
    </ul>
  );
}

/** Their place on the list: the address they asked with, and since when. */
function Ticket({
  sent,
  fresh,
  onWithdraw,
}: {
  sent: Sent;
  fresh: boolean;
  onWithdraw: (token: string) => Promise<void>;
}) {
  const [asking, setAsking] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState<string | null>(null);
  const token = sent.token;
  return (
    <div className="flex items-center gap-3.5 rounded-lg border border-line-strong bg-sunken/60 py-3.5 pr-2 pl-4">
      <span className={cn("relative inline-flex size-9 shrink-0", fresh && "animate-pop")}>
        {fresh && (
          <span className="absolute inset-0 animate-ping rounded-full bg-highlight/40 [animation-duration:900ms] [animation-fill-mode:forwards] [animation-iteration-count:1]" />
        )}
        <span className="relative inline-flex size-9 items-center justify-center rounded-full bg-highlight text-highlight-ink">
          <svg
            viewBox="0 0 24 24"
            className="size-[18px]"
            fill="none"
            stroke="currentColor"
            strokeWidth="2.6"
            strokeLinecap="round"
            strokeLinejoin="round"
            aria-hidden="true"
          >
            <path
              d="M5 12.5l4.5 4.5L19 7.5"
              pathLength={1}
              strokeDasharray="1"
              className={cn(fresh && "animate-draw [animation-delay:200ms]")}
            />
          </svg>
        </span>
      </span>
      <div className="min-w-0 flex-1">
        <div className="truncate font-mono text-[13px] text-ink">{sent.email}</div>
        <div className="text-[12.5px] text-ink-3">On the list since {day(sent.at)}</div>
      </div>
      {/* Only the browser that asked first can: its answer had the token. */}
      {token && (
        <Button size="sm" variant="quiet" onClick={() => setAsking(true)} className="shrink-0">
          Withdraw
        </Button>
      )}
      <Dialog
        open={asking}
        onClose={() => setAsking(false)}
        title="Withdraw your request?"
        description="You leave the list, and this browser forgets your address. Only the browser that first asked with it can withdraw it. You can ask again whenever you like."
      >
        <div className="flex flex-col gap-4">
          {error && <Banner tone="error">{error}</Banner>}
          <div className="flex justify-end gap-2">
            <Button variant="ghost" onClick={() => setAsking(false)}>
              Stay on the list
            </Button>
            <Button
              variant="danger"
              loading={busy}
              onClick={async () => {
                if (!token) return;
                setBusy(true);
                setError(null);
                try {
                  await onWithdraw(token);
                } catch (e) {
                  setError(e instanceof Error ? e.message : String(e));
                  setBusy(false);
                }
              }}
            >
              Withdraw my request
            </Button>
          </div>
        </div>
      </Dialog>
    </div>
  );
}

/**
 * Text over the foundry, on the page's own color frosted: nothing shows over the stars,
 * and the planet behind the text gives way.
 */
function Frosted({ children }: { children: ReactNode }) {
  return (
    <span className="inline-flex flex-wrap items-center justify-center gap-x-1.5 gap-y-1 rounded-full bg-canvas/60 px-3 py-1 backdrop-blur-md">
      {children}
    </span>
  );
}

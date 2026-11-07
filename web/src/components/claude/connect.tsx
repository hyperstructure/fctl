// Settings → Claude: connecting a Claude subscription. Two ways to choose from, each with what it
// comes to: a long-lived token from `claude setup-token`, recommended for a factory that runs
// around the clock since it lasts a year, which fctl keeps or your runners do; or the official
// sign-in, a full login, which brings Remote Control and claude.ai connectors but stops its sessions
// once Claude ends it, until you sign in again.

import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowUpRight, Check, Info, KeyRound, Minus, Radio } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { Banner, Button, buttonClass, CodeBox, Input, SecretInput, Spinner, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, queryClient } from "@/lib/queries";
import { Mono, Step } from "./guide";
import { OnRunnersSteps, SETUP_TOKEN } from "./on-runners";

/** How to connect: a long-lived token (recommended), or signing in with Claude. */
export type Way = "token" | "login";

/** Where a long-lived token is kept: fctl keeps it, or your runners do and fctl never has it. */
type Place = "fctl" | "runners";

/** The chooser's id, which a connection Claude ended scrolls to. */
export const CONNECT_ID = "connect-claude";

/** What a way comes to: what it gives (`good`), or what it lacks or costs. */
interface Fact {
  good: boolean;
  text: string;
}

export const TOKEN_FACTS: Fact[] = [
  { good: true, text: "Lasts a year: agents work around the clock without a new sign-in" },
  { good: true, text: "fctl keeps it, or your runners do and fctl never sees it" },
  { good: false, text: "No Remote Control or claude.ai connectors" },
];

export const LOGIN_FACTS: Fact[] = [
  { good: true, text: "Follow and steer sessions from claude.ai/code and the Claude apps" },
  { good: true, text: "Agents can use your claude.ai connectors, in projects that turn them on" },
  { good: false, text: "Claude ends it after a while: sessions stop until you sign in again" },
];

/**
 * The two ways to connect, as radio cards, the chosen one open on its steps. `way` is which one is
 * open (none, until one is chosen); the Claude section keeps it, so that a connection Claude ended
 * can open one.
 */
export function ConnectClaude({
  me,
  way,
  onWay,
}: {
  me: Pick<G.Me, "claude">;
  way: Way | null;
  onWay: (way: Way | null) => void;
}) {
  const name = useId();
  const first = me.claude.length === 0;
  return (
    <div id={CONNECT_ID} className="scroll-mt-6">
      <h3 className="text-[13.5px] font-medium text-ink">
        {first ? "Connect your subscription" : "Add a connection"}
      </h3>
      <fieldset className="mt-3 flex flex-col gap-3">
        <legend className="sr-only">How to connect Claude</legend>
        <WayCard
          name={name}
          value="token"
          way={way}
          onWay={onWay}
          icon={<KeyRound />}
          title="Long-lived token"
          recommended
          facts={TOKEN_FACTS}
        >
          <TokenFlow onRunners={me.claude.some((c) => c.on_runners)} onConnected={() => onWay(null)} />
        </WayCard>
        <WayCard
          name={name}
          value="login"
          way={way}
          onWay={onWay}
          icon={<Radio />}
          title="Sign in with Claude"
          facts={LOGIN_FACTS}
        >
          <SignInFlow onConnected={() => onWay(null)} />
        </WayCard>
      </fieldset>
    </div>
  );
}

/**
 * One way to connect: a radio, which the arrow keys move through, with what the way comes to, and
 * its steps under it once chosen.
 */
function WayCard({
  name,
  value,
  way,
  onWay,
  icon,
  title,
  recommended,
  facts,
  children,
}: {
  name: string;
  value: Way;
  way: Way | null;
  onWay: (way: Way) => void;
  icon: ReactNode;
  title: string;
  recommended?: boolean;
  facts: Fact[];
  children: ReactNode;
}) {
  const id = useId();
  const checked = way === value;
  return (
    <div
      className={cn(
        "rounded-xl border transition-[background-color,border-color,box-shadow] duration-150 motion-reduce:transition-none has-[input:focus-visible]:shadow-[0_0_0_3px_var(--c-accent-soft)]",
        checked
          ? "border-accent/55 bg-accent-soft/25 shadow-[inset_0_0_0_1px_var(--c-accent-soft)]"
          : "border-line bg-surface/60 hover:border-line-strong hover:bg-hover",
      )}
    >
      <label className={cn("flex items-start gap-3.5 p-4", !checked && "cursor-pointer")}>
        <input
          type="radio"
          name={name}
          value={value}
          checked={checked}
          onChange={() => onWay(value)}
          aria-labelledby={`${id}-title`}
          aria-describedby={`${id}-facts`}
          className="sr-only"
        />
        <span
          aria-hidden="true"
          className={cn(
            "flex size-9 shrink-0 items-center justify-center rounded-lg border transition-colors [&_svg]:size-4",
            checked ? "border-accent/40 bg-raised text-accent" : "border-line bg-raised/70 text-ink-3",
          )}
        >
          {icon}
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex flex-wrap items-center gap-x-2 gap-y-1">
            <span id={`${id}-title`} className="text-[14px] font-medium text-ink">
              {title}
            </span>
            {recommended && (
              <span className="rounded-full bg-highlight px-2 py-0.5 text-[11px] font-medium text-highlight-ink">
                Recommended
              </span>
            )}
          </span>
          <span id={`${id}-facts`} className="mt-2 flex flex-col gap-1.5">
            {facts.map((f) => (
              <span key={f.text} className="flex gap-2 text-[12.5px] leading-snug text-ink-3">
                {f.good ? (
                  <Check
                    aria-hidden="true"
                    className="mt-px size-3.5 shrink-0 text-accent"
                    strokeWidth={2.5}
                  />
                ) : (
                  <Minus
                    aria-hidden="true"
                    className="mt-px size-3.5 shrink-0 text-ink-4"
                    strokeWidth={2.5}
                  />
                )}
                {f.text}
              </span>
            ))}
          </span>
        </span>
        <span
          aria-hidden="true"
          className={cn(
            "mt-0.5 flex size-4 shrink-0 items-center justify-center rounded-full border transition-colors",
            checked ? "border-highlight bg-highlight text-highlight-ink" : "border-line-strong",
          )}
        >
          {checked && <Check className="size-2.5" strokeWidth={3} />}
        </span>
      </label>
      {/* Hidden rather than gone while another way is open, so that a pasted token or a sign-in
          under way is still there on coming back. */}
      <div hidden={!checked} className="animate-rise border-t border-line px-4 pt-5 pb-5 sm:pl-[66px]">
        {children}
      </div>
    </div>
  );
}

const PLACES: { value: Place; label: string; hint: string }[] = [
  {
    value: "fctl",
    label: "fctl keeps it",
    hint: "Encrypted, and given only to your sessions, on whichever runner runs them.",
  },
  {
    value: "runners",
    label: "My runners keep it",
    hint: "fctl never has it: your sessions run only on your runners that keep it, and wait while none of them is online.",
  },
];

/** A long-lived token: who keeps it, then the steps for that. */
function TokenFlow({ onRunners, onConnected }: { onRunners: boolean; onConnected: () => void }) {
  const [place, setPlace] = useState<Place>("fctl");
  const name = useId();
  // You keep one on your runners already: what is left is giving fctl one.
  const runners = place === "runners" && !onRunners;
  return (
    <div className="flex flex-col gap-6">
      {!onRunners && (
        <fieldset>
          <legend className="sr-only">Who keeps the token</legend>
          <div className="inline-flex rounded-lg border border-line bg-sunken p-0.5">
            {PLACES.map((p) => (
              <label
                key={p.value}
                className={cn(
                  "cursor-pointer rounded-md px-3 py-1.5 text-[12.5px] whitespace-nowrap transition-colors has-[:focus-visible]:shadow-[0_0_0_2px_var(--c-accent-soft)]",
                  place === p.value
                    ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25)]"
                    : "text-ink-3 hover:text-ink",
                )}
              >
                <input
                  type="radio"
                  name={name}
                  value={p.value}
                  checked={place === p.value}
                  onChange={() => setPlace(p.value)}
                  className="sr-only"
                />
                {p.label}
              </label>
            ))}
          </div>
          <p className="mt-2 text-[12.5px] leading-relaxed text-ink-3">
            {PLACES.find((p) => p.value === place)?.hint}
          </p>
        </fieldset>
      )}
      {/* Both stay, one hidden, so that a pasted token is still there after a look at the other. */}
      <div hidden={runners}>
        <PasteSteps onConnected={onConnected} />
      </div>
      {!onRunners && (
        <div hidden={!runners}>
          <OnRunnersSteps onConnected={onConnected} />
        </div>
      )}
      <p className="flex gap-2 text-[12px] leading-relaxed text-ink-4">
        <Info aria-hidden="true" className="mt-[3px] size-3.5 shrink-0" />
        <span>
          Agents can read the token they run on: one talked into handing it over gives away a year of access,
          not the hours a sign-in's token lasts. Use it where you trust everyone who can message your agents.
        </span>
      </p>
    </div>
  );
}

/** Making a long-lived token and giving it to fctl. */
function PasteSteps({ onConnected }: { onConnected: () => void }) {
  const [token, setToken] = useState("");
  const add = useMutation({
    mutationFn: () => api.claudeToken({ token: token.trim() }),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      setToken("");
      toast("Claude connected: new sessions run on the token");
      onConnected();
    },
  });
  return (
    <ol>
      <Step n={1} done={token.trim().length > 0} title="Create a token">
        <p>
          On any machine with Claude Code. It opens claude.ai to sign in, then prints a token that starts with{" "}
          <Mono>sk-ant-oat01-</Mono>.
        </p>
        <CodeBox value={SETUP_TOKEN} />
      </Step>
      <Step n={2} done={false} title="Paste it here">
        <form
          className="flex gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            add.mutate();
          }}
        >
          <SecretInput
            aria-label="Token to paste"
            value={token}
            onChange={(e) => setToken(e.target.value)}
            placeholder="sk-ant-oat01-…"
            className="min-w-0 flex-1 font-mono"
          />
          <Button
            type="submit"
            variant="primary"
            size="field"
            loading={add.isPending}
            disabled={!token.trim()}
          >
            Connect
          </Button>
        </form>
        {add.error && (
          <p className="text-[12.5px] text-danger" role="alert">
            {add.error.message}
          </p>
        )}
      </Step>
    </ol>
  );
}

/**
 * A sign-in with Claude: starting it, following its job, and relaying the code. Once it succeeds,
 * it starts over, so that the next one, for another account or once Claude ends this login, can.
 */
function useClaudeLogin(onConnected: () => void) {
  const [job, setJob] = useState<G.ClaudeLoginJob | null>(null);
  const [code, setCode] = useState("");
  const start = useMutation({
    mutationFn: api.claudeLogin,
    onSuccess: (j) => {
      setCode("");
      setJob(j);
    },
  });
  const submit = useMutation({
    mutationFn: () => api.claudeLoginCode(job?.id ?? "", code.trim()),
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
  const connected = useRef(onConnected);
  connected.current = onConnected;
  useEffect(() => {
    if (current?.state !== "succeeded") return;
    void queryClient.invalidateQueries({ queryKey: keys.me });
    toast("Signed in with Claude: Remote Control is on");
    setJob(null);
    setCode("");
    start.reset();
    submit.reset();
    connected.current();
  }, [current?.state, start.reset, submit.reset]);
  return { current, start, submit, code, setCode };
}

/** Whether a Claude sign-in is still under way: its runner may still pass on what it signs in to. */
function signingIn(job: G.ClaudeLoginJob): boolean {
  return job.state !== "succeeded" && job.state !== "failed";
}

/**
 * What a Claude sign-in that runs on someone else's runner means: none of the person's is online, and
 * that runner's host sees the login's long-lived tokens, and could keep them or connect a Claude
 * account of its own in their place (docs/security.md).
 */
function OthersRunner({ runner }: { runner: G.ClaudeLoginRunner }) {
  const whose = runner.owner ? `${runner.owner}'s runner` : "a runner";
  return (
    <>
      This sign-in runs on <span className="font-medium text-ink">{runner.name}</span>, {whose} that runs your
      sessions, because none of yours is online. Its machine sees your Claude login's long-lived tokens as
      they pass to fctl: it could keep them, or connect a Claude account of its own in place of yours, on
      which your sessions would then run. Go on only if you trust {runner.owner ?? "its owner"} with your
      Claude account; otherwise start one of your runners and start again, or use a long-lived token.
    </>
  );
}

/** The official sign-in, in three steps that tick off as the job goes. */
function SignInFlow({ onConnected }: { onConnected: () => void }) {
  const { current, start, submit, code, setCode } = useClaudeLogin(onConnected);
  const [opened, setOpened] = useState<string | null>(null);
  const state = current?.state;
  const starting = state === "pending" || state === "starting";
  const awaiting = current?.state === "awaiting_code" ? current : null;
  const past = state === "verifying" || state === "succeeded";
  return (
    <div className="flex flex-col gap-5">
      <ol>
        <Step n={1} done={!!awaiting || past} title="Start the sign-in">
          <p>
            Claude Code's own sign-in starts in a sandbox on one of your runners or, when none is online, on
            one that runs your sessions, whose machine sees the tokens.
          </p>
          {current?.runner && !current.runner.yours && signingIn(current) && (
            <Banner tone="warn">
              <OthersRunner runner={current.runner} />
            </Banner>
          )}
          {starting ? (
            <p className="flex items-center gap-2 text-ink-3" role="status">
              <Spinner className="size-3.5" /> Starting Claude Code in a sandbox…
            </p>
          ) : (
            !awaiting &&
            !past && (
              <div>
                <Button variant="primary" size="sm" loading={start.isPending} onClick={() => start.mutate()}>
                  {state === "failed" ? "Start again" : "Start the sign-in"}
                </Button>
              </div>
            )
          )}
        </Step>
        <Step n={2} done={past || (!!awaiting && opened === awaiting.id)} title="Approve on claude.ai">
          {awaiting ? (
            <>
              <p>Sign in with the account whose plan your sessions should run on, and approve.</p>
              <a
                href={awaiting.url}
                target="_blank"
                rel="noreferrer"
                onClick={() => setOpened(awaiting.id)}
                className={buttonClass({
                  variant: opened === awaiting.id ? "secondary" : "primary",
                  size: "sm",
                  className: "self-start",
                })}
              >
                Open claude.ai <ArrowUpRight className="size-3.5" />
              </a>
            </>
          ) : (
            <p className={cn(!past && "text-ink-4")}>
              {past ? "Approved." : "The link shows here once Claude Code is ready."}
            </p>
          )}
        </Step>
        <Step n={3} done={state === "succeeded"} title="Paste the code claude.ai shows">
          {awaiting ? (
            <form
              className="flex gap-2"
              onSubmit={(e) => {
                e.preventDefault();
                submit.mutate();
              }}
            >
              <Input
                aria-label="Code from claude.ai"
                value={code}
                onChange={(e) => setCode(e.target.value)}
                className="min-w-0 flex-1 font-mono"
                autoComplete="off"
              />
              <Button
                type="submit"
                variant="primary"
                size="field"
                loading={submit.isPending}
                disabled={!code.includes("#")}
              >
                Finish
              </Button>
            </form>
          ) : state === "verifying" ? (
            <p className="flex items-center gap-2 text-ink-3" role="status">
              <Spinner className="size-3.5" /> Verifying with Claude…
            </p>
          ) : (
            <p className="text-ink-4">Once you approve, claude.ai shows a code to bring back here.</p>
          )}
          {submit.error && (
            <p className="text-[12.5px] text-danger" role="alert">
              {submit.error.message}
            </p>
          )}
        </Step>
      </ol>
      {start.error && <Banner tone="error">{start.error.message}</Banner>}
      {current?.state === "failed" && <Banner tone="error">{current.error}</Banner>}
    </div>
  );
}

/**
 * Under a login Claude ended, which sessions can't run on: signing in again, or connecting a
 * long-lived token, which doesn't end for a year. Only logins end: Claude refuses to refresh them.
 * Tokens from `claude setup-token` aren't refreshed, so fctl never finds one ended.
 */
export function EndedLogin({ onReconnect }: { onReconnect: (way: Way) => void }) {
  return (
    <Banner tone="error">
      <p>
        Claude ended this login, so sessions can't run on it. Sign in again, or connect a long-lived token,
        which lasts a year.
      </p>
      <div className="mt-2.5 flex flex-wrap gap-2">
        <Button
          size="sm"
          variant="primary"
          icon={<KeyRound className="size-3.5" />}
          onClick={() => onReconnect("token")}
        >
          Connect a long-lived token
        </Button>
        <Button size="sm" variant="ghost" onClick={() => onReconnect("login")}>
          Sign in again
        </Button>
      </div>
    </Banner>
  );
}

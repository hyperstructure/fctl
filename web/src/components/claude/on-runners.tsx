// Keeping your Claude token on your runners (`fctl host claude set-token`), so that fctl never
// receives it: how to set it up, where each of your runners stands with it, and connecting it.

import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Check, ChevronDown } from "lucide-react";
import { useState } from "react";
import { Button, CodeBox, Skeleton, toast } from "@/components/ui";
import { api } from "@/lib/api";
import { type RunnerToken, yourRunnerTokens } from "@/lib/claude";
import { cn } from "@/lib/cn";
import { ago } from "@/lib/format";
import { keys, queryClient, useRunners, useWorkspaces } from "@/lib/queries";
import { Mono, Step } from "./guide";

/** Where `fctl host claude set-token` keeps the token, unless `host.toml` says otherwise. */
export const TOKEN_FILE = "~/.config/fctl/claude-token";

/** Commands, as people copy them. */
export const SETUP_TOKEN = "claude setup-token";
export const SET_TOKEN = "fctl host claude set-token";
/** The same on a machine nobody signs in on, from a file its secret store put there. */
export const SET_TOKEN_FROM_FILE = "fctl host claude set-token < /run/secrets/claude-token";

/** Steps 1 and 2: making a token and keeping it on a runner. */
function TokenSteps({ done, start = 1 }: { done: boolean; start?: number }) {
  return (
    <>
      <Step n={start} done={done} title="Create a token">
        <p>
          On the runner, or any machine with Claude Code. It opens claude.ai to sign in, then prints a token
          that starts with <Mono>sk-ant-oat01-</Mono> and lasts a year.
        </p>
        <CodeBox value={SETUP_TOKEN} />
      </Step>
      <Step n={start + 1} done={done} title="Keep it on the runner">
        <p>
          On the runner, paste the token when asked. fctl keeps it in <Mono>{TOKEN_FILE}</Mono>, which only
          you can read. It goes only into your sessions' sandboxes on that machine, and the runner takes it
          out of what they send to fctl wherever it shows up whole.
        </p>
        <CodeBox value={SET_TOKEN} />
        <p className="text-[12.5px] text-ink-4">
          Do it on each runner you want your sessions on. <Mono>fctl host claude status</Mono> shows what a
          runner keeps.
        </p>
      </Step>
    </>
  );
}

/** Where a runner stands with your token, in a few words. */
function TokenState({ entry: { runner: r, state } }: { entry: RunnerToken }) {
  if (state === "kept")
    return (
      <span className="inline-flex shrink-0 items-center gap-1.5 rounded-full bg-accent-soft px-2 py-0.5 text-[11.5px] whitespace-nowrap text-ink-2">
        <Check className="size-3 text-accent" />
        Keeps your token
      </span>
    );
  if (state === "old")
    return (
      <span
        className="shrink-0 text-right text-[12px] text-amber"
        title={`Its fctl can't keep one: run fctl self update on ${r.name}`}
      >
        Update its fctl first
      </span>
    );
  return <span className="shrink-0 text-right text-[12px] text-ink-4">No token yet</span>;
}

/** What a runner is, under its name: online or not, when it saved the token, where it works. */
function RunnerFacts({ entry: { runner: r, state } }: { entry: RunnerToken }) {
  const facts = [
    r.status === "online" ? "online" : r.status === "disabled" ? "disabled" : "offline",
    state === "kept" && r.claude_token?.saved_at ? `saved ${ago(r.claude_token.saved_at)}` : null,
    state === "old" ? "fctl self update, then fctl host claude set-token" : null,
    r.workspaces?.length ? r.workspaces.map((w) => w.name).join(", ") : "in no workspace yet",
  ].filter(Boolean);
  return <div className="truncate text-[12px] text-ink-4">{facts.join(" · ")}</div>;
}

/**
 * Your runners, each with where it stands with your token, as they report it (the feed brings what
 * changes). Without runners, where to bring one online.
 */
export function RunnerTokens({ className }: { className?: string }) {
  const runners = useRunners();
  const workspaces = useWorkspaces();
  if (!runners.data) return <Skeleton className={cn("h-14", className)} />;
  const list = yourRunnerTokens(runners.data.runners);
  if (list.length === 0) {
    const ws = workspaces.data?.workspaces ?? [];
    return (
      <p className={cn("text-[13px] leading-relaxed text-ink-3", className)}>
        You have no runners yet: a runner is a machine of yours that runs your sessions.{" "}
        {ws.length > 0 ? (
          <>
            Bring one online from the Runners tab of{" "}
            {ws.slice(0, 3).map((w, i) => (
              <span key={w.id}>
                {i > 0 && (i === Math.min(ws.length, 3) - 1 ? " or " : ", ")}
                <Link
                  to="/w/$slug"
                  params={{ slug: w.slug }}
                  search={{ tab: "runners" }}
                  className="text-ink-2 underline underline-offset-2 hover:text-ink"
                >
                  {w.name}
                </Link>
              </span>
            ))}
            .
          </>
        ) : (
          "Bring one online from a workspace's Runners tab."
        )}
      </p>
    );
  }
  return (
    <ul className={cn("divide-y divide-line rounded-lg border border-line", className)}>
      {list.map((entry) => (
        <li key={entry.runner.id} className="flex items-center gap-3 px-3 py-2.5">
          <span
            className={cn(
              "size-2 shrink-0 rounded-full",
              entry.runner.status === "disabled"
                ? "bg-danger"
                : entry.runner.status === "online"
                  ? "bg-accent shadow-[0_0_10px_var(--c-accent)]"
                  : "bg-ink-4",
            )}
          />
          <div className="min-w-0 flex-1">
            <div className="truncate text-[13.5px] text-ink">{entry.runner.name}</div>
            <RunnerFacts entry={entry} />
          </div>
          <TokenState entry={entry} />
        </li>
      ))}
    </ul>
  );
}

/** "Updates as they report": the list follows the runners live. */
function Live() {
  return (
    <span className="inline-flex items-center gap-1.5 text-[11.5px] text-ink-4">
      <span className="size-1.5 rounded-full bg-accent motion-safe:animate-pulse" />
      Updates as your runners report
    </span>
  );
}

/**
 * Settings → Claude, when you choose to keep a long-lived token on your runners: the steps that set
 * it up, with where your runners stand as they go, and connecting it.
 */
export function OnRunnersSteps({ onConnected }: { onConnected?: () => void }) {
  const runners = useRunners();
  const kept = yourRunnerTokens(runners.data?.runners ?? []).filter((r) => r.state === "kept");
  const connect = useMutation({
    mutationFn: () => api.claudeOnRunners(),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      toast("New sessions run on the token your runners keep");
      onConnected?.();
    },
  });
  return (
    <ol>
      <TokenSteps done={kept.length > 0} />
      <Step n={3} done={kept.length > 0} title="Check your runners" aside={<Live />}>
        <RunnerTokens />
      </Step>
      <Step n={4} done={false} title="Run your sessions on it">
        <p>
          New sessions run on it from then on, in every workspace where you didn't choose another connection:
          only on your runners that keep the token, and they wait while none of those is online.
        </p>
        {kept.length === 0 && runners.data && (
          <p className="text-[12.5px] text-amber">
            None of your runners keeps it yet: sessions you start wait until one does.
          </p>
        )}
        <div>
          <Button variant="primary" size="sm" loading={connect.isPending} onClick={() => connect.mutate()}>
            Use the token on my runners
          </Button>
        </div>
        {connect.error && <p className="text-[12.5px] text-danger">{connect.error.message}</p>}
      </Step>
    </ol>
  );
}

/**
 * Under the connection whose token you keep on your runners: where each of them stands with it, and
 * the steps that keep it on another one.
 */
export function OnRunnersDetails() {
  const runners = useRunners();
  const [more, setMore] = useState(false);
  const list = yourRunnerTokens(runners.data?.runners ?? []);
  const none = runners.data && !list.some((r) => r.state === "kept");
  return (
    <div className="flex flex-col gap-3">
      {none && (
        <p className="text-[12.5px] leading-relaxed text-amber">
          None of your runners keeps the token yet, so sessions on this connection wait. Keep it on one:
        </p>
      )}
      <RunnerTokens />
      {none ? (
        <ol className="mt-2">
          <TokenSteps done={false} />
        </ol>
      ) : (
        <div>
          <button
            type="button"
            onClick={() => setMore((m) => !m)}
            aria-expanded={more}
            className="inline-flex items-center gap-1 rounded text-[12.5px] text-ink-3 transition-colors hover:text-ink"
          >
            <ChevronDown
              className={cn(
                "size-3.5 transition-transform motion-reduce:transition-none",
                more && "rotate-180",
              )}
            />
            Keep it on another runner
          </button>
          {more && (
            <ol className="mt-4 animate-rise">
              <TokenSteps done={false} />
            </ol>
          )}
        </div>
      )}
    </div>
  );
}

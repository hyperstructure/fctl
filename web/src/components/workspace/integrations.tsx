// A workspace's Integrations tab: GitHub, Linear and Slack side by side, each with its mark and how
// it stands for the workspace (connected, not yet, access revoked), then the open one's page. Each is
// an address (`?tab=integrations&integration=slack`), so that a sign-in elsewhere comes back to it.

import { useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import type { ReactNode } from "react";
import { GithubIcon, LinearIcon, Skeleton, SlackIcon } from "@/components/ui";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, useInstance } from "@/lib/queries";
import {
  githubStanding,
  INTEGRATION_NAMES,
  type Integration,
  linearStanding,
  type Standing,
  slackStanding,
} from "@/lib/slack";

const MARKS: Record<Integration, (className: string) => ReactNode> = {
  github: (className) => <GithubIcon className={className} />,
  // Linear's mark is filled: a little smaller, it weighs as much as the others.
  linear: (className) => <LinearIcon className={cn(className, "scale-[0.85]")} />,
  slack: (className) => <SlackIcon className={className} />,
};

/** How each integration stands for the workspace, from what the tab's pages ask anyway. */
function useStandings(w: G.Workspace, shown: readonly Integration[]): Record<Integration, Standing> {
  const { data: instance } = useInstance();
  const linear = useQuery({
    queryKey: keys.workspaceLinear(w.id),
    queryFn: () => workspaces.linear(w.id),
    enabled: shown.includes("linear"),
  });
  const slack = useQuery({
    queryKey: keys.workspaceSlack(w.id),
    queryFn: () => workspaces.slack(w.id),
    enabled: shown.includes("slack"),
  });
  return {
    github: githubStanding(w.github, instance?.features.github ?? true),
    linear: linear.isError ? FAILED : linearStanding(linear.data),
    slack: slack.isError ? FAILED : slackStanding(slack.data),
  };
}

/** What the switcher says of one that didn't load: its page says why, with Try again. */
const FAILED: Standing = { state: "off", text: "Didn't load", short: "Didn't load" };

const DOT: Record<Standing["state"], string> = {
  on: "bg-accent shadow-[0_0_8px_var(--c-accent)]",
  off: "bg-ink-4",
  wrong: "bg-amber",
  loading: "bg-ink-4",
};

/**
 * The integrations the workspace has, to move between (where there are several), and the open one's
 * page under them.
 */
export function Integrations({
  w,
  shown,
  open,
  children,
}: {
  w: G.Workspace;
  /** Those this instance has: GitHub always, Linear and Slack where it has their apps. */
  shown: readonly Integration[];
  open: Integration;
  children: ReactNode;
}) {
  const standings = useStandings(w, shown);
  return (
    <div className="flex flex-col gap-9">
      {shown.length > 1 && (
        <nav aria-label="Integrations">
          <ul className={cn("grid gap-2 sm:gap-2.5", shown.length === 2 ? "grid-cols-2" : "grid-cols-3")}>
            {shown.map((id) => {
              const here = id === open;
              const s = standings[id];
              return (
                <li key={id} className="min-w-0">
                  <Link
                    to="/w/$slug"
                    params={{ slug: w.slug }}
                    search={{ tab: "integrations", integration: id }}
                    aria-current={here ? "page" : undefined}
                    activeOptions={{ exact: true }}
                    className={cn(
                      "group flex h-full flex-col gap-2 rounded-xl border p-3 transition-[background-color,border-color,box-shadow] duration-150 sm:flex-row sm:items-center sm:gap-3 sm:p-3.5",
                      here
                        ? "border-accent/55 bg-accent-soft/40 shadow-[inset_0_0_0_1px_var(--c-accent-soft)]"
                        : "border-line bg-surface/60 hover:border-line-strong hover:bg-hover active:bg-hover",
                    )}
                  >
                    <span
                      aria-hidden="true"
                      className={cn(
                        "flex size-8 shrink-0 items-center justify-center rounded-lg border bg-raised transition-colors",
                        here ? "border-accent/40 text-ink" : "border-line text-ink-2 group-hover:text-ink",
                      )}
                    >
                      {MARKS[id]("size-4")}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="block text-[14px] leading-5 font-medium text-ink">
                        {INTEGRATION_NAMES[id]}
                      </span>
                      {s.state === "loading" ? (
                        <Skeleton className="mt-1.5 h-3 w-20 max-w-full" />
                      ) : (
                        <span
                          className={cn(
                            "mt-0.5 flex min-w-0 items-start gap-1.5 text-[12px] leading-4",
                            s.state === "wrong" ? "text-amber" : "text-ink-3",
                          )}
                        >
                          <span
                            aria-hidden="true"
                            className={cn("mt-[5px] size-1.5 shrink-0 rounded-full", DOT[s.state])}
                          />
                          {/* On a phone, a word or two: the page under it says the rest. */}
                          <span className="min-w-0 truncate sm:hidden">{s.short}</span>
                          <span className="min-w-0 truncate max-sm:hidden">{s.text}</span>
                        </span>
                      )}
                    </span>
                  </Link>
                </li>
              );
            })}
          </ul>
        </nav>
      )}
      <div key={open} className="min-w-0 animate-rise">
        {children}
      </div>
    </div>
  );
}

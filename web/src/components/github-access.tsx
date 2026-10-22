// What a session's agent may do on GitHub, by a symbol: an arrow up for an agent that pushes, a
// speech bubble for one that reviews and comments, an eye for one that only reads.

import { ArrowUpFromLine, Eye, MessageSquareText } from "lucide-react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { githubLevel } from "@/lib/github-access";

const ICONS = { push: ArrowUpFromLine, comment: MessageSquareText, read: Eye } as const;

export function GithubAccessIcon({ access, className }: { access: G.GithubAccess; className?: string }) {
  const Icon = ICONS[access];
  return <Icon className={cn("size-3.5 shrink-0", className)} aria-hidden="true" />;
}

/** The symbol and the label, as a session's details list it: inline, so that it keeps to the text's baseline. */
export function GithubAccessLabel({ access }: { access: G.GithubAccess }) {
  const level = githubLevel(access);
  const Icon = ICONS[access];
  return (
    <span title={level.description}>
      <Icon className="mr-1.5 inline size-3 align-[-1px]" aria-hidden="true" />
      {level.label}
    </span>
  );
}

/** A session whose agent does less than push says so beside its status; one that pushes, as all did, doesn't. */
export function GithubAccessChip({ access, className }: { access: G.GithubAccess; className?: string }) {
  if (access === "push") return null;
  const level = githubLevel(access);
  return (
    <span
      title={level.description}
      className={cn(
        "inline-flex shrink-0 items-center gap-1 rounded-full border border-line-strong px-2 py-px text-[11.5px] text-ink-2",
        className,
      )}
    >
      <GithubAccessIcon access={access} className="size-3" />
      {level.label}
    </span>
  );
}

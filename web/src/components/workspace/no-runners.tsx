// Under the forms that start sessions: that the workspace's sessions wait, while none of its
// runners is online, and what brings one online.

import { Link } from "@tanstack/react-router";
import { Cpu } from "lucide-react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { useRunners } from "@/lib/queries";
import { runnersOf } from "@/lib/workspaces";

/** Nothing while one of `workspace`'s runners is online, or until the runners load. */
export function NoRunnerNote({ workspace: w, className }: { workspace?: G.Workspace; className?: string }) {
  const runners = useRunners();
  const count = w && runners.data && runnersOf(runners.data.runners, w.id);
  if (!w || !count || count.online > 0) return null;
  const admin = w.role === "owner" || w.role === "admin";
  const link = (label: string) => (
    <Link
      to="/w/$slug"
      params={{ slug: w.slug }}
      search={{ tab: "runners" }}
      className="text-ink-2 underline underline-offset-2 transition-colors hover:text-ink"
    >
      {label}
    </Link>
  );
  return (
    <p className={cn("flex gap-2 px-1 text-[12.5px] leading-relaxed text-ink-3", className)}>
      <Cpu className="mt-[3px] size-3.5 shrink-0 text-ink-4" />
      <span>
        {count.total === 1 ? (
          <>
            {w.name}'s runner isn't online: sessions you start in it wait until it is.{" "}
            {link("See its runner")}
          </>
        ) : count.total > 1 ? (
          <>
            None of {w.name}'s runners is online: sessions you start in it wait until one is.{" "}
            {link("See its runners")}
          </>
        ) : admin ? (
          <>
            {w.name} has no runners yet: sessions you start in it wait until one is online.{" "}
            {link("Add a runner")}
          </>
        ) : (
          <>
            {w.name} has no runners yet: sessions you start in it wait until its admins add one.
            {w.primary && ` Ask ${w.primary.name}, its primary admin.`}
          </>
        )}
      </span>
    </p>
  );
}

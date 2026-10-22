// Under the forms that start sessions: that the workspace's sessions wait, while none of its
// runners is online, and what brings one online.

import { Link } from "@tanstack/react-router";
import { Cpu } from "lucide-react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { useRunners } from "@/lib/queries";
import { runnersOf } from "@/lib/workspaces";

/**
 * Nothing while one of the runners that run your sessions in `workspace` is online, or until the
 * runners load. Another member's runner for their own sessions doesn't count, nor, while you keep
 * your sessions there on your own runners, anyone else's (`runnersOf`).
 */
export function NoRunnerNote({ workspace: w, className }: { workspace?: G.Workspace; className?: string }) {
  const runners = useRunners();
  const count = w && runners.data && runnersOf(runners.data.runners, w);
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
        {w.own_runners ? (
          count.total > 0 ? (
            <>
              None of your runners in {w.name} is online: your sessions here run only on yours, so sessions
              you start wait until one is. {link("See its runners")}
            </>
          ) : (
            <>
              Your sessions in {w.name} run only on your runners, and none of yours is in it yet: sessions you
              start here wait until one is. {link("Add a runner")}
            </>
          )
        ) : count.total === 1 ? (
          <>
            {w.name}'s runner isn't online: sessions you start here wait until it is. {link("See its runner")}
          </>
        ) : count.total > 1 ? (
          <>
            None of {w.name}'s runners is online: sessions you start here wait until one is.{" "}
            {link("See its runners")}
          </>
        ) : count.others > 0 ? (
          <>
            {w.name}'s runners run only their owners' sessions: sessions you start here wait until{" "}
            {admin ? "one for everyone's is online" : "its admins add one for everyone's, or you add yours"}.{" "}
            {link("Add a runner")}
          </>
        ) : admin ? (
          <>
            {w.name} has no runners yet: sessions you start here wait until one is online.{" "}
            {link("Add a runner")}
          </>
        ) : (
          <>
            {w.name} has no runners yet: sessions you start here wait until its admins add one
            {w.primary && ` (ask ${w.primary.name}, its primary admin)`}, or you add one of yours.{" "}
            {link("Add a runner")}
          </>
        )}
      </span>
    </p>
  );
}

// A runner's row on a workspace's Runners tab: its dot, name and state, pills where they matter,
// its facts, the lines that show only when true, and what you may do with it. Under the list, or
// under the pool it enrolled in.

import { KeyRound, ShieldCheck } from "lucide-react";
import { DriverLine } from "@/components/runner-driver";
import { SnapshotStoreLine } from "@/components/runner-snapshots";
import { Button } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { ManageRunner } from "@/components/workspace/runners";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { minutesText } from "@/lib/enrollment";
import { ago, driverProblem } from "@/lib/format";

export function RunnerRow({
  w,
  runner: r,
  admin,
  live,
  onTakeOut,
  inPool,
}: {
  w: G.Workspace;
  runner: G.Runner;
  admin: boolean;
  /** What the feed says of your runners, fresher than the list. */
  live: Map<string, G.Runner>;
  /** Asks before taking it out of the workspace, or removing one that enrolled here. */
  onTakeOut: (r: G.Runner) => void;
  /** Under its pool, which says what it enrolled with and when it goes. */
  inPool?: boolean;
}) {
  const others = (r.workspaces ?? []).filter((x) => x.id !== w.id);
  // It enrolled here: the workspace's own, which its admins manage, or a member's for
  // their own sessions, which they manage and which goes when it is taken out.
  const enrolled = r.workspace?.id === w.id;
  const own = enrolled && !r.created_by;
  return (
    <li key={r.id} className="flex items-center gap-4 border-b border-line py-4">
      <span
        className={cn(
          "size-2 shrink-0 rounded-full",
          r.status === "disabled"
            ? "bg-danger"
            : r.drain || driverProblem(r)
              ? "bg-amber"
              : r.status === "online"
                ? "bg-accent shadow-[0_0_10px_var(--c-accent)]"
                : "bg-ink-4",
        )}
      />
      <div className="min-w-0 flex-1">
        <div className="text-[14px] text-ink">
          {r.name}{" "}
          <span className="ml-1 text-[12px] text-ink-4">
            {r.status === "online" && r.drain ? "draining" : driverProblem(r) ? "driver not ready" : r.status}
          </span>
          {r.personal?.includes(w.id) && (
            <span className="ml-2 inline-block whitespace-nowrap rounded-full border border-line px-2 py-px text-[11.5px] text-ink-3">
              {r.yours ? "Only your sessions" : `Only ${r.created_by?.name ?? "its owner"}'s sessions`}
            </span>
          )}
          {r.yours && (live.get(r.id) ?? r).claude_token?.saved_at && (
            <span className="ml-2 inline-flex items-center gap-1 whitespace-nowrap rounded-full bg-accent-soft px-2 py-px align-middle text-[11.5px] text-ink-2">
              <ShieldCheck className="size-3 text-accent" />
              Keeps your Claude token
            </span>
          )}
        </div>
        <div className="mt-0.5 truncate text-[12px] text-ink-4">
          {own ? `${w.name}'s` : r.created_by ? `${r.created_by.name}'s` : "A"}{" "}
          {r.driver ? `${r.driver} runner` : "runner"} · {r.active_sessions}/{r.capacity} sessions
          {r.last_seen_at ? ` · seen ${ago(r.last_seen_at)}` : " · never connected"}
        </div>
        <DriverLine runner={r} />
        <SnapshotStoreLine runner={r} />
        {!inPool && enrolled && (r.enrollment?.token_name || r.remove_offline_after_minutes) && (
          <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[11.5px] text-ink-4">
            {r.enrollment?.token_name && (
              <span className="inline-flex items-center gap-1 text-ink-3">
                <KeyRound className="size-3 text-ink-4" /> Enrolled with “{r.enrollment.token_name}”
              </span>
            )}
            {r.remove_offline_after_minutes && (
              <span>Removed once offline for {minutesText(r.remove_offline_after_minutes)}</span>
            )}
          </div>
        )}
        {others.length > 0 && (
          <div className="mt-1.5 flex flex-wrap items-center gap-1.5 text-[11.5px] text-ink-4">
            Also runs
            {others.map((x) => (
              <span key={x.id} className="inline-flex items-center gap-1 text-ink-3">
                <WorkspaceAvatar workspace={x} size={13} /> {x.name}
              </span>
            ))}
          </div>
        )}
      </div>
      {own || (enrolled && r.yours)
        ? (admin || r.yours) && <ManageRunner w={w} runner={r} />
        : (admin || r.yours) && (
            <Button size="sm" variant="ghost" onClick={() => onTakeOut(r)}>
              {enrolled ? "Remove" : "Take out"}
            </Button>
          )}
    </li>
  );
}

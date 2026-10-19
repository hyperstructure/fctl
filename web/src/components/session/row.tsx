// A session in a list (the floor, a workspace's page): its status, title,
// workspace and reference, who started it and when it last moved.

import { Link } from "@tanstack/react-router";
import { ArrowRight } from "lucide-react";
import { Avatar, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { agoShort, stamp } from "@/lib/format";
import { statusReason } from "@/lib/status";

export function SessionRow({ s, workspace }: { s: G.Session; workspace?: G.WorkspaceRef }) {
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      className="group flex items-center gap-3.5 border-b border-line p-3 transition-colors last:border-b-0 hover:bg-hover/30"
    >
      <StatusDot status={s.status} />
      <div className="min-w-0 flex-1">
        <div className="truncate text-[14.5px] text-ink transition-colors group-hover:text-ink">
          {s.title}
        </div>
        <div className="mt-0.5 flex min-w-0 items-center gap-2 text-[12px] text-ink-4">
          {workspace && (
            <span className="inline-flex min-w-0 shrink-0 items-center gap-1.5 text-ink-3">
              <WorkspaceAvatar workspace={workspace} size={14} />
              <span className="max-w-[10rem] truncate">{workspace.name}</span>
              <span className="text-ink-4">/</span>
            </span>
          )}
          <span className="shrink-0 font-mono">
            {s.project_slug}#{s.number}
          </span>
          {statusReason(s) && <span className="truncate">· {statusReason(s)}</span>}
          {s.stats.files_changed > 0 && (
            <span className="shrink-0">
              · <span className="text-accent/80">+{s.stats.additions}</span>{" "}
              <span className="text-danger/80">−{s.stats.deletions}</span>
            </span>
          )}
          {s.pull_request && <span className="shrink-0">· PR #{s.pull_request.number}</span>}
        </div>
      </div>
      <Avatar user={s.created_by} size={20} />
      <time
        dateTime={new Date(s.last_activity_at ?? s.created_at).toISOString()}
        title={stamp(s.last_activity_at ?? s.created_at)}
        className="w-10 shrink-0 text-right text-[12px] whitespace-nowrap text-ink-4 tabular-nums"
      >
        {agoShort(s.last_activity_at ?? s.created_at)}
      </time>
      <ArrowRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-all duration-200 group-hover:translate-x-0.5 group-hover:opacity-100" />
    </Link>
  );
}

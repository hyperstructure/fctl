// A project's retrospectives: what its agents recorded as they finished
// pieces of work, newest first, by outcome. What a future agent should know
// goes into the project's memory from here.

import { useInfiniteQuery, useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Ellipsis, ScrollText, Trash2 } from "lucide-react";
import { useState } from "react";
import { OutcomeIcon, RetrospectiveParts } from "@/components/retrospective";
import {
  Avatar,
  Banner,
  Button,
  Dialog,
  Empty,
  IconButton,
  Menu,
  MenuItem,
  Skeleton,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago, stamp } from "@/lib/format";
import { canEditMemory } from "@/lib/memory";
import { keys, queryClient, useMe } from "@/lib/queries";
import { canDelete, OUTCOMES, outcomeLabel, total } from "@/lib/retrospectives";

export function RetrospectivesTab({ project }: { project: G.Project }) {
  const { data: me } = useMe();
  const [outcome, setOutcome] = useState<G.RetrospectiveOutcome>();
  const list = useInfiniteQuery({
    queryKey: keys.retrospectives(project.id, outcome),
    queryFn: ({ pageParam }) => api.retrospectives(project.id, { outcome, before: pageParam, limit: 20 }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.next_before ?? undefined,
  });
  const pages = list.data?.pages ?? [];
  const counts = pages[0]?.outcomes;
  const all = total(counts);
  const items = pages.flatMap((p) => p.retrospectives);
  const manage = project.role === "owner" || project.role === "maintainer";
  // Insights go into the memory for people who may change it.
  const remember = canEditMemory(project.role) ? project : undefined;
  return (
    <div className="flex flex-col gap-6">
      <p className="max-w-2xl text-[13.5px] leading-relaxed text-ink-3">
        What this project's agents recorded as they finished pieces of work: what they worked on, the approach
        they took, what got in the way and what a future agent should know.
        {remember ? " Put an insight in the project's memory, and the agents after them start with it." : ""}
      </p>
      {!project.settings.retrospectives && (
        <Banner tone="info">
          This project doesn't ask its agents for retrospectives.{" "}
          {manage ? (
            <>
              Turn them on under{" "}
              <Link
                to="/p/$slug"
                params={{ slug: project.slug }}
                search={{ tab: "settings" }}
                className="underline underline-offset-2"
              >
                Settings
              </Link>
              .
            </>
          ) : (
            "Its maintainers can turn them on."
          )}
        </Banner>
      )}
      {list.isPending ? (
        <div className="space-y-3 py-2">
          <Skeleton className="h-7 w-80" />
          <Skeleton className="h-28" />
          <Skeleton className="h-28" />
        </div>
      ) : all === 0 && !outcome ? (
        <Empty title="No retrospectives yet" icon={<ScrollText className="size-7" strokeWidth={1.5} />}>
          {project.settings.retrospectives
            ? "Agents record one as they finish a piece of work. They show here, and on each session's page."
            : "Once its maintainers turn them on, agents record one as they finish a piece of work."}
        </Empty>
      ) : (
        <section>
          {/* The outcomes, with how many ended each way: the filters, and the legend of the icons below. */}
          <div className="mb-1 flex flex-wrap items-center gap-1">
            <Pill active={!outcome} onClick={() => setOutcome(undefined)} label="All" count={all} />
            {OUTCOMES.map((o) => (
              <Pill
                key={o.value}
                active={outcome === o.value}
                onClick={() => setOutcome(outcome === o.value ? undefined : o.value)}
                label={o.label}
                title={o.description}
                count={counts?.[o.value] ?? 0}
                icon={<OutcomeIcon outcome={o.value} />}
              />
            ))}
          </div>
          {items.length === 0 ? (
            <p className="border-t border-line py-6 text-[13px] text-ink-4">
              None of this project's retrospectives says{" "}
              {outcome ? outcomeLabel(outcome).toLowerCase() : "that"}.
            </p>
          ) : (
            <ul className="border-t border-line">
              {items.map((r) => (
                <Entry
                  key={r.id}
                  r={r}
                  project={project}
                  remember={remember}
                  deletable={canDelete(r, project.role, me?.user.id)}
                />
              ))}
            </ul>
          )}
          {list.isError && <p className="mt-4 text-[13px] text-danger">{list.error.message}</p>}
          {list.hasNextPage && (
            <div className="mt-4">
              <Button
                size="sm"
                variant="ghost"
                loading={list.isFetchingNextPage}
                onClick={() => void list.fetchNextPage()}
              >
                Older
              </Button>
            </div>
          )}
        </section>
      )}
    </div>
  );
}

function Pill({
  active,
  onClick,
  label,
  count,
  icon,
  title,
}: {
  active: boolean;
  onClick: () => void;
  label: string;
  count: number;
  icon?: React.ReactNode;
  title?: string;
}) {
  return (
    <button
      type="button"
      aria-pressed={active}
      onClick={onClick}
      title={title}
      className={cn(
        "inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-[12.5px] transition-colors",
        active ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]" : "text-ink-3 hover:text-ink",
      )}
    >
      {icon}
      {label}
      <span className="text-ink-4 tabular-nums">{count}</span>
    </button>
  );
}

function Entry({
  r,
  project,
  remember,
  deletable,
}: {
  r: G.Retrospective;
  project: G.Project;
  remember?: G.Project;
  deletable: boolean;
}) {
  const [confirm, setConfirm] = useState(false);
  const del = useMutation({
    mutationFn: () => api.deleteRetrospective(r.id),
    onSuccess: () => {
      setConfirm(false);
      toast("Deleted the retrospective");
      void queryClient.invalidateQueries({ queryKey: keys.projectRetrospectives(project.id) });
      void queryClient.invalidateQueries({ queryKey: keys.sessionRetrospectives(r.session.id) });
    },
  });
  return (
    <li className="group/retro border-b border-line py-5">
      <div className="mb-3 flex items-center gap-2.5">
        <OutcomeIcon outcome={r.outcome} className="size-4" />
        <div className="flex min-w-0 flex-1 flex-wrap items-baseline gap-x-2 gap-y-0.5">
          <span className="text-[12.5px] font-medium text-ink-2">{outcomeLabel(r.outcome)}</span>
          <Link
            to="/s/$id"
            params={{ id: r.session.id }}
            className="min-w-0 text-[14px] text-ink transition-colors [overflow-wrap:anywhere] hover:text-accent"
          >
            <span className="font-mono text-[12.5px] text-ink-3">#{r.session.number}</span> {r.session.title}
          </Link>
        </div>
        {r.started_by && <Avatar user={r.started_by} size={18} />}
        <time
          dateTime={new Date(r.created_at).toISOString()}
          title={stamp(r.created_at)}
          className="shrink-0 text-[12px] whitespace-nowrap text-ink-4"
        >
          {ago(r.created_at)}
        </time>
        {deletable && (
          <Menu
            trigger={(toggle, open) => (
              <IconButton
                label="Retrospective actions"
                aria-haspopup="menu"
                aria-expanded={open}
                onClick={toggle}
                className={cn(
                  "-my-1 size-7 opacity-0 transition-opacity group-hover/retro:opacity-100 focus-visible:opacity-100 pointer-coarse:opacity-100",
                  open && "opacity-100",
                )}
              >
                <Ellipsis className="size-4" />
              </IconButton>
            )}
          >
            {(close) => (
              <MenuItem
                danger
                icon={<Trash2 />}
                onClick={() => {
                  close();
                  setConfirm(true);
                }}
              >
                Delete…
              </MenuItem>
            )}
          </Menu>
        )}
      </div>
      <RetrospectiveParts r={r} wide remember={remember} className="sm:pl-6.5" />
      {confirm && (
        <Dialog
          open={confirm}
          onClose={() => setConfirm(false)}
          title="Delete this retrospective?"
          description={`It goes from this list and from #${r.session.number}'s page for good. The session's transcript keeps what the agent recorded.`}
        >
          <div className="flex flex-col gap-5">
            {del.error && <Banner tone="error">{del.error.message}</Banner>}
            <div className="flex justify-end gap-2">
              <Button variant="ghost" onClick={() => setConfirm(false)}>
                Cancel
              </Button>
              <Button variant="danger" loading={del.isPending} onClick={() => del.mutate()}>
                Delete
              </Button>
            </div>
          </div>
        </Dialog>
      )}
    </li>
  );
}

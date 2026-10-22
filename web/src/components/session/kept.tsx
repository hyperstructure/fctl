// What a session's agent kept for the people and agents after it, as the
// session page's rail and details sheet show it: its retrospectives, and
// the notes it left in the project's memory.

import { useQuery } from "@tanstack/react-query";
import { Archive, BookMarked } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { NoteLine, NoteLink } from "@/components/project/memory";
import { OutcomeBadge, RetrospectiveParts, RetrospectiveTime } from "@/components/retrospective";
import { Dialog } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { canEditMemory, headline, ofSession } from "@/lib/memory";
import { keys } from "@/lib/queries";
import { unlisted } from "@/lib/retrospectives";
import type { Item } from "@/lib/transcript";

/**
 * The session's retrospectives, newest first, as the control plane lists
 * them. One that comes in the transcript, which the list doesn't have yet,
 * or an update newer than the list's, reads the list again, once; a
 * deleted one stays in the transcript and out of the list.
 */
export function useRetrospectives(session: G.Session, items: Item[]) {
  const listed = useQuery({
    queryKey: keys.sessionRetrospectives(session.id),
    queryFn: () => api.sessionRetrospectives(session.id),
  });
  const missing = listed.isSuccess ? unlisted(listed.data.retrospectives, items) : [];
  const asked = useRef(new Set<string>());
  const refetch = listed.refetch;
  const key = missing.join(" ");
  useEffect(() => {
    const unasked = key.split(" ").filter((id) => id && !asked.current.has(id));
    if (!unasked.length) return;
    for (const id of unasked) asked.current.add(id);
    void refetch();
  }, [key, refetch]);
  return listed.data?.retrospectives ?? [];
}

/**
 * The session's retrospectives: the latest one's outcome and what it
 * worked on, and all of them in full a click away.
 */

export function SessionRetrospectives({ list, project }: { list: G.Retrospective[]; project?: G.Project }) {
  const [open, setOpen] = useState(false);
  const latest = list[0];
  if (!latest)
    return (
      <p className="text-[12.5px] leading-relaxed text-ink-4">
        The agent records one as it finishes a piece of work: what it worked on, how, what got in the way and
        what a future agent should know.
      </p>
    );
  return (
    <div>
      <button
        type="button"
        onClick={() => setOpen(true)}
        className="-mx-2 -my-1.5 block w-[calc(100%+1rem)] rounded-md px-2 py-1.5 text-left transition-colors hover:bg-hover"
      >
        <span className="flex items-center justify-between gap-2">
          <OutcomeBadge outcome={latest.outcome} />
          <RetrospectiveTime r={latest} />
        </span>
        <span className="mt-2 line-clamp-4 block text-[12.5px] leading-relaxed text-ink-2 [overflow-wrap:anywhere]">
          {latest.worked_on}
        </span>
        <span className="mt-1.5 block text-[12px] text-ink-3">
          {list.length > 1 ? `Read all ${list.length}` : "Read it"}
        </span>
      </button>
      <Dialog
        open={open}
        onClose={() => setOpen(false)}
        wide
        title={list.length > 1 ? `${list.length} retrospectives` : "Retrospective"}
        description={
          list.length > 1
            ? "What the agent recorded as it finished each piece of work, newest first."
            : "What the agent recorded as it finished the work."
        }
      >
        <ol className="-mx-6 max-h-[70dvh] overflow-y-auto border-t border-line px-6">
          {list.map((r) => (
            <li key={r.id} className="border-b border-line py-4 last:border-b-0">
              <div className="mb-3 flex items-center gap-2">
                <OutcomeBadge outcome={r.outcome} />
                <RetrospectiveTime r={r} />
              </div>
              <RetrospectiveParts
                r={r}
                wide
                remember={project && canEditMemory(project.role) ? project : undefined}
              />
            </li>
          ))}
        </ol>
      </Dialog>
    </div>
  );
}

/**
 * The project's memory as the session page shows it: the notes its agent
 * left and those it marked stale, if there is anything to show: notes it
 * left, or a project that keeps a memory.
 */
export function useSessionMemory(session: G.Session, project?: G.Project) {
  const memories = useQuery({
    queryKey: keys.sessionMemories(project?.id ?? "", session.id),
    queryFn: () => api.memories(project?.id ?? "", { session: session.id }),
    enabled: !!project,
  });
  const kept = ofSession(memories.data?.memories ?? [], session.id);
  const shown = !!project && (project.settings.project_memory || kept.added.length + kept.staled.length > 0);
  return { ...kept, current: memories.data?.current ?? 0, shown };
}

/** The notes the session's agent left in the project's memory, and those it marked stale. */
export function SessionMemory({
  project,
  memory: { added, staled, current },
}: {
  project: G.Project;
  memory: ReturnType<typeof useSessionMemory>;
}) {
  return (
    <div className="flex flex-col gap-2.5">
      {added.length + staled.length === 0 ? (
        <p className="text-[12.5px] leading-relaxed text-ink-4">
          Its agent left no notes yet. What it learns that a future agent should know goes in the project's
          memory.
        </p>
      ) : (
        <ul className="flex flex-col gap-2">
          {added.map((m) => (
            <li key={m.id} className="flex items-start gap-2 text-[12.5px] leading-relaxed text-ink-2">
              <BookMarked className="mt-[3px] size-3.5 shrink-0 text-accent" />
              <NoteLink
                slug={project.slug}
                id={m.id}
                arrow={false}
                className={cn(
                  "line-clamp-3 min-w-0 [overflow-wrap:anywhere]",
                  m.stale && "text-ink-4 line-through",
                )}
              >
                <NoteLine text={headline(m.text, 400)} />
              </NoteLink>
            </li>
          ))}
          {staled.map((m) => (
            <li key={m.id} className="flex items-start gap-2 text-[12.5px] leading-relaxed text-ink-3">
              <Archive className="mt-[3px] size-3.5 shrink-0 text-ink-4" />
              <NoteLink
                slug={project.slug}
                id={m.id}
                arrow={false}
                className="line-clamp-3 min-w-0 [overflow-wrap:anywhere]"
              >
                <span className="text-ink-4">
                  {m.stale?.duplicate_of ? "Marked a duplicate:" : "Marked stale:"}
                </span>{" "}
                <NoteLine text={headline(m.text, 400)} />
              </NoteLink>
            </li>
          ))}
        </ul>
      )}
      <NoteLink slug={project.slug} className="self-start text-[12px] text-ink-3">
        {current === 1 ? "The project's 1 note" : `The project's ${current} notes`}
      </NoteLink>
    </div>
  );
}

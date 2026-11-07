// The new session dialog: the Agent Floor's task box over any page (N, or ⌘⇧O even while typing,
// or New session in ⌘K), to start a session without leaving what you're looking at. It is the
// same box as the Agent Floor's, with the same draft; a new draft starts in the project the page
// is about. Starting the session opens it, which closes the dialog (app-shell.tsx).

import { useQueryClient } from "@tanstack/react-query";
import { Link, useRouterState } from "@tanstack/react-router";
import { X } from "lucide-react";
import { useId, useLayoutEffect, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { startableProjects, TaskBox } from "@/components/session/task-box";
import { IconButton, Skeleton } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { own } from "@/lib/dialog";
import { keys, useMe, useProjects, useWorkspaces } from "@/lib/queries";

export function NewSessionDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  // What it shows mounts once it is open, so that the task box takes the focus (ui/overlay.tsx).
  const [shown, setShown] = useState(false);
  // A press that starts beside the box, not a selection dragged out of it, closes the dialog.
  const pressedOutside = useRef(false);
  useLayoutEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
    setShown(open);
  }, [open]);
  return createPortal(
    // biome-ignore lint/a11y/useKeyWithClickEvents: the click closes beside the box only; Escape is native to <dialog>.
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onClose={own(onClose)}
      onMouseDown={(e) => {
        pressedOutside.current = e.target === ref.current;
      }}
      onClick={(e) => {
        if (e.target === ref.current && pressedOutside.current) ref.current?.close();
      }}
      className={cn(
        // Under the top of the window, where ⌘K's palette is: it grows down as the task does.
        "top-(--viewport-shift) -bottom-(--viewport-shift) mx-auto mt-[13vh] mb-auto w-[min(872px,calc(100vw-2rem))] max-w-none overflow-visible bg-transparent p-0 text-ink outline-none",
        "backdrop:bg-black/40 backdrop:backdrop-blur-[3px] open:animate-rise motion-reduce:open:animate-none",
      )}
    >
      {open && shown && <Content titleId={titleId} onClose={() => ref.current?.close()} />}
    </dialog>,
    document.body,
  );
}

function Content({ titleId, onClose }: { titleId: string; onClose: () => void }) {
  const { data: me } = useMe();
  const projects = useProjects().data?.projects;
  const workspaces = useWorkspaces().data?.workspaces;
  const here = useHere(projects, workspaces);
  const none = projects && workspaces && startableProjects(projects, workspaces).length === 0;
  const header = (
    <div className="flex items-center justify-between gap-4 pt-2 pr-2 pl-5">
      <h2 id={titleId} className="text-[12.5px] font-medium text-ink-3">
        New session
      </h2>
      <IconButton label="Close" title="Close (Esc)" onClick={onClose}>
        <X className="size-4" />
      </IconButton>
    </div>
  );
  const box = "rounded-2xl border border-line-strong bg-surface shadow-[0_30px_120px_rgb(0_0_0/0.55)]";
  if (!me || !projects || !workspaces)
    return (
      <div className={box}>
        {header}
        <div className="px-5 pt-3 pb-5">
          <Skeleton className="h-[88px]" />
        </div>
      </div>
    );
  if (none)
    return (
      <div className={box}>
        {header}
        <p className="px-5 pt-1.5 pb-5 text-[14px] leading-relaxed text-ink-2">
          There's no project where you can start sessions yet.{" "}
          <Link to="/" onClick={onClose} className="text-ink underline underline-offset-2">
            The Agent Floor
          </Link>{" "}
          says how to set one up.
        </p>
      </div>
    );
  return (
    <TaskBox
      dialog
      header={header}
      user={me.user.id}
      projects={projects}
      workspaces={workspaces}
      here={here}
    />
  );
}

/**
 * The project the page behind is about, by slug: a project's page, a session's project, or a
 * workspace's first project where you start sessions, in your order.
 */
function useHere(projects?: G.Project[], workspaces?: G.Workspace[]): string | undefined {
  const path = useRouterState({ select: (s) => s.location.pathname });
  const client = useQueryClient();
  const [kind, id] = path.split("/").slice(1);
  if (!id) return undefined;
  if (kind === "p") return decodeURIComponent(id);
  if (kind === "s") return client.getQueryData<G.Session>(keys.session(id))?.project_slug;
  if (kind === "w" && projects && workspaces) {
    const w = workspaces.find((x) => x.slug === decodeURIComponent(id));
    const startable = startableProjects(projects, workspaces);
    return w && startable.find((p) => p.workspace?.id === w.id)?.slug;
  }
  return undefined;
}

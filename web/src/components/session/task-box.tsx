// The Agent Floor's task box: on the Agent Floor, and over any other page in the new session
// dialog (N, or ⌘⇧O, from anywhere: components/shell/new-session.tsx). Both are the same box,
// with the same draft, which stays in this browser until a session starts with it.

import { useEffect, useMemo, useRef, useState } from "react";
import { ScheduleButton } from "@/components/project/schedules";
import {
  AttachButton,
  DraftTray,
  DropHint,
  useAttachments,
  useFileDrop,
} from "@/components/session/attachments";
import {
  StartButton,
  StartChoices,
  useKeepDraft,
  useStartChoice,
  useStartSession,
} from "@/components/session/start";
import { Select, Textarea, TextareaControls } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { NoRunnerNote } from "@/components/workspace/no-runners";
import type * as G from "@/gen";
import { filesFromPaste } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { draftId, forgetDraft, loadText } from "@/lib/draft";
import { apple } from "@/lib/hotkeys";
import { asked } from "@/lib/models";
import { floorPlaceholder } from "@/lib/placeholders";
import { repoSummary } from "@/lib/repos";
import { useTaskBoxFocus } from "@/lib/task-box";
import { freshest } from "@/lib/workspaces";

/**
 * A project's repositories in a line. Where space is short the name gives way and the count
 * stays: the grid's narrowest is the count alone, its widest the whole line.
 */
function RepoText({ project }: { project: G.Project }) {
  const s = repoSummary(project);
  if (!s) return "No repository";
  return (
    <span className="grid grid-cols-[minmax(0,max-content)_auto] font-mono text-[11.5px]">
      <span className="truncate">{s.name}</span>
      <span className="whitespace-pre">{s.name && s.more ? ` ${s.more}` : s.more}</span>
    </span>
  );
}

/** The project a new session goes to: picked here last time, while it is still there. */
const LAST_TARGET = "fctl:last-target";

/** The projects where `projects`' person may start sessions, in their order (the list's). */
export function startableProjects(projects: G.Project[], workspaces: G.Workspace[]): G.Project[] {
  const blocked = new Set(workspaces.filter((w) => w.blocked).map((w) => w.id));
  return projects.filter((p) => p.role !== "viewer" && !(p.workspace && blocked.has(p.workspace.id)));
}

export function TaskBox({
  user,
  projects,
  workspaces,
  workspace,
  here,
  dialog,
  className,
}: {
  user: string;
  projects: G.Project[];
  workspaces: G.Workspace[];
  /** The workspace whose work the Agent Floor shows, by id, if only one's: the project follows it there. */
  workspace?: string;
  /** The project the page behind is about, by slug: a new draft starts in it. */
  here?: string;
  /**
   * In the new session dialog: it takes the focus as it opens, and Escape closes the dialog. On the
   * Agent Floor it takes the focus as the page opens, and Escape leaves it.
   */
  dialog?: boolean;
  className?: string;
}) {
  const index = useMemo(() => new Map(workspaces.map((w) => [w.id, w])), [workspaces]);
  // Projects where you may start sessions, under their workspaces, in your order (the list's).
  const editable = useMemo(() => startableProjects(projects, workspaces), [projects, workspaces]);
  // What was being written here before, kept in this browser until a session starts with it.
  const keep = draftId(user);
  const [kept] = useState(() => loadText(keep));
  const remembered = (() => {
    try {
      return localStorage.getItem(LAST_TARGET) ?? undefined;
    } catch {
      return undefined;
    }
  })();
  const startable = (slug?: string) => editable.find((p) => p.slug === slug)?.slug;
  // A draft's own project first: its task was written for it.
  const first = startable(kept?.project) ?? startable(here) ?? startable(remembered) ?? editable[0]?.slug;
  const [project, setProject] = useState(first ?? "");
  const [prompt, setPrompt] = useState(kept?.prompt ?? "");
  // A line picked as the page opens.
  const [placeholder] = useState(() => floorPlaceholder());
  const target = editable.find((p) => p.slug === project);
  const { choice, picked, pick: pickChoice } = useStartChoice(target, kept);
  const ref = useRef<HTMLTextAreaElement>(null);
  useTaskBoxFocus(ref, !dialog && editable.length > 0);
  useEffect(() => {
    if ((!project || !editable.some((p) => p.slug === project)) && first) setProject(first);
  }, [first, project, editable]);
  const pick = (slug: string) => {
    setProject(slug);
    try {
      localStorage.setItem(LAST_TARGET, slug);
    } catch {}
  };
  const files = useAttachments(editable.length && project ? { project } : undefined, { keep });
  const draft = useKeepDraft(keep, {
    kept,
    textarea: ref,
    text: { prompt, project, ...picked },
    files: files.drafts.length,
  });
  // Once a session starts with the draft, it is forgotten.
  const { start, starting } = useStartSession(draft.forget);
  // When the page shows one workspace's work, the project goes to its first in your order, even
  // from another of its projects. One picked here afterwards stays.
  // biome-ignore lint/correctness/useExhaustiveDependencies: only showing another workspace moves it
  useEffect(() => {
    if (!workspace || starting) return;
    const next = editable.find((p) => p.workspace?.id === workspace);
    if (next) pick(next.slug);
  }, [workspace]);
  const attach = (f: File[], pasted?: boolean) => {
    files.add(f, { pasted });
    ref.current?.focus();
  };
  const dragging = useFileDrop(attach, editable.length > 0 && !starting);
  if (!editable.length) return null;
  const go = () => start(project, prompt, files, target && asked(target, choice));
  const several = workspaces.length > 1;
  return (
    <>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          go();
        }}
        className={cn(
          "group relative rounded-2xl border border-line-strong transition-[border-color,box-shadow] duration-200",
          dialog
            ? "bg-surface shadow-[0_30px_120px_rgb(0_0_0/0.55)]"
            : "bg-surface/85 shadow-(--composer-shadow) backdrop-blur-2xl focus-within:shadow-(--composer-shadow-focus)",
          dragging ? "border-accent/50" : "focus-within:border-ink-4/60",
          className,
        )}
      >
        <DraftTray
          drafts={files.drafts}
          onRemove={files.remove}
          onRetry={files.retry}
          readOnly={starting}
          className="px-4 pt-4"
        />
        <Textarea
          ref={ref}
          bare
          minRows={2}
          maxRows={8}
          value={prompt}
          readOnly={starting}
          autoFocus={dialog}
          onChange={(e) => setPrompt(e.target.value)}
          onBlur={draft.save}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              go();
            } else if (e.key === "Escape" && !dialog && !e.nativeEvent.isComposing) {
              // Out of the box, the page's keys work again.
              e.currentTarget.blur();
            }
          }}
          onPaste={(e) => {
            const pasted = filesFromPaste(e.clipboardData);
            if (!pasted.length || starting) return;
            e.preventDefault();
            attach(pasted, true);
          }}
          placeholder={placeholder}
          className="block px-5 pt-4 font-display text-[18px] leading-[1.5] text-ink placeholder:text-ink-4"
          aria-label="Task for a new session"
        />
        {/* On a phone it keeps one line: where the controls run out of room, the project's name and
            the model's give way, and their starts show. */}
        <TextareaControls textarea={ref} className="flex-wrap gap-2 pt-4 pr-3 pb-3 pl-2.5 max-sm:flex-nowrap">
          <AttachButton onFiles={(f) => attach(f)} disabled={starting} />
          {/* It takes the room left on its line, which puts what follows at the end. On a phone it
              keeps room for the start of the project's name, and gives up some of it with the model's
              name where the row has too little. Wider, it starts from nothing, so that it never
              pushes Start onto a line of its own (as its "Starting" would, being wider): the picker,
              as wide as the longest name it offers, gives up room it doesn't need first. */}
          <div className="flex min-w-0 grow basis-0 items-center gap-2 text-[12.5px] max-sm:basis-24">
            <span className="hidden text-ink-4 sm:inline">in</span>
            <Select
              size="sm"
              // On a phone it shows the project's name, after its workspace's avatar, as wide as it
              // is, so that the model, the effort level and Start fit beside it.
              narrow="fit"
              aria-label="Workspace and project"
              value={project}
              disabled={starting}
              onChange={pick}
              options={editable.map((p) => {
                const w = p.workspace ? freshest(index, p.workspace) : undefined;
                return {
                  value: p.slug,
                  label: several && w ? `${w.name} / ${p.name}` : p.name,
                  short: p.name,
                  item: p.name,
                  icon: w ? <WorkspaceAvatar workspace={w} size={14} title={false} /> : undefined,
                  description: <RepoText project={p} />,
                  section:
                    several && w
                      ? {
                          key: w.id,
                          label: (
                            <>
                              <WorkspaceAvatar workspace={w} size={14} title={false} />
                              {w.name}
                            </>
                          ),
                        }
                      : undefined,
                };
              })}
            />
          </div>
          {target && (
            <StartChoices project={target} choice={choice} onPick={pickChoice} issues disabled={starting} />
          )}
          {/* From lg the pickers share its row, which has room for it again from xl. Its margins set it
              apart from them, and a little from the button. */}
          <span className="mr-1 ml-5 hidden text-[11.5px] text-ink-4 sm:inline lg:hidden xl:inline">
            {startKeys()}
          </span>
          {target && (
            <ScheduleButton
              project={target}
              prompt={prompt}
              choice={choice}
              files={files.drafts.length}
              disabled={starting}
              // Its symbol alone, as the row has the project's picker too, and only from xl, where the
              // row is as wide as it gets: narrower, it would push Start onto a line of its own, or
              // the row onto another. The project's own task box has it at every width.
              bare
              className="max-xl:hidden"
              // The draft stays here until a schedule is set up with it.
              onSaved={() => void forgetDraft(keep)}
            />
          )}
          <StartButton
            starting={starting}
            disabled={!prompt.trim() && !files.drafts.length}
            className="enabled:hover:scale-[1.03]"
          >
            Start
          </StartButton>
        </TextareaControls>
        <DropHint show={dragging} />
      </form>
      <NoRunnerNote workspace={target?.workspace && index.get(target.workspace.id)} className="mt-3" />
    </>
  );
}

/** What starts the session from a task box, as its hint says it: ⌘+Enter, or Ctrl+Enter. */
export const startKeys = () => (apple() ? "⌘+Enter" : "Ctrl+Enter");

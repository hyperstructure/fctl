// A project's memory: the notes its agents, and its people, keep for the
// agents of later sessions. Everyone in the project reads them; editors
// add notes, change them, mark them stale and delete them.

import { useMutation } from "@tanstack/react-query";
import { Link, useLocation } from "@tanstack/react-router";
import {
  Archive,
  ArchiveRestore,
  ArrowRight,
  BookMarked,
  Copy,
  Ellipsis,
  Pencil,
  Plus,
  Search,
  Trash2,
  X,
} from "lucide-react";
import { useEffect, useMemo, useRef, useState } from "react";
import { Markdown } from "@/components/markdown";
import {
  Avatar,
  Banner,
  Button,
  Dialog,
  Empty,
  Field,
  IconButton,
  Input,
  Menu,
  MenuItem,
  MenuSeparator,
  Skeleton,
  Textarea,
  TextareaControls,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago, stamp } from "@/lib/format";
import {
  canEditMemory,
  headline,
  MAX_NOTE_CHARS,
  matches,
  noteAnchor,
  original,
  replacement,
} from "@/lib/memory";
import { keys, queryClient, useMemories } from "@/lib/queries";

/** Puts a changed note in the cached list, or takes a deleted one out. */
function saved(project: string, note: G.Memory, removed?: boolean) {
  queryClient.setQueryData<G.MemoryList>(keys.memories(project), (old) => {
    if (!old) return old;
    const others = old.memories.filter((m) => m.id !== note.id);
    return { memories: removed ? others : [note, ...others] };
  });
  void queryClient.invalidateQueries({ queryKey: keys.memories(project) });
}

export function MemoryTab({ project }: { project: G.Project }) {
  const memories = useMemories(project.id);
  const edit = canEditMemory(project.role);
  const manage = project.role === "owner" || project.role === "maintainer";
  const [view, setView] = useState<"current" | "stale">("current");
  const [query, setQuery] = useState("");
  const all = memories.data?.memories ?? [];
  const current = all.filter((m) => !m.stale);
  const stale = all.filter((m) => m.stale);
  const shown = (view === "current" ? current : stale).filter((m) => matches(m, query));
  const flash = useFlash(all, setView);

  return (
    <div className="flex flex-col gap-6">
      <p className="max-w-2xl text-[13.5px] leading-relaxed text-ink-3">
        Notes this project's agents keep for the agents after them: what they learned that the repository
        doesn't say. Each session's agent starts with the current notes, adds what a future agent should know,
        and marks the ones that turned out wrong stale.
        {edit ? " Change or delete the ones you don't want agents to rely on." : ""}
      </p>
      {!project.settings.project_memory && (
        <Banner tone="info">
          This project's agents keep no memory: they don't start with these notes or add any.{" "}
          {manage ? (
            <>
              Turn it on under{" "}
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
            "Its maintainers can turn it on."
          )}
        </Banner>
      )}
      {edit && <AddNote project={project} />}
      {memories.isPending ? (
        <div className="space-y-3 py-2">
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
        </div>
      ) : all.length === 0 ? (
        <Empty title="No notes yet" icon={<BookMarked className="size-7" strokeWidth={1.5} />}>
          {project.settings.project_memory
            ? "As this project's agents learn what its repository doesn't say, they note it here for the agents after them."
            : "Notes people add here wait for the project's agents to keep a memory."}
          {edit && " You can add one too."}
        </Empty>
      ) : (
        <section>
          <div className="mb-1 flex flex-wrap items-center gap-x-3 gap-y-2">
            <div className="flex items-center gap-1">
              {(
                [
                  ["current", "Current", current.length],
                  ["stale", "Stale", stale.length],
                ] as const
              ).map(([v, label, n]) => (
                <button
                  key={v}
                  type="button"
                  aria-pressed={view === v}
                  onClick={() => setView(v)}
                  className={cn(
                    "inline-flex items-center gap-1.5 rounded-full px-3 py-1 text-[12.5px] transition-colors",
                    view === v
                      ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                      : "text-ink-3 hover:text-ink",
                  )}
                >
                  {label}
                  <span className="text-ink-4 tabular-nums">{n}</span>
                </button>
              ))}
            </div>
            <span className="flex-1" />
            <div className="relative flex min-w-0 items-center max-sm:w-full sm:w-64">
              <Search className="pointer-events-none absolute left-2.5 size-3.5 text-ink-4" />
              <Input
                value={query}
                onChange={(e) => setQuery(e.target.value)}
                placeholder="Find a note"
                aria-label="Find a note"
                className="h-8 w-full pr-7 pl-8 text-[13px]"
              />
              {query && (
                <button
                  type="button"
                  aria-label="Clear"
                  onClick={() => setQuery("")}
                  className="absolute right-1.5 rounded p-1 text-ink-4 hover:text-ink"
                >
                  <X className="size-3.5" />
                </button>
              )}
            </div>
          </div>
          {shown.length === 0 ? (
            <p className="border-t border-line py-6 text-[13px] text-ink-4">
              {query
                ? "No note has those words."
                : view === "stale"
                  ? "No stale notes. Agents mark a note stale when it turns out wrong or out of date."
                  : "No current notes: every note is stale."}
            </p>
          ) : (
            <ul className="border-t border-line">
              {shown.map((m) => (
                <Note
                  key={m.id}
                  note={m}
                  project={project}
                  all={all}
                  edit={edit}
                  flashing={flash.id === m.id}
                  onShow={(id) => flash.show(id)}
                />
              ))}
            </ul>
          )}
        </section>
      )}
    </div>
  );
}

/**
 * The note a link points to (`#note-mem_…`), in view and lit for a moment:
 * the transcript's links, and a stale note's link to the one that took its
 * place.
 */
function useFlash(all: G.Memory[], setView: (v: "current" | "stale") => void) {
  const hash = useLocation({ select: (l) => l.hash });
  const [id, setId] = useState<string>();
  const shown = useRef<string>(undefined);
  const show = (target: string) => {
    const note = all.find((m) => m.id === target);
    if (!note) return;
    setView(note.stale ? "stale" : "current");
    setId(target);
    requestAnimationFrame(() => {
      const reduce = window.matchMedia("(prefers-reduced-motion: reduce)").matches;
      document
        .getElementById(noteAnchor(target))
        ?.scrollIntoView({ block: "center", behavior: reduce ? "auto" : "smooth" });
    });
  };
  // biome-ignore lint/correctness/useExhaustiveDependencies: once the notes and the link are both there
  useEffect(() => {
    const target = hash.replace(/^#/, "").replace(/^note-/, "");
    if (!target || shown.current === target || !all.some((m) => m.id === target)) return;
    shown.current = target;
    show(target);
  }, [hash, all]);
  useEffect(() => {
    if (!id) return;
    const t = setTimeout(() => setId(undefined), 2200);
    return () => clearTimeout(t);
  }, [id]);
  return { id, show };
}

function AddNote({ project }: { project: G.Project }) {
  const [text, setText] = useState("");
  const ref = useRef<HTMLTextAreaElement>(null);
  const add = useMutation({
    mutationFn: () => api.addMemory(project.id, { text }),
    onSuccess: (note) => {
      saved(project.id, note);
      setText("");
      toast(
        project.settings.project_memory ? "Added: agents start with it" : "Added to the project's memory",
      );
    },
  });
  const left = MAX_NOTE_CHARS - [...text].length;
  const go = () => {
    if (text.trim() && left >= 0 && !add.isPending) add.mutate();
  };
  return (
    <div>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          go();
        }}
        className="rounded-xl border border-line-strong bg-surface/80 transition-[border-color,box-shadow] focus-within:border-accent/45 focus-within:shadow-[0_0_0_4px_var(--c-accent-soft)]"
      >
        <Textarea
          ref={ref}
          bare
          minRows={1}
          maxRows={10}
          value={text}
          onChange={(e) => setText(e.target.value)}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              go();
            }
          }}
          placeholder="Add a note for this project's agents: a command that works, a trap to avoid, a decision…"
          aria-label="A note for the project's agents"
          className="block px-4 pt-3 text-[14px] leading-relaxed placeholder:text-ink-4"
        />
        <TextareaControls textarea={ref} className="gap-3 px-3 pt-1 pb-2.5">
          <span className={cn("text-[12px]", left < 0 ? "text-danger" : "text-ink-4")}>
            {left < 200 ? `${left} characters left` : "Markdown · one fact per note"}
          </span>
          <span className="flex-1" />
          <span className="text-[11.5px] text-ink-4 max-sm:hidden">⌘↵</span>
          <Button
            type="submit"
            size="sm"
            variant="primary"
            icon={<Plus className="size-3.5" />}
            loading={add.isPending}
            disabled={!text.trim() || left < 0}
          >
            Add note
          </Button>
        </TextareaControls>
      </form>
      {add.error && (
        <p className="mt-2 animate-rise text-[12.5px] text-danger" role="alert">
          {add.error.message}
        </p>
      )}
    </div>
  );
}

function Note({
  note,
  project,
  all,
  edit,
  flashing,
  onShow,
}: {
  note: G.Memory;
  project: G.Project;
  all: G.Memory[];
  edit: boolean;
  flashing: boolean;
  onShow: (id: string) => void;
}) {
  const [editing, setEditing] = useState(false);
  const [dialog, setDialog] = useState<"stale" | "delete">();
  const restore = useMutation({
    mutationFn: () => api.updateMemory(note.id, { stale: false }),
    onSuccess: (m) => {
      saved(project.id, m);
      toast("Current again: agents start with it");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const next = replacement(all, note);
  // The note it repeats, unless a newer one took its place since.
  const first = next ? undefined : original(all, note);
  const author = note.created_by;
  return (
    <li id={noteAnchor(note.id)} className="group/note scroll-mt-24 border-b border-line py-1">
      {/* Lit for a moment when a link points to it. */}
      <div
        className={cn(
          "-mx-3 flex items-start gap-3 rounded-lg px-3 py-3 transition-colors duration-700",
          flashing && "bg-accent-soft",
        )}
      >
        <div className="min-w-0 flex-1">
          {editing ? (
            <EditNote note={note} project={project} onDone={() => setEditing(false)} />
          ) : (
            // The prose takes its size and color from around it.
            <div className={cn("text-[14px]", note.stale ? "text-ink-3" : "text-ink")}>
              <Markdown text={note.text} className="prose-note" />
            </div>
          )}
          <div className="mt-2 flex flex-wrap items-center gap-x-1.5 gap-y-1 text-[12px] text-ink-4">
            {author && <Avatar user={author} size={16} />}
            {note.session ? (
              <span className="min-w-0">
                {author ? `${author.name}'s agent` : "An agent"} in <SessionLink session={note.session} />
              </span>
            ) : (
              <span>{author?.name ?? "Someone who left"}</span>
            )}
            <span aria-hidden="true">·</span>
            <time dateTime={new Date(note.created_at).toISOString()} title={stamp(note.created_at)}>
              {ago(note.created_at)}
            </time>
            {note.edited_at && (
              <span title={stamp(note.edited_at)}>
                · edited{note.edited_by ? ` by ${note.edited_by.name}` : ""} {ago(note.edited_at)}
              </span>
            )}
          </div>
          {note.stale && (
            <div className="mt-2 flex items-start gap-2 rounded-md bg-sunken/60 px-2.5 py-1.5 text-[12.5px] leading-relaxed text-ink-3">
              <Archive className="mt-[3px] size-3.5 shrink-0 text-ink-4" />
              <span className="min-w-0 [overflow-wrap:anywhere]">
                Stale since {ago(note.stale.at)}
                {note.stale.session ? (
                  <>
                    , marked by the agent in <SessionLink session={note.stale.session} />
                  </>
                ) : note.stale.by ? (
                  `, marked by ${note.stale.by.name}`
                ) : null}
                {note.stale.reason && !next && !first ? `: ${note.stale.reason}` : ""}
                {next && (
                  <>
                    {": "}
                    <button
                      type="button"
                      onClick={() => onShow(next.id)}
                      className="text-ink-2 underline underline-offset-2 hover:text-ink"
                    >
                      a newer note takes its place
                    </button>
                  </>
                )}
                {first && (
                  <>
                    {": "}
                    <button
                      type="button"
                      onClick={() => onShow(first.id)}
                      className="text-ink-2 underline underline-offset-2 hover:text-ink"
                    >
                      a duplicate of another note
                    </button>
                  </>
                )}
              </span>
            </div>
          )}
        </div>
        {edit && !editing && (
          <Menu
            trigger={(toggle, open) => (
              <IconButton
                label="Note actions"
                aria-haspopup="menu"
                aria-expanded={open}
                onClick={toggle}
                className={cn(
                  "-mt-1 -mr-1 opacity-0 transition-opacity group-hover/note:opacity-100 focus-visible:opacity-100 pointer-coarse:opacity-100",
                  open && "opacity-100",
                )}
              >
                <Ellipsis className="size-4" />
              </IconButton>
            )}
          >
            {(close) => (
              <>
                <MenuItem
                  icon={<Pencil />}
                  onClick={() => {
                    close();
                    setEditing(true);
                  }}
                >
                  Edit
                </MenuItem>
                {note.stale ? (
                  <MenuItem
                    icon={<ArchiveRestore />}
                    onClick={() => {
                      close();
                      restore.mutate();
                    }}
                  >
                    Make current again
                  </MenuItem>
                ) : (
                  <MenuItem
                    icon={<Archive />}
                    onClick={() => {
                      close();
                      setDialog("stale");
                    }}
                  >
                    Mark stale…
                  </MenuItem>
                )}
                <MenuItem
                  icon={<Copy />}
                  onClick={() => {
                    close();
                    void navigator.clipboard.writeText(note.text);
                    toast("Copied the note");
                  }}
                >
                  Copy text
                </MenuItem>
                <MenuSeparator />
                <MenuItem
                  danger
                  icon={<Trash2 />}
                  onClick={() => {
                    close();
                    setDialog("delete");
                  }}
                >
                  Delete…
                </MenuItem>
              </>
            )}
          </Menu>
        )}
      </div>
      {/* Only while open: a memory can hold hundreds of notes. */}
      {dialog === "stale" && (
        <MarkStale note={note} project={project} open onClose={() => setDialog(undefined)} />
      )}
      {dialog === "delete" && (
        <DeleteNote note={note} project={project} open onClose={() => setDialog(undefined)} />
      )}
    </li>
  );
}

function SessionLink({ session }: { session: G.SessionRef }) {
  return (
    <Link
      to="/s/$id"
      params={{ id: session.id }}
      className="text-ink-3 transition-colors hover:text-ink"
      title={session.title}
    >
      <span className="font-mono">#{session.number}</span> {headline(session.title, 60)}
    </Link>
  );
}

function EditNote({ note, project, onDone }: { note: G.Memory; project: G.Project; onDone: () => void }) {
  const [text, setText] = useState(note.text);
  const save = useMutation({
    mutationFn: () => api.updateMemory(note.id, { text }),
    onSuccess: (m) => {
      saved(project.id, m);
      onDone();
    },
  });
  const left = MAX_NOTE_CHARS - [...text].length;
  const changed = text.trim() !== note.text.trim();
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (changed && text.trim()) save.mutate();
        else onDone();
      }}
      className="flex flex-col gap-2"
    >
      <Textarea
        value={text}
        onChange={(e) => setText(e.target.value)}
        onKeyDown={(e) => {
          if (e.key === "Escape") onDone();
          if (e.key === "Enter" && (e.metaKey || e.ctrlKey) && changed && text.trim()) {
            e.preventDefault();
            save.mutate();
          }
        }}
        minRows={2}
        maxRows={12}
        aria-label="The note"
        autoFocus
        className="text-[14px]"
      />
      {save.error && (
        <p className="animate-rise text-[12.5px] text-danger" role="alert">
          {save.error.message}
        </p>
      )}
      <div className="flex items-center justify-end gap-2">
        {left < 200 && (
          <span className={cn("mr-auto text-[12px]", left < 0 ? "text-danger" : "text-ink-4")}>
            {left} characters left
          </span>
        )}
        <Button type="button" size="sm" variant="ghost" onClick={onDone}>
          Cancel
        </Button>
        <Button
          type="submit"
          size="sm"
          variant="primary"
          loading={save.isPending}
          disabled={!text.trim() || left < 0}
        >
          Save
        </Button>
      </div>
    </form>
  );
}

function MarkStale({
  note,
  project,
  open,
  onClose,
}: {
  note: G.Memory;
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const [reason, setReason] = useState("");
  useEffect(() => {
    if (open) setReason("");
  }, [open]);
  const mark = useMutation({
    mutationFn: () => api.updateMemory(note.id, { stale: true, reason: reason.trim() || undefined }),
    onSuccess: (m) => {
      saved(project.id, m);
      toast("Marked stale: agents no longer start with it");
      onClose();
    },
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Mark this note stale?"
      description="Agents no longer start with it. It stays under Stale, where you can make it current again."
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          mark.mutate();
        }}
      >
        <blockquote className="border-l-2 border-line-strong pl-3 text-[13.5px] leading-relaxed text-ink-2 [overflow-wrap:anywhere]">
          {headline(note.text, 200)}
        </blockquote>
        <Field label="Why" hint="Optional. It shows with the note." error={mark.error?.message}>
          {(id) => (
            <Input
              id={id}
              value={reason}
              onChange={(e) => setReason(e.target.value)}
              placeholder="The script moved to xtask"
              autoFocus
            />
          )}
        </Field>
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            icon={<Archive className="size-3.5" />}
            loading={mark.isPending}
          >
            Mark stale
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

function DeleteNote({
  note,
  project,
  open,
  onClose,
}: {
  note: G.Memory;
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const del = useMutation({
    mutationFn: () => api.deleteMemory(note.id),
    onSuccess: () => {
      saved(project.id, note, true);
      toast("Deleted the note");
      onClose();
    },
  });
  const teaser = useMemo(() => headline(note.text, 200), [note.text]);
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Delete this note?"
      description={[
        note.stale
          ? "It goes from the project's memory for good."
          : "Agents no longer start with it, and it goes from the project's memory for good. To keep it for the record, mark it stale instead.",
        note.session ? `#${note.session.number}'s transcript keeps what its agent wrote.` : "",
      ]
        .filter(Boolean)
        .join(" ")}
    >
      <div className="flex flex-col gap-5">
        <blockquote className="border-l-2 border-line-strong pl-3 text-[13.5px] leading-relaxed text-ink-2 [overflow-wrap:anywhere]">
          {teaser}
        </blockquote>
        {del.error && <Banner tone="error">{del.error.message}</Banner>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            variant="danger"
            icon={<Trash2 className="size-3.5" />}
            loading={del.isPending}
            onClick={() => del.mutate()}
          >
            Delete
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

/** A link to a project's Memory tab, which scrolls to the note `id` names. */
export function NoteLink({
  slug,
  id,
  children,
  className,
  arrow = true,
}: {
  slug: string;
  id?: string;
  children: React.ReactNode;
  className?: string;
  /** An arrow after it, for a link that stands alone. */
  arrow?: boolean;
}) {
  return (
    <Link
      to="/p/$slug"
      params={{ slug }}
      search={{ tab: "memory" }}
      hash={id ? noteAnchor(id) : undefined}
      className={cn("transition-colors hover:text-ink", arrow && "inline-flex items-center gap-1", className)}
    >
      {children}
      {arrow && <ArrowRight className="size-3" />}
    </Link>
  );
}

/** A line of a note, with its `code` in the code font: what lists show of a note. */
export function NoteLine({ text }: { text: string }) {
  return (
    <>
      {text.split("`").map((part, i) =>
        i % 2 ? (
          <code key={i} className="font-mono text-[0.9em]">
            {part}
          </code>
        ) : (
          part
        ),
      )}
    </>
  );
}

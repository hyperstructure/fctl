// A changed file's diff over the session page: opened from the files its
// Changes list, it shows one file's diff, highlighted, with the session's
// other changed files beside it to step through. It follows the agent:
// when the file changes again, its diff does too.

import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowUpRight,
  Binary,
  ChevronDown,
  ChevronUp,
  Columns2,
  FileQuestion,
  FileX2,
  History,
  PanelLeft,
  Rows3,
  Search,
  TextWrap,
  X,
} from "lucide-react";
import {
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { type DiffLayout, DiffLines } from "@/components/diff";
import { Button, CodeBox, CopyButton, IconButton, Kbd, Skeleton } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure } from "@/lib/api";
import { cn } from "@/lib/cn";
import { firstLine, parsePatch } from "@/lib/diff";
import { useHoldPage } from "@/lib/hold-page";
import { languageOf } from "@/lib/languages";
import { fileDiffQuery, useFileDiff } from "@/lib/queries";

/** A repository's changed files, as the dialog lists them. */
export interface DiffGroup {
  /** `owner/name`; absent for the primary repository. */
  repo?: string;
  /** What the list calls it, when the session has several. */
  title?: string;
  files: G.FileChange[];
  /** Where a file is on GitHub on the session's branch, while the branch is pushed. */
  blob?: (path: string) => string;
}

/** Which file: its repository and path. */
export interface DiffTarget {
  repo?: string;
  path: string;
}

const same = (a: DiffTarget, b: DiffTarget) => a.path === b.path && (a.repo ?? "") === (b.repo ?? "");

/** How a file changed, as its letter in lists and its word in the dialog's header. */
export function changeOf(status: string): { letter: string; label: string; tone: string } {
  switch (status) {
    case "added":
      return { letter: "A", label: "Added", tone: "text-accent" };
    case "untracked":
      return { letter: "A", label: "New, untracked", tone: "text-accent" };
    case "deleted":
      return { letter: "D", label: "Deleted", tone: "text-danger" };
    case "renamed":
      return { letter: "R", label: "Renamed", tone: "text-blue" };
    default:
      return { letter: "M", label: "Modified", tone: "text-blue" };
  }
}

/** A path's directory, with its slash, and its name. */
const split = (path: string) => {
  const cut = path.lastIndexOf("/") + 1;
  return { dir: path.slice(0, cut), name: path.slice(cut) };
};

/** A preference this browser keeps, such as how diffs are laid out. */
function useKept<T extends string>(key: string, initial: T, allowed: readonly T[]): [T, (v: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const v = localStorage.getItem(key) as T | null;
      return v && allowed.includes(v) ? v : initial;
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (v: T) => {
      setValue(v);
      try {
        localStorage.setItem(key, v);
      } catch {}
    },
    [key],
  );
  return [value, set];
}

/** Whether the window is at least this wide (px). */
function useWide(px: number) {
  const query = `(min-width: ${px}px)`;
  const [wide, setWide] = useState(() => window.matchMedia?.(query).matches ?? true);
  useEffect(() => {
    const m = window.matchMedia?.(query);
    if (!m) return;
    const on = () => setWide(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, [query]);
  return wide;
}

/**
 * A changed file's diff over the page, on the native modal <dialog>: Escape,
 * the X or a click beside it closes it, and focus goes back to what opened
 * it. `j` and `k` (or the arrows in its header) step through the session's
 * changed files, which the list at its side shows; `s` lays the diff out
 * side by side, `w` wraps long lines.
 */
export function DiffDialog({
  session,
  cliRef,
  groups,
  current,
  onSelect,
  onClose,
}: {
  session: string;
  /** How `fctl` names the session (`project#12`), for the command that prints a diff too large to show. */
  cliRef: string;
  groups: DiffGroup[];
  current: DiffTarget;
  onSelect: (t: DiffTarget) => void;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [layout, setLayout] = useKept<DiffLayout>("fctl.diff.layout", "unified", ["unified", "split"]);
  const [wrap, setWrap] = useKept<"wrap" | "scroll">("fctl.diff.wrap", "scroll", ["wrap", "scroll"]);
  const [listShown, setListShown] = useKept<"shown" | "hidden">("fctl.diff.list", "shown", [
    "shown",
    "hidden",
  ]);
  // On a phone the list covers the diff while it is open.
  const [listOpen, setListOpen] = useState(false);
  const wide = useWide(1024);
  // Side by side takes room: below it, lines go one below the other.
  const roomy = useWide(900);
  const pressedOutside = useRef(false);

  const all = useMemo(
    () => groups.flatMap((g) => g.files.map((file) => ({ repo: g.repo, file, group: g }))),
    [groups],
  );
  const index = all.findIndex((f) => same({ repo: f.repo, path: f.file.path }, current));
  const entry = all[index];

  useEffect(() => {
    ref.current?.setAttribute("autofocus", "");
    ref.current?.showModal();
    ref.current?.focus();
  }, []);
  useHoldPage();

  const go = useCallback(
    (by: -1 | 1) => {
      const next = all[index + by];
      if (next) onSelect({ repo: next.repo, path: next.file.path });
    },
    [all, index, onSelect],
  );

  // Another file starts at its top.
  // biome-ignore lint/correctness/useExhaustiveDependencies: when the file shown changes
  useLayoutEffect(() => {
    body.current?.scrollTo({ top: 0, left: 0 });
  }, [current.path, current.repo]);

  // The files beside this one load ahead, so that stepping to them is instant.
  const qc = useQueryClient();
  useEffect(() => {
    for (const n of [all[index + 1], all[index - 1]]) {
      if (n?.file.digest) void qc.prefetchQuery(fileDiffQuery(session, { repo: n.repo, file: n.file }));
    }
  }, [all, index, qc, session]);

  const onKeyDown = (e: React.KeyboardEvent) => {
    const t = e.target as HTMLElement;
    if (e.metaKey || e.ctrlKey || e.altKey || t.tagName === "INPUT" || t.tagName === "TEXTAREA") return;
    const key = e.key.toLowerCase();
    if (key === "j" || key === "k") go(key === "j" ? 1 : -1);
    else if (key === "s") setLayout(layout === "split" ? "unified" : "split");
    else if (key === "w") setWrap(wrap === "wrap" ? "scroll" : "wrap");
    else if (key === "/") document.getElementById(`${titleId}-filter`)?.focus();
    else return;
    e.preventDefault();
  };

  const list = (
    <FileList
      groups={groups}
      current={current}
      filterId={`${titleId}-filter`}
      onSelect={(t) => {
        onSelect(t);
        setListOpen(false);
      }}
    />
  );
  const listVisible = wide ? listShown === "shown" : listOpen;

  return createPortal(
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onClose={onClose}
      onKeyDown={onKeyDown}
      onMouseDown={(e) => {
        pressedOutside.current = e.target === ref.current;
      }}
      onClick={(e) => {
        if (e.target === ref.current && pressedOutside.current) ref.current?.close();
      }}
      className={cn(
        "top-(--viewport-shift) -bottom-(--viewport-shift) m-auto overflow-hidden bg-surface p-0 text-ink outline-none",
        "h-dvh max-h-none w-screen max-w-none sm:h-[calc(100dvh-2.5rem)] sm:w-[min(1440px,calc(100vw-2.5rem))] sm:rounded-xl sm:border sm:border-line-strong",
        "shadow-[0_30px_120px_rgb(0_0_0/0.55)] backdrop:bg-black/55 backdrop:backdrop-blur-[3px] open:animate-rise",
      )}
    >
      <div className="flex h-full flex-col">
        <Header
          entry={entry}
          current={current}
          titleId={titleId}
          index={index}
          count={all.length}
          go={go}
          layout={layout}
          setLayout={setLayout}
          sideBySide={roomy}
          wrap={wrap === "wrap"}
          toggleWrap={() => setWrap(wrap === "wrap" ? "scroll" : "wrap")}
          listVisible={listVisible}
          toggleList={() =>
            wide ? setListShown(listShown === "shown" ? "hidden" : "shown") : setListOpen(!listOpen)
          }
          close={() => ref.current?.close()}
          session={session}
        />
        <div className="relative flex min-h-0 flex-1">
          {listVisible && (
            <aside
              aria-label="Changed files"
              className={cn(
                "flex min-h-0 flex-col border-line bg-surface",
                wide
                  ? "w-[288px] shrink-0 border-r"
                  : "absolute inset-0 z-10 animate-fade border-b sm:right-auto sm:w-[340px] sm:border-r",
              )}
            >
              {list}
            </aside>
          )}
          <div
            ref={body}
            // The diff scrolls; a key moves it once it has focus.
            // biome-ignore lint/a11y/noNoninteractiveTabindex: a scrolling region takes focus to scroll
            tabIndex={0}
            className="min-w-0 flex-1 overflow-auto bg-canvas/40 outline-none"
          >
            <Body
              key={`${current.repo ?? ""}\u0000${current.path}`}
              session={session}
              cliRef={cliRef}
              entry={entry}
              current={current}
              layout={roomy ? layout : "unified"}
              wrap={wrap === "wrap"}
            />
          </div>
        </div>
      </div>
    </dialog>,
    document.body,
  );
}

type Entry = { repo?: string; file: G.FileChange; group: DiffGroup };

/** The file's name and how it changed, and what moves between files and lays the diff out. */
function Header({
  entry,
  current,
  titleId,
  index,
  count,
  go,
  layout,
  setLayout,
  sideBySide,
  wrap,
  toggleWrap,
  listVisible,
  toggleList,
  close,
  session,
}: {
  entry?: Entry;
  current: DiffTarget;
  titleId: string;
  index: number;
  count: number;
  go: (by: -1 | 1) => void;
  layout: DiffLayout;
  setLayout: (l: DiffLayout) => void;
  /** The window is wide enough to show the diff side by side. */
  sideBySide: boolean;
  wrap: boolean;
  toggleWrap: () => void;
  listVisible: boolean;
  toggleList: () => void;
  close: () => void;
  session: string;
}) {
  const { dir, name } = split(current.path);
  const change = changeOf(entry?.file.status ?? "modified");
  const a = entry?.file.additions;
  const d = entry?.file.deletions;
  const blob = entry && entry.file.status !== "deleted" ? entry.group.blob?.(entry.file.path) : undefined;
  const lang = languageOf(current.path);
  return (
    <header className="flex shrink-0 items-center gap-2 border-b border-line py-2.5 pr-2.5 pl-2.5 sm:gap-3 sm:pl-3">
      <IconButton
        label={listVisible ? "Hide the changed files" : "Show the changed files"}
        aria-expanded={listVisible}
        onClick={toggleList}
        className={cn(listVisible && "text-ink")}
      >
        <PanelLeft className="size-4" />
      </IconButton>
      <div className="flex min-w-0 flex-1 flex-col gap-0.5">
        <div className="flex min-w-0 items-center gap-2">
          <h2
            id={titleId}
            className="flex min-w-0 font-mono text-[13.5px] leading-tight"
            title={current.path}
          >
            {dir && <span className="min-w-0 shrink-[1000] truncate text-ink-4">{dir}</span>}
            <span className="min-w-0 truncate font-medium text-ink">{name}</span>
          </h2>
          <CopyButton
            text={current.path}
            label="Copy path"
            className="hidden shrink-0 py-0.5 sm:inline-flex"
          />
        </div>
        <div className="flex min-w-0 items-center gap-x-2.5 overflow-hidden text-[12px] whitespace-nowrap text-ink-3">
          <span className={cn("font-medium", change.tone)}>{change.label}</span>
          {(a !== undefined || d !== undefined) && (
            <span className="font-mono text-[11.5px] tabular-nums">
              <span className={cn(a && "text-accent")}>+{a ?? 0}</span>{" "}
              <span className={cn(d && "text-danger")}>−{d ?? 0}</span>
            </span>
          )}
          {lang.id !== "text" && <span className="hidden text-ink-4 sm:inline">{lang.label}</span>}
          {entry?.repo && <span className="truncate font-mono text-[11.5px] text-ink-4">{entry.repo}</span>}
          <Updated session={session} entry={entry} />
        </div>
      </div>
      <nav aria-label="Changed files" className="flex shrink-0 items-center">
        <IconButton label="Previous file (k)" onClick={() => go(-1)} disabled={index <= 0}>
          <ChevronUp className="size-4" />
        </IconButton>
        <span
          className="min-w-[4.5ch] text-center font-mono text-[11.5px] text-ink-3 tabular-nums"
          aria-live="polite"
        >
          {index >= 0 ? index + 1 : "–"}
          <span className="text-ink-4">/{count}</span>
        </span>
        <IconButton label="Next file (j)" onClick={() => go(1)} disabled={index < 0 || index >= count - 1}>
          <ChevronDown className="size-4" />
        </IconButton>
      </nav>
      <div className="hidden h-5 w-px bg-line sm:block" />
      {sideBySide && (
        <div className="flex shrink-0 rounded-md border border-line p-0.5">
          {(
            [
              ["unified", "One below the other", <Rows3 key="u" className="size-3.5" />],
              ["split", "Side by side (s)", <Columns2 key="s" className="size-3.5" />],
            ] as const
          ).map(([value, label, icon]) => (
            <button
              key={value}
              type="button"
              aria-pressed={layout === value}
              aria-label={label}
              title={label}
              onClick={() => setLayout(value)}
              className={cn(
                "inline-flex h-6.5 w-7.5 items-center justify-center rounded-[5px] transition-colors",
                layout === value
                  ? "bg-raised text-ink shadow-[0_0_0_1px_var(--c-line-strong)]"
                  : "text-ink-4 hover:text-ink-2",
              )}
            >
              {icon}
            </button>
          ))}
        </div>
      )}
      <IconButton
        label={wrap ? "Scroll long lines (w)" : "Wrap long lines (w)"}
        aria-pressed={wrap}
        onClick={toggleWrap}
        className={cn("hidden sm:inline-flex", wrap && "bg-hover text-ink")}
      >
        <TextWrap className="size-4" />
      </IconButton>
      {blob && (
        <a
          href={blob}
          target="_blank"
          rel="noreferrer"
          title="The file on GitHub, on the session's branch"
          className="hidden h-8 shrink-0 items-center gap-1 rounded-md px-2 text-[12.5px] text-ink-3 transition-colors hover:bg-hover hover:text-ink md:inline-flex"
        >
          GitHub <ArrowUpRight className="size-3.5" />
        </a>
      )}
      <IconButton label="Close (Esc)" onClick={close}>
        <X className="size-4" />
      </IconButton>
    </header>
  );
}

/** Says, for a moment, that the file's diff changed while it is open: the agent is at work on it. */
function Updated({ session, entry }: { session: string; entry?: Entry }) {
  const q = useFileDiff(session, entry);
  const digest = q.data?.digest;
  const seen = useRef<string | undefined>(undefined);
  const [shown, setShown] = useState(false);
  useEffect(() => {
    if (!digest) return;
    const before = seen.current;
    seen.current = digest;
    if (!before || before === digest) return;
    setShown(true);
    const t = setTimeout(() => setShown(false), 2600);
    return () => clearTimeout(t);
  }, [digest]);
  // Another file: what it saw goes.
  // biome-ignore lint/correctness/useExhaustiveDependencies: when the file changes
  useEffect(() => {
    seen.current = undefined;
    setShown(false);
  }, [entry?.file.path, entry?.repo]);
  if (!shown) return null;
  return (
    <span className="inline-flex animate-fade items-center gap-1 rounded-full bg-accent-soft px-1.5 py-px text-[11px] text-accent">
      <span className="size-1.5 animate-pulse-soft rounded-full bg-accent" />
      Updated
    </span>
  );
}

/** The changed files of every repository, the one shown marked, with a filter for long lists. */
function FileList({
  groups,
  current,
  filterId,
  onSelect,
}: {
  groups: DiffGroup[];
  current: DiffTarget;
  filterId: string;
  onSelect: (t: DiffTarget) => void;
}) {
  const [filter, setFilter] = useState("");
  const list = useRef<HTMLDivElement>(null);
  const total = groups.reduce((n, g) => n + g.files.length, 0);
  const words = filter.toLowerCase().split(/\s+/).filter(Boolean);
  const shown = groups
    .map((g) => ({
      ...g,
      files: g.files.filter((f) => words.every((w) => f.path.toLowerCase().includes(w))),
    }))
    .filter((g) => g.files.length);
  const several = groups.filter((g) => g.files.length).length > 1;
  // The file shown stays in sight in the list.
  // biome-ignore lint/correctness/useExhaustiveDependencies: when the file shown changes
  useEffect(() => {
    list.current?.querySelector('[aria-current="true"]')?.scrollIntoView({ block: "nearest" });
  }, [current.path, current.repo]);
  return (
    <>
      {total > 6 && (
        <div className="shrink-0 border-b border-line p-2">
          <label className="flex h-8 items-center gap-2 rounded-md border border-line bg-sunken px-2.5 focus-within:border-line-strong">
            <Search className="size-3.5 shrink-0 text-ink-4" />
            <input
              id={filterId}
              value={filter}
              onChange={(e) => setFilter(e.target.value)}
              onKeyDown={(e) => {
                // Escape clears the filter before it closes the dialog.
                if (e.key === "Escape" && filter) {
                  e.preventDefault();
                  e.stopPropagation();
                  setFilter("");
                } else if (e.key === "Enter") {
                  const first = shown[0]?.files[0];
                  if (first) onSelect({ repo: shown[0]?.repo, path: first.path });
                }
              }}
              placeholder={`Filter ${total} files`}
              aria-label="Filter the changed files"
              autoComplete="off"
              spellCheck={false}
              className="min-w-0 flex-1 bg-transparent text-[12.5px] text-ink outline-none placeholder:text-ink-4"
            />
            {!filter && (
              <span className="hidden sm:inline-flex">
                <Kbd>/</Kbd>
              </span>
            )}
          </label>
        </div>
      )}
      <div ref={list} className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-1.5">
        {shown.length === 0 && (
          <p className="px-2.5 py-3 text-[12.5px] text-ink-4">No changed file matches.</p>
        )}
        {shown.map((g) => (
          <div key={g.repo ?? ""} className="mb-1.5 last:mb-0">
            {several && (
              <div className="truncate px-2.5 pt-2 pb-1 font-mono text-[11.5px] text-ink-3" title={g.title}>
                {g.title}
              </div>
            )}
            <ul>
              {g.files.map((f) => (
                <li key={f.path}>
                  <FileRow
                    file={f}
                    active={same({ repo: g.repo, path: f.path }, current)}
                    onClick={() => onSelect({ repo: g.repo, path: f.path })}
                  />
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
      <div className="hidden shrink-0 items-center gap-1.5 border-t border-line px-3 py-2 text-[11.5px] text-ink-4 lg:flex">
        <Kbd>j</Kbd>
        <Kbd>k</Kbd>
        <span className="mr-2">files</span>
        <Kbd>s</Kbd>
        <span className="mr-2">side by side</span>
        <Kbd>w</Kbd>
        <span>wrap</span>
      </div>
    </>
  );
}

function FileRow({ file, active, onClick }: { file: G.FileChange; active: boolean; onClick: () => void }) {
  const { dir, name } = split(file.path);
  const change = changeOf(file.status);
  const a = file.additions ?? 0;
  const d = file.deletions ?? 0;
  return (
    <button
      type="button"
      onClick={onClick}
      aria-current={active}
      title={file.path}
      className={cn(
        "group/file relative flex w-full items-center gap-2 rounded-md py-1.5 pr-2 pl-2.5 text-left transition-colors",
        active ? "bg-hover text-ink" : "text-ink-2 hover:bg-hover hover:text-ink",
      )}
    >
      {active && (
        <span className="absolute inset-y-1.5 left-0 w-[2px] rounded-full bg-accent" aria-hidden="true" />
      )}
      <span className="sr-only">{change.label}: </span>
      <span className={cn("w-3 shrink-0 text-center font-mono text-[11px]", change.tone)} aria-hidden="true">
        {change.letter}
      </span>
      <span className="flex min-w-0 flex-1 items-baseline gap-1.5">
        <span className="min-w-0 truncate font-mono text-[12.5px]">{name}</span>
        {dir && (
          <span className="min-w-0 shrink-[1000] truncate font-mono text-[11px] text-ink-4">
            {dir.slice(0, -1)}
          </span>
        )}
      </span>
      {(a > 0 || d > 0) && (
        <span className="shrink-0 font-mono text-[10.5px] text-ink-4 tabular-nums">
          {a > 0 && <span className="text-accent/80">+{a}</span>}
          {a > 0 && d > 0 && " "}
          {d > 0 && <span className="text-danger/80">−{d}</span>}
        </span>
      )}
    </button>
  );
}

/** The diff of the file shown, or why there is none to show. */
function Body({
  session,
  cliRef,
  entry,
  current,
  layout,
  wrap,
}: {
  session: string;
  cliRef: string;
  entry?: Entry;
  current: DiffTarget;
  layout: DiffLayout;
  wrap: boolean;
}) {
  const q = useFileDiff(session, entry);
  const diff = q.data;
  const hunks = useMemo(() => (diff?.patch ? parsePatch(diff.patch) : []), [diff?.patch]);
  const lang = useMemo(() => languageOf(current.path, firstLine(hunks)).id, [current.path, hunks]);
  if (!entry) {
    return (
      <Note icon={<FileQuestion />} title="No changes in this file any more">
        The agent's work no longer changes <code className="font-mono text-ink-2">{current.path}</code>.
      </Note>
    );
  }
  if (!entry.file.digest) {
    return (
      <Note icon={<History />} title="No diff of this file">
        Diffs come from the session's agent, and this one sent none: its runner runs an older version of fctl.
      </Note>
    );
  }
  if (!diff) {
    if (q.isError) {
      const missing = q.error instanceof ApiFailure && q.error.status === 404;
      return (
        <Note
          icon={<FileX2 />}
          title={missing ? "The session doesn't have this diff yet" : "The diff didn't load"}
          action={
            <Button size="sm" onClick={() => q.refetch()} loading={q.isFetching}>
              Try again
            </Button>
          }
        >
          {missing ? "The agent sends it with its next look at the repository." : q.error.message}
        </Note>
      );
    }
    return <Loading />;
  }
  const command = `fctl diff ${cliRef} ${/\s|['"$`\\]/.test(current.path) ? `'${current.path.replaceAll("'", "'\\''")}'` : current.path}${entry.repo ? ` --repo ${entry.repo}` : ""}`;
  const notes: ReactNode[] = [];
  if (diff.old_path) {
    notes.push(
      <span key="renamed">
        Renamed from <code className="font-mono text-ink-2">{diff.old_path}</code>
      </span>,
    );
  }
  if (diff.old_mode && diff.new_mode) {
    const exec = (m: string) => m.endsWith("755");
    notes.push(
      <span key="mode">
        Mode {diff.old_mode} → {diff.new_mode}
        {exec(diff.new_mode) && !exec(diff.old_mode) ? ": executable now" : ""}
        {exec(diff.old_mode) && !exec(diff.new_mode) ? ": no longer executable" : ""}
      </span>,
    );
  }
  let content: ReactNode;
  if (diff.binary) {
    content = (
      <Note icon={<Binary />} title="A binary file">
        git shows no lines of it.
      </Note>
    );
  } else if (diff.too_large) {
    content = (
      <Note icon={<FileX2 />} title="This diff is too large to show here">
        <p>
          {entry.file.additions ?? "?"} lines added and {entry.file.deletions ?? "?"} taken out: more than the
          session keeps. The CLI prints it:
        </p>
        <CodeBox value={command} className="mt-3 text-left" />
      </Note>
    );
  } else if (!hunks.length) {
    content = (
      <Note icon={<FileQuestion />} title={notes.length ? "No lines changed" : "An empty file"}>
        {notes.length ? "Only what's above changed." : "It has no lines."}
      </Note>
    );
  } else {
    content = (
      <DiffLines
        hunks={hunks}
        lang={lang}
        layout={layout}
        wrap={wrap}
        className={cn(
          "min-h-full pb-10 [--diff-bg:var(--c-surface)]",
          (q.isFetching || q.isPlaceholderData) && "opacity-90",
        )}
      />
    );
  }
  return (
    <div className="flex min-h-full flex-col">
      {notes.length > 0 && (
        <div className="flex flex-wrap gap-x-4 gap-y-1 border-b border-line bg-surface px-4 py-2 text-[12px] text-ink-3">
          {notes}
        </div>
      )}
      <div className="flex-1 animate-fade">{content}</div>
    </div>
  );
}

/** Lines that shimmer while the diff loads, as long as the file's change. */
function Loading() {
  const widths = ["w-2/5", "w-3/5", "w-1/3", "w-4/5", "w-1/2", "w-2/3", "w-1/4", "w-3/4", "w-2/5", "w-1/2"];
  return (
    <div className="flex flex-col gap-2.5 px-5 py-5" aria-busy="true">
      <span className="sr-only" role="status">
        Loading the diff
      </span>
      {widths.map((w, i) => (
        <div key={i} className="flex items-center gap-4" style={{ opacity: 1 - i * 0.07 }}>
          <Skeleton className="h-3 w-8 shrink-0" />
          <Skeleton className={cn("h-3", w)} />
        </div>
      ))}
    </div>
  );
}

/** Why the dialog shows no lines: a binary file, one too large, one an older agent sent no diff of. */
function Note({
  icon,
  title,
  children,
  action,
}: {
  icon: ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex min-h-[60%] animate-rise flex-col items-center justify-center px-6 py-16 text-center">
      <div className="mb-4 flex size-11 items-center justify-center rounded-full border border-line bg-sunken text-ink-3 [&_svg]:size-5">
        {icon}
      </div>
      <h3 className="text-[15px] font-medium text-ink">{title}</h3>
      {children && <div className="mt-1.5 max-w-md text-[13px] leading-relaxed text-ink-3">{children}</div>}
      {action && <div className="mt-5">{action}</div>}
    </div>
  );
}

// The diff dialog's list of the session's changed files, beside the diffs:
// grouped by repository and directory, the file in view marked, a filter
// for long lists, and what finding found in each file while it finds. The
// arrow keys go from file to file, as in a code editor's file tree.

import { Folder, Search } from "lucide-react";
import { type KeyboardEvent, type ReactNode, useEffect, useRef, useState } from "react";
import { Kbd } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { modKey } from "./diff-find";

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

/** A file's key in the dialog: its repository and path. */
export const entryKey = (repo: string | undefined, path: string) => `${repo ?? ""}\u0000${path}`;

/** How a file changed, as its letter in lists and its word in the dialog. */
export function changeOf(status: string): { letter: string; label: string; tone: string } {
  switch (status) {
    case "added":
      return { letter: "A", label: "Added", tone: "text-added" };
    case "untracked":
      return { letter: "A", label: "New, untracked", tone: "text-added" };
    case "deleted":
      return { letter: "D", label: "Deleted", tone: "text-danger" };
    case "renamed":
      return { letter: "R", label: "Renamed", tone: "text-blue" };
    default:
      return { letter: "M", label: "Modified", tone: "text-blue" };
  }
}

/** A path's directory, with its slash, and its name. */
export const splitPath = (path: string) => {
  const cut = path.lastIndexOf("/") + 1;
  return { dir: path.slice(0, cut), name: path.slice(cut) };
};

/**
 * A repository's files under their directories: those at its root first,
 * then each directory once, in the order the list has them, above its files.
 */
export function byDirectory(files: G.FileChange[]): { dir: string; files: G.FileChange[] }[] {
  const dirs = new Map<string, G.FileChange[]>([["", []]]);
  for (const f of files) {
    const { dir } = splitPath(f.path);
    const list = dirs.get(dir);
    if (list) list.push(f);
    else dirs.set(dir, [f]);
  }
  return [...dirs].map(([dir, files]) => ({ dir, files })).filter((d) => d.files.length > 0);
}

/** The changed files of every repository, the one in view marked, with a filter for long lists. */
export function FileList({
  groups,
  active,
  filterId,
  found,
  sideBySide,
  onSelect,
}: {
  groups: DiffGroup[];
  /** The file in view ([`entryKey`]). */
  active?: string;
  filterId: string;
  /** While finding: how many matches each file has. */
  found?: ReadonlyMap<string, number>;
  /** The diffs can go side by side: its key is worth showing. */
  sideBySide: boolean;
  onSelect: (key: string) => void;
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
  // The files as the list shows them, top to bottom, for the arrow keys.
  const order = shown.flatMap((g) =>
    byDirectory(g.files).flatMap((d) => d.files.map((f) => entryKey(g.repo, f.path))),
  );
  // The one row Tab stops at: the file in view, or the first one shown.
  const stop = active && order.includes(active) ? active : order[0];

  // The file in view stays in sight in the list.
  // biome-ignore lint/correctness/useExhaustiveDependencies: when the file in view changes
  useEffect(() => {
    list.current?.querySelector('[aria-current="true"]')?.scrollIntoView({ block: "nearest" });
  }, [active]);

  const row = (key: string | undefined) =>
    key === undefined ? null : list.current?.querySelector<HTMLElement>(`[data-row="${order.indexOf(key)}"]`);

  /** Goes to the file `key`, and takes the focus there, as arrow keys do in a list. */
  const select = (key: string | undefined) => {
    if (key === undefined) return;
    onSelect(key);
    row(key)?.focus({ preventScroll: true });
    row(key)?.scrollIntoView({ block: "nearest" });
  };

  const onKeyDown = (e: KeyboardEvent<HTMLDivElement>) => {
    if (e.metaKey || e.ctrlKey || e.altKey) return;
    const at = stop === undefined ? -1 : order.indexOf(stop);
    // In the list, j and k go from file to file as the arrows do, the focus with them.
    const key = e.key === "j" ? "ArrowDown" : e.key === "k" ? "ArrowUp" : e.key;
    let to: string | undefined;
    if (key === "ArrowDown") to = order[Math.min(at + 1, order.length - 1)];
    else if (key === "ArrowUp") to = order[Math.max(at - 1, 0)];
    else if (key === "Home") to = order[0];
    else if (key === "End") to = order[order.length - 1];
    else return;
    if ((e.target as HTMLElement).tagName === "INPUT") return;
    e.preventDefault();
    e.stopPropagation();
    select(to);
  };

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
                  if (order[0]) onSelect(order[0]);
                } else if (e.key === "ArrowDown") {
                  e.preventDefault();
                  select(order[0]);
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
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the arrow keys go between the buttons it holds */}
      <div
        ref={list}
        onKeyDown={onKeyDown}
        className="min-h-0 flex-1 overflow-y-auto overscroll-contain p-1.5"
      >
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
              {byDirectory(g.files).map(({ dir, files }) => (
                <li key={dir}>
                  {dir && (
                    <div
                      className="flex min-w-0 items-center gap-1.5 px-2.5 pt-2 pb-0.5 font-mono text-[11px] text-ink-4"
                      title={dir}
                    >
                      <Folder className="size-3 shrink-0" aria-hidden="true" />
                      <span className="truncate">{dir.slice(0, -1)}</span>
                    </div>
                  )}
                  <ul>
                    {files.map((f) => {
                      const key = entryKey(g.repo, f.path);
                      return (
                        <li key={f.path}>
                          <FileRow
                            file={f}
                            row={order.indexOf(key)}
                            nested={!!dir}
                            active={key === active}
                            tabbable={key === stop}
                            found={found && (found.get(key) ?? 0)}
                            onClick={() => onSelect(key)}
                          />
                        </li>
                      );
                    })}
                  </ul>
                </li>
              ))}
            </ul>
          </div>
        ))}
      </div>
      <div className="hidden shrink-0 flex-wrap items-center gap-x-3 gap-y-1.5 border-t border-line px-3 py-2 text-[11.5px] text-ink-4 lg:flex">
        <Hint keys={["j", "k"]}>files</Hint>
        <Hint keys={["n", "p"]}>changes</Hint>
        <Hint keys={[modKey("F")]}>find</Hint>
        {sideBySide && <Hint keys={["s"]}>side by side</Hint>}
        <Hint keys={["w"]}>wrap</Hint>
      </div>
    </>
  );
}

function Hint({ keys, children }: { keys: string[]; children: ReactNode }) {
  return (
    <span className="inline-flex items-center gap-1">
      {keys.map((k) => (
        <Kbd key={k}>{k}</Kbd>
      ))}
      <span className="ml-0.5">{children}</span>
    </span>
  );
}

/**
 * A file in the list: how it changed, its name (its directory is above it),
 * its lines added and taken out, or, while finding, how many matches it has.
 */
function FileRow({
  file,
  row,
  nested,
  active,
  tabbable,
  found,
  onClick,
}: {
  file: G.FileChange;
  /** Its place among the rows shown, for the arrow keys. */
  row: number;
  /** Under its directory: indented. */
  nested: boolean;
  /** It is in view. */
  active: boolean;
  /** Tab stops at it: the list is one stop, which the arrow keys move within. */
  tabbable: boolean;
  found?: number;
  onClick: () => void;
}) {
  const { name } = splitPath(file.path);
  const change = changeOf(file.status);
  const a = file.additions ?? 0;
  const d = file.deletions ?? 0;
  return (
    <button
      type="button"
      onClick={onClick}
      data-row={row}
      tabIndex={tabbable ? 0 : -1}
      aria-current={active}
      title={file.path}
      className={cn(
        "group/file relative flex w-full items-center gap-2 rounded-md py-1.5 pr-2 text-left transition-[background-color,color,opacity]",
        // Focus shows as the row's own outline, inside it, not the page's yellow ring around it.
        "focus-visible:shadow-[inset_0_0_0_1px_var(--c-line-strong)] focus-visible:outline-none",
        nested ? "pl-[22px]" : "pl-2.5",
        active ? "bg-hover text-ink" : "text-ink-2 hover:bg-hover hover:text-ink focus-visible:bg-hover",
        found === 0 && !active && "opacity-45",
      )}
    >
      {active && (
        <span className="absolute inset-y-1.5 left-0 w-[2px] rounded-full bg-accent" aria-hidden="true" />
      )}
      <span className="sr-only">{change.label}: </span>
      <span className={cn("w-3 shrink-0 text-center font-mono text-[11px]", change.tone)} aria-hidden="true">
        {change.letter}
      </span>
      <span className="min-w-0 flex-1 truncate font-mono text-[12.5px]">{name}</span>
      {found !== undefined
        ? found > 0 && (
            <span className="shrink-0 rounded-full bg-accent-soft px-1.5 font-mono text-[10.5px] text-ink tabular-nums">
              {found}
              <span className="sr-only"> {found === 1 ? "match" : "matches"}</span>
            </span>
          )
        : (a > 0 || d > 0) && (
            <span className="shrink-0 font-mono text-[10.5px] text-ink-4 tabular-nums">
              {a > 0 && <span className="text-added/80">+{a}</span>}
              {a > 0 && d > 0 && " "}
              {d > 0 && <span className="text-danger/80">−{d}</span>}
            </span>
          )}
    </button>
  );
}

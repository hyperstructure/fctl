// A repository's changed files and totals, as the session page's rail and
// details sheet show them: the first few files, and the rest a click away.

import { ChevronDown } from "lucide-react";
import { useId, useRef, useState } from "react";
import { cn } from "@/lib/cn";
import { type RepoChanges, whereCommits } from "@/lib/git";

/** The files a repository's list shows until someone asks for the rest. */
const FEW = 10;

/** A repository's changed files and totals. */
export function Changes({
  repo,
  title,
  showTotals,
}: {
  repo: RepoChanges;
  /** The repository's name, when the session has more than one. */
  title?: string;
  showTotals: boolean;
}) {
  const [all, setAll] = useState(false);
  const list = useId();
  const toggle = useRef<HTMLButtonElement>(null);
  const { files, ahead } = repo;
  // The files past the first few: none when a button would take the place of just one.
  const more = files.length > FEW + 1 ? files.length - FEW : 0;
  const shown = all || !more ? files : files.slice(0, FEW);
  const maxChange = Math.max(1, ...files.map((f) => (f.additions ?? 0) + (f.deletions ?? 0)));
  return (
    <div>
      {title && (
        <div className="mb-1.5 flex items-baseline justify-between gap-2">
          <span className="min-w-0 truncate font-mono text-[12px] text-ink-2" title={title}>
            {title}
          </span>
          {repo.readOnly && <span className="shrink-0 text-[11.5px] text-ink-4">read only</span>}
        </div>
      )}
      {files.length === 0 ? (
        <p className="text-[12.5px] text-ink-4">No changes yet.</p>
      ) : (
        <ul id={list} className="flex flex-col gap-1">
          {shown.map((f) => {
            const a = f.additions ?? 0;
            const d = f.deletions ?? 0;
            return (
              <li key={f.path} className="flex items-center gap-2 text-[12px]">
                <span
                  className={cn(
                    "w-3 shrink-0 text-center font-mono text-[11px]",
                    f.status === "added" || f.status === "untracked"
                      ? "text-accent"
                      : f.status === "deleted"
                        ? "text-danger"
                        : "text-blue",
                  )}
                >
                  {f.status === "added" || f.status === "untracked"
                    ? "A"
                    : f.status === "deleted"
                      ? "D"
                      : f.status === "renamed"
                        ? "R"
                        : "M"}
                </span>
                <span className="min-w-0 flex-1 truncate font-mono text-ink-2" title={f.path}>
                  {f.path}
                </span>
                <span className="flex h-1.5 w-12 shrink-0 overflow-hidden rounded-full bg-raised">
                  <span className="bg-accent/80" style={{ width: `${(a / maxChange) * 100}%` }} />
                  <span className="bg-danger/70" style={{ width: `${(d / maxChange) * 100}%` }} />
                </span>
              </li>
            );
          })}
        </ul>
      )}
      {more > 0 && (
        // Lined up with the paths, its chevron where the files have their status.
        <button
          ref={toggle}
          type="button"
          aria-expanded={all}
          aria-controls={list}
          onClick={() => {
            setAll(!all);
            // Folding a long list moves the button up, maybe out of sight.
            if (all) requestAnimationFrame(() => toggle.current?.scrollIntoView({ block: "nearest" }));
          }}
          className="mt-0.5 -ml-1.5 flex w-fit items-center gap-2 rounded-md px-1.5 py-0.5 text-[12px] text-ink-3 transition-colors hover:bg-hover hover:text-ink"
        >
          <ChevronDown className={cn("size-3 transition-transform duration-200", all && "rotate-180")} />
          {all ? "Show fewer files" : `Show ${more} more files`}
        </button>
      )}
      {showTotals && (
        <div className="mt-3 flex flex-wrap gap-x-3 text-[12px] text-ink-4">
          <span>
            <span className="text-accent">+{repo.additions}</span>{" "}
            <span className="text-danger">−{repo.deletions}</span>
          </span>
          {ahead > 0 && (
            <span>
              {ahead} {ahead === 1 ? "commit" : "commits"}
            </span>
          )}
          {whereCommits(repo).map((where) => (
            <span key={where}>{where}</span>
          ))}
        </div>
      )}
    </div>
  );
}

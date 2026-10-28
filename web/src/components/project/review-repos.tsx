// Which of a project's repositories its review agents review, in their settings: each repository its
// agents may write to, with a switch. Owners turn one off where another project's review agents
// review it already, so that its pull requests aren't reviewed twice.

import { useId } from "react";
import { GithubIcon, Switch } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { repoName, sameRepo } from "@/lib/reviews";

/**
 * The repositories the review agents may review (`repos`, in order), each on unless `skipped` (as the
 * form has it, `owner/name`) names it, which the project's owners change; the form saves them.
 */
export function ReviewRepos({
  repos,
  primary,
  skipped,
  onChange,
  error,
}: {
  repos: readonly G.RepoRef[];
  primary?: G.RepoRef;
  skipped: readonly string[];
  onChange: (skipped: string[]) => void;
  error?: string;
}) {
  const id = useId();
  const named = (r: G.RepoRef) => (s: string) => s.toLowerCase() === repoName(r).toLowerCase();
  const set = (r: G.RepoRef, on: boolean) =>
    onChange(on ? skipped.filter((s) => !named(r)(s)) : [...skipped, repoName(r)]);
  return (
    <fieldset aria-describedby={`${id}-hint`} className="flex min-w-0 flex-col gap-1.5">
      <legend className="mb-1.5 text-[12.5px] font-medium text-ink-2">Repositories</legend>
      <p id={`${id}-hint`} className="text-[12.5px] leading-relaxed text-ink-3">
        Review agents review pull requests in the repositories that are on, and in those linked later. Turn
        off one that another project's review agents review already, so that its pull requests aren't reviewed
        twice.
      </p>
      <ul
        aria-label="Repositories they review"
        className="mt-1.5 divide-y divide-line overflow-hidden rounded-lg border border-line"
      >
        {repos.length === 0 && (
          <li className="px-3.5 py-3 text-[13px] text-ink-4">
            None: link one the project's agents may write to under Repositories.
          </li>
        )}
        {repos.map((r) => {
          const name = repoName(r);
          const on = !skipped.some(named(r));
          return (
            <li key={name} className="flex items-center gap-3 px-3.5 py-2.5">
              <GithubIcon
                className={cn(
                  "size-4 shrink-0 transition-colors duration-200",
                  on ? "text-ink-3" : "text-ink-4",
                )}
              />
              <span className="flex min-w-0 flex-1 items-baseline gap-2">
                <a
                  href={`https://github.com/${name}`}
                  target="_blank"
                  rel="noreferrer"
                  title={`${name} on GitHub`}
                  className={cn(
                    "truncate font-mono text-[13px] underline-offset-2 transition-colors duration-200 hover:underline",
                    on ? "text-ink" : "text-ink-4",
                  )}
                >
                  {name}
                </a>
                {primary && sameRepo(r, primary) && (
                  <span className="shrink-0 text-[11.5px] text-ink-4 max-sm:hidden">Primary</span>
                )}
              </span>
              {!on && <span className="shrink-0 text-[12px] text-ink-4">Skipped</span>}
              <Switch checked={on} onChange={(v) => set(r, v)} label={`Review ${name}`} />
            </li>
          );
        })}
      </ul>
      {error && (
        <p className="animate-rise text-[12.5px] text-danger [overflow-wrap:anywhere]" role="alert">
          {error}
        </p>
      )}
    </fieldset>
  );
}

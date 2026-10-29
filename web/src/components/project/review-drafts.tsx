// Whether a project's review agents review drafts, in their settings: each new commit of a draft, as
// of other pull requests, or none until it's marked ready for review. Once they leave drafts, it shows
// what that looks like on a pull request.

import { GitCommitHorizontal } from "lucide-react";
import type { ReactNode } from "react";
import { ReviewStateIcon } from "@/components/review";
import { PullRequestIcon, Switch } from "@/components/ui";

/** The switch, with what it does now; the form saves it. */
export function ReviewDrafts({ on, onChange }: { on: boolean; onChange: (on: boolean) => void }) {
  return (
    <div className="flex flex-col gap-3">
      <div className="flex items-start justify-between gap-4">
        <div className="min-w-0">
          <p className="text-[12.5px] font-medium text-ink-2">Drafts</p>
          <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
            {on
              ? "Each new commit of a draft gets a review, as other pull requests' commits do. Turn it off to wait until a draft is marked ready for review."
              : "Drafts get no review until they're marked ready for review, which starts one of their newest commit. Until then, people who start sessions here can ask for one."}
          </p>
        </div>
        <Switch checked={on} onChange={onChange} label="Review drafts" />
      </div>
      {!on && <Timeline />}
    </div>
  );
}

/** A draft's life on GitHub, as review agents that leave drafts take it. */
const STEPS: { icon: ReactNode; text: string; reviewed?: boolean }[] = [
  {
    icon: <PullRequestIcon state="draft" className="size-3.5 text-pr-draft" />,
    text: "Opened as a draft",
  },
  { icon: <GitCommitHorizontal className="size-3.5 text-ink-3" />, text: "2 new commits" },
  {
    icon: <PullRequestIcon state="open" className="size-3.5 text-pr-open" />,
    text: "Ready for review",
    reviewed: true,
  },
];

function Timeline() {
  return (
    <ol aria-hidden="true" className="animate-rise rounded-lg border border-line bg-surface px-3.5 py-2">
      {STEPS.map((s, i) => (
        <li key={s.text} className="relative flex items-center gap-3 py-1.5 text-[12.5px]">
          {i < STEPS.length - 1 && (
            <span className="absolute top-1/2 left-[13px] h-full w-px bg-line" aria-hidden="true" />
          )}
          <span className="relative grid size-[27px] shrink-0 place-items-center rounded-full border border-line bg-surface">
            {s.icon}
          </span>
          <span className="min-w-0 flex-1 truncate text-ink-2">{s.text}</span>
          {s.reviewed ? (
            <span className="flex shrink-0 items-center gap-1.5">
              <ReviewStateIcon look="reviewing" className="size-3.5" />
              <span className="font-semibold text-ink">fctl review</span>
              <span className="text-ink-3 max-sm:hidden">Reviewing</span>
            </span>
          ) : (
            <span className="shrink-0 text-ink-4">No review</span>
          )}
        </li>
      ))}
    </ol>
  );
}

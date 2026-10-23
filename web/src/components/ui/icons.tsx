// Icons lucide does not ship: GitHub's mark and its octicons for pull requests and their
// checks, as GitHub draws them (16 units, filled), and fctl's own, drawn on lucide's grid
// (24 units, 2-unit round strokes) so that they sit beside its icons.

import type { CSSProperties } from "react";

/**
 * The Agent Floor: three agents whose traces run into a core with its light on, as the
 * foundry's threads stream into its planet, drawn as a circuit board. With `live`, packets run
 * down the traces into the core while an ancestor has `data-lit` (styles/app.css).
 */
export function AgentFloorIcon({ className, live }: { className?: string; live?: boolean }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className ?? "size-4"}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <circle cx="16.5" cy="12" r="4.5" />
      <circle
        className={live ? "floor-core" : undefined}
        cx="16.5"
        cy="12"
        r="1.5"
        fill="currentColor"
        stroke="none"
      />
      {FLOOR_TRACES.map((d, i) => (
        <g key={d}>
          <circle cx="4" cy={6 + i * 6} r="2" fill="currentColor" stroke="none" />
          <path d={d} />
          {live && (
            <path d={d} pathLength={1} className="floor-packet" style={{ "--i": i } as CSSProperties} />
          )}
        </g>
      ))}
    </svg>
  );
}

/** The Agent Floor's traces, from each agent into the core: the outer two turn towards its center. */
const FLOOR_TRACES = ["M6 6h4.5l2.8 2.8", "M6 12h6", "M6 18h4.5l2.8-2.8"];

export function GithubIcon({ className }: { className?: string }) {
  return (
    <svg viewBox="0 0 16 16" className={className ?? "size-4"} fill="currentColor" aria-hidden="true">
      <path d="M8 0C3.58 0 0 3.58 0 8a8 8 0 0 0 5.47 7.59c.4.07.55-.17.55-.38v-1.33c-2.23.48-2.7-1.08-2.7-1.08-.36-.92-.89-1.17-.89-1.17-.73-.5.06-.49.06-.49.8.06 1.23.83 1.23.83.72 1.23 1.88.87 2.34.67.07-.52.28-.87.5-1.07-1.78-.2-3.65-.89-3.65-3.95 0-.87.31-1.59.82-2.15-.08-.2-.36-1.02.08-2.12 0 0 .67-.21 2.2.82a7.6 7.6 0 0 1 4 0c1.53-1.04 2.2-.82 2.2-.82.44 1.1.16 1.92.08 2.12.51.56.82 1.27.82 2.15 0 3.07-1.87 3.75-3.65 3.95.29.25.54.73.54 1.48v2.2c0 .21.15.46.55.38A8 8 0 0 0 16 8c0-4.42-3.58-8-8-8z" />
    </svg>
  );
}

/** GitHub's octicons for a pull request's state, as GitHub shows them. */
const PULL_REQUEST_PATHS = {
  open: "M1.5 3.25a2.25 2.25 0 1 1 3 2.122v5.256a2.251 2.251 0 1 1-1.5 0V5.372A2.25 2.25 0 0 1 1.5 3.25Zm5.677-.177L9.573.677A.25.25 0 0 1 10 .854V2.5h1A2.5 2.5 0 0 1 13.5 5v5.628a2.251 2.251 0 1 1-1.5 0V5a1 1 0 0 0-1-1h-1v1.646a.25.25 0 0 1-.427.177L7.177 3.427a.25.25 0 0 1 0-.354ZM3.75 2.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm0 9.5a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm8.25.75a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0Z",
  draft:
    "M3.25 1A2.25 2.25 0 0 1 4 5.372v5.256a2.251 2.251 0 1 1-1.5 0V5.372A2.251 2.251 0 0 1 3.25 1Zm9.5 14a2.25 2.25 0 1 1 0-4.5 2.25 2.25 0 0 1 0 4.5ZM2.5 3.25a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0ZM3.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm9.5 0a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5ZM14 7.5a1.25 1.25 0 1 1-2.5 0 1.25 1.25 0 0 1 2.5 0Zm0-4.25a1.25 1.25 0 1 1-2.5 0 1.25 1.25 0 0 1 2.5 0Z",
  queued:
    "M3.75 4.5a1.25 1.25 0 1 0 0-2.5 1.25 1.25 0 0 0 0 2.5ZM3 7.75a.75.75 0 0 1 1.5 0v2.878a2.251 2.251 0 1 1-1.5 0Zm.75 5.75a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm5-7.75a1.25 1.25 0 1 1-2.5 0 1.25 1.25 0 0 1 2.5 0Zm5.75 2.5a2.25 2.25 0 1 1-4.5 0 2.25 2.25 0 0 1 4.5 0Zm-1.5 0a.75.75 0 1 0-1.5 0 .75.75 0 0 0 1.5 0Z",
  merged:
    "M5.45 5.154A4.25 4.25 0 0 0 9.25 7.5h1.378a2.251 2.251 0 1 1 0 1.5H9.25A5.734 5.734 0 0 1 5 7.123v3.505a2.25 2.25 0 1 1-1.5 0V5.372a2.25 2.25 0 1 1 1.95-.218ZM4.25 13.5a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5Zm8.5-4.5a.75.75 0 1 0 0-1.5.75.75 0 0 0 0 1.5ZM5 3.25a.75.75 0 1 0 0 .005V3.25Z",
  closed:
    "M3.25 1A2.25 2.25 0 0 1 4 5.372v5.256a2.251 2.251 0 1 1-1.5 0V5.372A2.251 2.251 0 0 1 3.25 1Zm9.5 5.5a.75.75 0 0 1 .75.75v3.378a2.251 2.251 0 1 1-1.5 0V7.25a.75.75 0 0 1 .75-.75Zm-2.03-5.273a.75.75 0 0 1 1.06 0l.97.97.97-.97a.748.748 0 0 1 1.265.332.75.75 0 0 1-.205.729l-.97.97.97.97a.751.751 0 0 1-.018 1.042.751.751 0 0 1-1.042.018l-.97-.97-.97.97a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734l.97-.97-.97-.97a.75.75 0 0 1 0-1.06ZM2.5 3.25a.75.75 0 1 0 1.5 0 .75.75 0 0 0-1.5 0ZM3.25 12a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Zm9.5 0a.75.75 0 1 0 0 1.5.75.75 0 0 0 0-1.5Z",
} as const;

/** GitHub's octicons for a pull request's checks: all passed, still to finish, or one failed. */
const CHECKS_PATHS = {
  passed:
    "M13.78 4.22a.75.75 0 0 1 0 1.06l-7.25 7.25a.75.75 0 0 1-1.06 0L2.22 9.28a.751.751 0 0 1 .018-1.042.751.751 0 0 1 1.042-.018L6 10.94l6.72-6.72a.75.75 0 0 1 1.06 0Z",
  pending: "M8 4a4 4 0 1 1 0 8 4 4 0 0 1 0-8Z",
  failed:
    "M3.72 3.72a.75.75 0 0 1 1.06 0L8 6.94l3.22-3.22a.749.749 0 0 1 1.275.326.749.749 0 0 1-.215.734L9.06 8l3.22 3.22a.749.749 0 0 1-.326 1.275.749.749 0 0 1-.734-.215L8 9.06l-3.22 3.22a.751.751 0 0 1-1.042-.018.751.751 0 0 1-.018-1.042L6.94 8 3.72 4.78a.75.75 0 0 1 0-1.06Z",
} as const;

function Octicon({ path, className }: { path: string; className?: string }) {
  return (
    <svg viewBox="0 0 16 16" className={className ?? "size-4"} fill="currentColor" aria-hidden="true">
      <path d={path} />
    </svg>
  );
}

/** A pull request in a state: git-pull-request when open (and when nobody knows), git-pull-request-draft,
 * git-merge-queue, git-merge or git-pull-request-closed. */
export function PullRequestIcon({
  state,
  className,
}: {
  state: keyof typeof PULL_REQUEST_PATHS | "unknown";
  className?: string;
}) {
  return <Octicon path={PULL_REQUEST_PATHS[state === "unknown" ? "open" : state]} className={className} />;
}

/** A pull request's checks: check, dot-fill or x. */
export function ChecksIcon({ state, className }: { state: keyof typeof CHECKS_PATHS; className?: string }) {
  return <Octicon path={CHECKS_PATHS[state]} className={className} />;
}

/** GitHub's alert octicon, which its pull requests show for conflicts with their base branch. */
const CONFLICTS_PATH =
  "M6.457 1.047c.659-1.234 2.427-1.234 3.086 0l6.082 11.378A1.75 1.75 0 0 1 14.082 15H1.918a1.75 1.75 0 0 1-1.543-2.575Zm1.763.707a.25.25 0 0 0-.44 0L1.698 13.132a.25.25 0 0 0 .22.368h12.164a.25.25 0 0 0 .22-.368Zm.53 3.996v2.5a.75.75 0 0 1-1.5 0v-2.5a.75.75 0 0 1 1.5 0ZM9 11a1 1 0 1 1-2 0 1 1 0 0 1 2 0Z";

/** A pull request that conflicts with its base branch: alert. */
export function ConflictsIcon({ className }: { className?: string }) {
  return <Octicon path={CONFLICTS_PATH} className={className} />;
}

/** GitHub's octicons for what a review left: comments (comment), or nothing, superseded or stopped (skip). */
const REVIEW_PATHS = {
  comment:
    "M1 2.75C1 1.784 1.784 1 2.75 1h10.5c.966 0 1.75.784 1.75 1.75v7.5A1.75 1.75 0 0 1 13.25 12H9.06l-2.573 2.573A1.458 1.458 0 0 1 4 13.543V12H2.75A1.75 1.75 0 0 1 1 10.25Zm1.75-.25a.25.25 0 0 0-.25.25v7.5c0 .138.112.25.25.25h2a.75.75 0 0 1 .75.75v2.19l2.72-2.72a.749.749 0 0 1 .53-.22h4.5a.25.25 0 0 0 .25-.25v-7.5a.25.25 0 0 0-.25-.25Z",
  skip: "M8 0a8 8 0 1 1 0 16A8 8 0 0 1 8 0ZM1.5 8a6.5 6.5 0 1 0 13 0 6.5 6.5 0 0 0-13 0Zm9.78-2.22-5.5 5.5a.749.749 0 0 1-1.275-.326.749.749 0 0 1 .215-.734l5.5-5.5a.751.751 0 0 1 1.042.018.751.751 0 0 1 .018 1.042Z",
} as const;

export function ReviewIcon({ kind, className }: { kind: keyof typeof REVIEW_PATHS; className?: string }) {
  return <Octicon path={REVIEW_PATHS[kind]} className={className} />;
}

/** A thought: a cloud with the bubbles that lead up to it, as comics draw one. */
export function ThoughtIcon({ className }: { className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className ?? "size-4"}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      <path d="M18.4 13.7h-6.8a5.6 5.6 0 1 1 5.37-7.2h1.43a3.6 3.6 0 1 1 0 7.2Z" />
      <circle cx="6.1" cy="17.6" r="1.9" fill="currentColor" stroke="none" />
      <circle cx="3.2" cy="20.85" r="1.15" fill="currentColor" stroke="none" />
    </svg>
  );
}

/**
 * An effort level by the symbol Claude Code shows for it: ○ low, ◐ medium, ● high, ◉ extra high,
 * ◈ max, and a dotted circle for Claude's default (`""`).
 */
export function EffortIcon({ level, className }: { level: string; className?: string }) {
  return (
    <svg
      viewBox="0 0 24 24"
      className={className ?? "size-4"}
      fill="none"
      stroke="currentColor"
      strokeWidth="2"
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
    >
      {level === "max" ? (
        <>
          <path d="m12 3 9 9-9 9-9-9Z" />
          <path d="m12 8.5 3.5 3.5-3.5 3.5-3.5-3.5Z" fill="currentColor" />
        </>
      ) : (
        <>
          <circle
            cx="12"
            cy="12"
            r="8"
            fill={level === "high" ? "currentColor" : "none"}
            // Ten round dots around it.
            strokeDasharray={level ? undefined : "0 5.0265"}
          />
          {level === "medium" && <path d="M12 4a8 8 0 0 0 0 16Z" fill="currentColor" />}
          {level === "xhigh" && <circle cx="12" cy="12" r="3.5" fill="currentColor" stroke="none" />}
        </>
      )}
    </svg>
  );
}

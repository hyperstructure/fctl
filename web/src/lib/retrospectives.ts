// Retrospectives: what agents record as they finish a piece of work
// (fctl_domain::retrospectives on the Rust side).
import type * as G from "@/gen";
import type { Item } from "./transcript";

export const OUTCOMES: { value: G.RetrospectiveOutcome; label: string; description: string }[] = [
  { value: "completed", label: "Completed", description: "What was asked is done" },
  { value: "partial", label: "Partly done", description: "Some of it is left for later" },
  { value: "blocked", label: "Blocked", description: "It needs a person to go on" },
];

export const outcomeLabel = (o: G.RetrospectiveOutcome) =>
  OUTCOMES.find((x) => x.value === o)?.label ?? "Completed";

/** The parts of a retrospective that say something, in the order agents write them. */
export function parts(r: G.Retrospective): { key: string; label: string; text: string }[] {
  return [
    { key: "worked_on", label: "Worked on", text: r.worked_on },
    { key: "approach", label: "Approach", text: r.approach ?? "" },
    { key: "challenges", label: "Challenges", text: r.challenges ?? "" },
    { key: "insights", label: "Insights", text: r.insights ?? "" },
  ].filter((p) => p.text.trim());
}

/**
 * The retrospectives a session's transcript has that its list doesn't: new
 * ones the list doesn't have yet, or deleted ones.
 */
export function unlisted(listed: G.Retrospective[] | undefined, items: Item[]): string[] {
  const known = new Set((listed ?? []).map((r) => r.id));
  return items.flatMap((it) =>
    it.kind === "notice" && it.notice.retrospective && !known.has(it.notice.retrospective.id)
      ? [it.notice.retrospective.id]
      : [],
  );
}

/**
 * Whether someone may delete a retrospective: those who manage its session
 * (who started it, and the project's owners and maintainers).
 */
export const canDelete = (r: G.Retrospective, role?: G.ProjectRole, me?: string) =>
  role === "owner" || role === "maintainer" || (!!me && r.started_by?.id === me);

/** How many retrospectives there are in all. */
export const total = (o?: G.OutcomeCounts) => (o ? o.completed + o.partial + o.blocked : 0);

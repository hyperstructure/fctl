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
 * A session's retrospectives, newest first and each once: those the
 * control plane listed, and those its transcript has that came since.
 */
export function merged(listed: G.Retrospective[] | undefined, items: Item[]): G.Retrospective[] {
  const byId = new Map<string, G.Retrospective>();
  for (const r of listed ?? []) byId.set(r.id, r);
  for (const it of items)
    if (it.kind === "notice" && it.notice.retrospective && !byId.has(it.notice.retrospective.id))
      byId.set(it.notice.retrospective.id, it.notice.retrospective);
  return [...byId.values()].sort((a, b) => b.created_at - a.created_at || b.id.localeCompare(a.id));
}

/**
 * Whether someone may delete a retrospective: those who manage its session
 * (who started it, and the project's owners and maintainers).
 */
export const canDelete = (r: G.Retrospective, role?: G.ProjectRole, me?: string) =>
  role === "owner" || role === "maintainer" || (!!me && r.started_by?.id === me);

/** How many retrospectives there are in all. */
export const total = (o?: G.OutcomeCounts) => (o ? o.completed + o.partial + o.blocked : 0);

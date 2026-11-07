// The agent's plan: the to-do list it keeps with Claude Code's TodoWrite, as it last wrote it.
// The new session page (routes/session-next.tsx) keeps it in view beside the transcript, and says
// above the message box where the agent is in it, in the words Claude Code shows for the step at
// hand ("Formatting prices in DKK").

import type { Json } from "./claude-types";
import type { Item } from "./transcript";

export type PlanStatus = "pending" | "in_progress" | "completed";

export interface PlanStep {
  /** What the step is: "Format prices in DKK". */
  text: string;
  /** What the agent says while it does it: "Formatting prices in DKK". */
  doing?: string;
  status: PlanStatus;
}

export interface Plan {
  steps: PlanStep[];
  /** Steps done. */
  done: number;
  /** The step the agent is at, if it says. */
  current?: PlanStep;
  /** The call that wrote it: its key, its event, and when. */
  key: string;
  seq: number;
  ts: number;
}

const str = (v: Json | undefined) => (typeof v === "string" ? v.trim() : "");

/** A plan's steps, from a TodoWrite call's input. */
export function stepsOf(input: Record<string, Json>): PlanStep[] {
  const todos = Array.isArray(input.todos) ? input.todos : [];
  return todos.flatMap((t) => {
    if (!t || typeof t !== "object" || Array.isArray(t)) return [];
    const text = str(t.content);
    if (!text) return [];
    const s = str(t.status);
    const status: PlanStatus = s === "completed" || s === "in_progress" ? s : "pending";
    const doing = str(t.activeForm);
    return [{ text, status, ...(doing ? { doing } : {}) }];
  });
}

/**
 * The agent's newest plan in the transcript, as it last wrote it: none before it writes one, nor
 * once it clears the list (an empty one).
 */
export function planOf(items: readonly Item[]): Plan | undefined {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it?.kind !== "tool" || it.name !== "TodoWrite") continue;
    const steps = stepsOf(it.input);
    if (!steps.length) return undefined;
    return {
      steps,
      done: steps.filter((s) => s.status === "completed").length,
      current: steps.find((s) => s.status === "in_progress"),
      key: it.key,
      seq: it.seq,
      ts: it.ts,
    };
  }
  return undefined;
}

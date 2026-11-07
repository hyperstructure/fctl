import { describe, expect, it } from "vitest";
import { planOf, stepsOf } from "./plan";
import type { Item } from "./transcript";

let seq = 0;
const todo = (todos: unknown[]): Item => ({
  kind: "tool",
  key: `t${++seq}`,
  seq,
  ts: seq,
  id: `toolu_${seq}`,
  name: "TodoWrite",
  input: { todos } as never,
  children: 0,
});

describe("the agent's plan", () => {
  it("is its newest to-do list, with how far it got and the step it is at", () => {
    const plan = planOf([
      todo([{ content: "Read the ticket", status: "in_progress" }]),
      { kind: "prose", key: "p", seq: 99, ts: 99, text: "On it." },
      todo([
        { content: "Read the ticket", status: "completed", activeForm: "Reading the ticket" },
        { content: "Format prices in DKK", status: "in_progress", activeForm: "Formatting prices in DKK" },
        { content: "Tests", status: "pending" },
      ]),
    ]);
    expect(plan?.done).toBe(1);
    expect(plan?.steps).toHaveLength(3);
    expect(plan?.current).toEqual({
      text: "Format prices in DKK",
      doing: "Formatting prices in DKK",
      status: "in_progress",
    });
  });

  it("is nothing until the agent writes one, and leaves out what it can't read", () => {
    expect(planOf([{ kind: "prose", key: "p", seq: 1, ts: 1, text: "Hi" }])).toBeUndefined();
    expect(planOf([todo([])])).toBeUndefined();
    expect(
      stepsOf({ todos: [null, "x", { content: " " }, { content: "Ship", status: "odd" }] as never }),
    ).toEqual([{ text: "Ship", status: "pending" }]);
  });
});

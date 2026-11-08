import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { parts } from "@/lib/runs";
import type { Item } from "@/lib/transcript";
import { factsOf, turnsOf } from "@/lib/turns";
import { TurnList } from "./turn-nav";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { container, rerender: (n: ReactNode) => act(() => root.render(n)) };
}

let seq = 0;
const base = () => ({ key: `k${++seq}`, seq, ts: seq * 1000 });
/** `n` turns, each an ask and how it ended. */
const turns = (n: number) =>
  turnsOf(
    parts(
      Array.from({ length: n }, (_, i): Item[] => [
        { ...base(), kind: "user", text: `Step ${i + 1}`, source: "web" },
        { ...base(), kind: "turn", durationMs: 1000, ok: true },
      ]).flat(),
    ),
  );

const list = (t: ReturnType<typeof turns>, at: number) => (
  <TurnList turns={t} facts={(x) => factsOf(x)} at={at} live={false} hasMoreBefore={false} onGo={() => {}} />
);
const listed = (c: Element) =>
  [...c.querySelectorAll("button")].map((b) => b.textContent?.match(/Step (\d+)/)?.[1]).filter(Boolean);

describe("the details' list of turns", () => {
  it("lists the newest, and grows back to the turn the reader is in, marked", () => {
    const t = turns(12);
    const { container, rerender } = render(list(t, 11));
    expect(listed(container)).toEqual(["7", "8", "9", "10", "11", "12"]);
    // The reader goes back to the fourth turn: it shows, as the one they are in.
    rerender(list(t, 3));
    expect(listed(container)[0]).toBe("4");
    expect(container.querySelector('[aria-current="true"]')?.textContent).toContain("Step 4");
    expect(container.textContent).toContain("Show 3 earlier turns");
  });

  it("lists every turn of a short session", () => {
    const { container } = render(list(turns(8), 7));
    expect(listed(container)).toHaveLength(8);
    expect(container.textContent).not.toContain("earlier");
  });
});

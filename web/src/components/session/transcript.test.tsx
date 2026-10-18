import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Stall } from "@/lib/stall";
import { emptyLive, type Item } from "@/lib/transcript";
import { Transcript } from "./transcript";

vi.mock("@/components/ui", () => ({
  Avatar: () => null,
  Spinner: () => <svg className="animate-spin" />,
}));
vi.mock("@/components/markdown", () => ({
  CodeBlock: () => null,
  Markdown: ({ text }: { text: string }) => <p>{text}</p>,
}));
vi.mock("./attachments", () => ({ MessageFiles: () => null }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => act(() => root.unmount()));
  return container;
}

let seq = 0;
const tool = (result?: string): Item => ({
  kind: "tool",
  key: `tool-${++seq}`,
  seq,
  ts: 0,
  id: `toolu_${seq}`,
  name: "Bash",
  input: { command: "ls", description: "List files" },
  result: result === undefined ? undefined : { text: result, isError: false },
  children: 0,
});
const turn = (): Item => ({ kind: "turn", key: `turn-${++seq}`, seq, ts: 0, durationMs: 1000, ok: true });

const stall: Stall = {
  kind: "away",
  runner: "now",
  status: "running",
  since: new Date(2026, 9, 18, 2, 26).getTime(),
  showFrom: 0,
};

function transcript(items: Item[], status: "running" | "idle", withStall?: Stall) {
  return render(
    <Transcript
      sessionId="ses_1"
      items={items}
      live={emptyLive()}
      status={status}
      stall={withStall}
      onLoadOlder={() => {}}
      hasMoreBefore={false}
      loadingOlder={false}
    />,
  );
}

const spinners = (c: HTMLElement) => c.querySelectorAll(".animate-spin").length;

describe("the transcript", () => {
  it("shows a tool call of the turn in progress as running", () => {
    const c = transcript([tool("done"), tool()], "running");
    expect(spinners(c)).toBe(1);
    expect(c.textContent).toContain("Working");
  });

  it("never shows a tool call of a turn that ended as running, even without its result", () => {
    // A crash, or a result that never reached the control plane.
    const c = transcript([tool(), turn(), tool("done")], "running");
    expect(spinners(c)).toBe(0);
    expect(transcript([tool()], "idle").textContent).not.toContain("Working");
    expect(spinners(transcript([tool()], "idle"))).toBe(0);
  });

  it("says why the agent's work isn't coming through instead of showing it in progress", () => {
    const c = transcript([tool()], "running", stall);
    expect(spinners(c)).toBe(0);
    expect(c.textContent).not.toContain("Working");
    const [title, detail] = [...c.querySelectorAll("[role=status] p")].map((p) => p.textContent);
    expect(title).toMatch(/^Runner now hasn't been heard from since .+\.$/);
    expect(detail).toMatch(/^The agent's work shows up here when it's back/);
  });
});

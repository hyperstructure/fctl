import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { Reveal, Revealer } from "./find";
import { RetrospectiveCard } from "./retrospective";

vi.mock("@/components/markdown", () => ({
  Markdown: ({ text }: { text: string }) => <p>{text}</p>,
}));

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
  return container;
}

const retro = (over: Partial<G.Retrospective> = {}): G.Retrospective => ({
  id: "rtr_1",
  project_id: "prj_1",
  session: { id: "ses_1", number: 12, title: "Fix the login test" },
  created_at: 10,
  outcome: "completed",
  worked_on: "Made the login test wait for the redirect (#41).",
  approach: "Waited on the **URL**, not a timer:\n\n- it is what the user sees",
  challenges: "CI's runners are slower than a laptop, and `wait_for` needed a longer timeout.",
  insights: "Run `npm test -- --repeat 20` before calling a test fixed.",
  ...over,
});

const toggle = (c: Element) => c.querySelector<HTMLButtonElement>("section > button[aria-expanded]");

describe("a retrospective in the transcript", () => {
  it("shows each part in a line until it opens to all it says, and closes again", () => {
    const c = render(<RetrospectiveCard r={retro()} findKey="n-7" />);
    expect(toggle(c)?.getAttribute("aria-expanded")).toBe("false");
    // The gist: each part's words in a line, without Markdown's marks; nothing for finding yet.
    expect(c.textContent).toContain("Waited on the URL, not a timer: it is what the user sees");
    expect(c.textContent).toContain("Run npm test -- --repeat 20 before calling a test fixed.");
    expect(c.textContent).toContain("and wait_for needed a longer timeout.");
    expect(c.querySelector("[data-find]")).toBeNull();
    act(() => toggle(c)?.click());
    expect(toggle(c)?.getAttribute("aria-expanded")).toBe("true");
    expect(c.querySelector('[data-find="retro:n-7:approach"]')?.textContent).toContain("**URL**");
    expect(c.textContent).toContain("Show less");
    act(() => toggle(c)?.click());
    expect(c.querySelector("[data-find]")).toBeNull();
  });

  it("opens from anywhere on its gist", () => {
    const c = render(<RetrospectiveCard r={retro()} findKey="n-7" />);
    act(() => c.querySelector<HTMLButtonElement>('button[tabindex="-1"]')?.click());
    expect(toggle(c)?.getAttribute("aria-expanded")).toBe("true");
  });

  it("marks what an update changed, and the outcome it moved from", () => {
    const before = retro({ outcome: "partial", challenges: "CI is slow." });
    const after = retro({ updated_at: 20 });
    const c = render(<RetrospectiveCard r={after} before={before} findKey="n-9" />);
    expect(c.querySelector("section")?.getAttribute("aria-label")).toBe("Retrospective updated");
    expect(c.textContent).toContain("Partly done");
    expect(c.textContent).toContain("Completed");
    // Only the challenges changed.
    const marked = [...c.querySelectorAll(".sr-only")].filter((s) => s.textContent === " (changed)");
    expect(marked).toHaveLength(1);
    expect(marked[0]?.parentElement?.textContent).toContain("Challenges");
  });

  it("opens to a match finding goes to in it", () => {
    const reveal = new Revealer();
    const c = render(
      <Reveal.Provider value={reveal}>
        <RetrospectiveCard r={retro()} findKey="n-7" />
      </Reveal.Provider>,
    );
    act(() => reveal.set(new Map([["retro:n-7", "open"]])));
    expect(c.querySelector('[data-find="retro:n-7:insights"]')).not.toBeNull();
  });
});

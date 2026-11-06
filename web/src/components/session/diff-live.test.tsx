import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Rolling, UnseenChanges, useLive } from "./diff-live";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: React.ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { container, root };
}

/** The diffs' scrolling box 400 px tall at 100 px from the top, and where each row is down the diffs. */
const BOX = { top: 100, height: 400 };
const HEADER = 40;
const ROW = 20;

/**
 * A file's section of 100 rows under its 40 px header, laid out by hand
 * (the test's DOM lays nothing out): each row as far down the diffs as
 * `at` says, and on the screen as far as the box scrolled.
 */
function layout(body: HTMLElement) {
  const at = new Map<Element, number>();
  const rect = (top: number, height: number) =>
    ({ top, bottom: top + height, height, left: 0, right: 800, width: 800, x: 0, y: top }) as DOMRect;
  body.getBoundingClientRect = () => rect(BOX.top, BOX.height);
  const section = body.querySelector("section") as HTMLElement;
  const rows = () => [...section.querySelectorAll<HTMLElement>(".diff-line")];
  const place = () => {
    for (const [i, r] of rows().entries()) {
      if (!at.has(r)) at.set(r, HEADER + i * ROW);
      r.getBoundingClientRect = () => rect(BOX.top + (at.get(r) ?? 0) - body.scrollTop, ROW);
    }
    section.getBoundingClientRect = () => rect(BOX.top - body.scrollTop, HEADER + rows().length * ROW);
  };
  place();
  /** `n` rows come above row `from`: those from it down move down. */
  const insert = (from: number, n: number) => {
    for (const [r, top] of at) if (top >= HEADER + from * ROW) at.set(r, top + n * ROW);
  };
  return { place, insert, rows };
}

function Harness({ rows, onLive }: { rows: number; onLive: (l: ReturnType<typeof useLive>) => void }) {
  const body = useRef<HTMLDivElement>(null);
  const column = useRef<HTMLDivElement>(null);
  const live = useLive(body, column, HEADER);
  onLive(live);
  return (
    <div ref={body} data-body="">
      <div ref={column}>
        <section data-file="0">
          {Array.from({ length: rows }, (_, i) => (
            <div key={i} className="diff-line" data-l={`0:${i}`} />
          ))}
        </section>
      </div>
    </div>
  );
}

function setup() {
  let live: ReturnType<typeof useLive> | undefined;
  const { container } = render(<Harness rows={100} onLive={(l) => (live = l)} />);
  const body = container.querySelector("[data-body]") as HTMLElement;
  const fake = layout(body);
  const scroll = (top: number) =>
    act(() => {
      body.scrollTop = top;
      body.dispatchEvent(new Event("scroll"));
    });
  const top = (i: number) => fake.rows()[i]?.getBoundingClientRect().top;
  return { body, fake, scroll, top, live: () => live as ReturnType<typeof useLive> };
}

describe("the reader's place in the diffs", () => {
  it("holds the line at the top of the view as rows come above it", () => {
    const { body, fake, scroll, top, live } = setup();
    scroll(500);
    // The row under the header at the top of the view: row 25, 60 px down it.
    expect(top(25)).toBe(140);
    fake.insert(10, 3);
    live().view.hold();
    expect(body.scrollTop).toBe(560);
    expect(top(25)).toBe(140);
  });

  it("holds it at the very top of the diffs too, what comes above out of sight", () => {
    const { body, fake, scroll, top, live } = setup();
    scroll(0);
    expect(top(0)).toBe(140);
    fake.insert(0, 3);
    live().view.hold();
    expect(body.scrollTop).toBe(60);
    expect(top(0)).toBe(140);
  });

  it("finds the line again once its row is drawn anew", () => {
    const { body, fake, scroll, top, live } = setup();
    scroll(500);
    const old = fake.rows()[25] as HTMLElement;
    const again = old.cloneNode() as HTMLElement;
    old.replaceWith(again);
    fake.place();
    fake.insert(0, 2);
    live().view.hold();
    expect(body.scrollTop).toBe(540);
    expect(top(25)).toBe(140);
  });

  it("counts the changes out of sight, and lights one up as the reader comes to it", async () => {
    const { body, fake, scroll, live } = setup();
    scroll(0);
    const row = fake.rows()[60] as HTMLElement;
    act(() => live().view.unseen([[row]]));
    await act(() => new Promise((r) => requestAnimationFrame(() => r(undefined))));
    expect(live().counts).toEqual({ above: 0, below: 1 });
    expect(live().nearest("below")).toBe(row);
    scroll(1000);
    await act(() => new Promise((r) => requestAnimationFrame(() => r(undefined))));
    expect(live().counts).toEqual({ above: 0, below: 0 });
    expect(row.dataset.play).toBe("seen");
    expect(body.scrollTop).toBe(1000);
  });
});

describe("the button to changes out of sight", () => {
  it("says how many there are and where, and goes to them", () => {
    let went = 0;
    const { container } = render(<UnseenChanges dir="below" count={3} onGo={() => went++} />);
    const button = container.querySelector("button") as HTMLButtonElement;
    expect(button.textContent).toBe("3 new changes below");
    act(() => button.click());
    expect(went).toBe(1);
  });
});

describe("a rolling number", () => {
  it("rolls from the number before to the new one", () => {
    const { container, root } = render(<Rolling value={3} />);
    expect(container.textContent).toBe("3");
    act(() => root.render(<Rolling value={5} />));
    // Both, for a moment: the one before rolls away, the new one rolls in.
    expect(container.textContent).toBe("35");
    expect(container.querySelector('[aria-hidden="true"]')?.textContent).toBe("3");
  });
});

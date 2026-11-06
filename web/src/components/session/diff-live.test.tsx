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
 * Two files' sections of 100 rows each under their 40 px headers, laid out
 * by hand (the test's DOM lays nothing out): each row as far down the diffs
 * as `at` says, and on the screen as far as the box scrolled.
 */
function layout(body: HTMLElement) {
  const at = new Map<Element, number>();
  const rect = (top: number, height: number) =>
    ({ top, bottom: top + height, height, left: 0, right: 800, width: 800, x: 0, y: top }) as DOMRect;
  body.getBoundingClientRect = () => rect(BOX.top, BOX.height);
  const sections = () => [...body.querySelectorAll<HTMLElement>("section")];
  const rows = (s = 0) => [...(sections()[s]?.querySelectorAll<HTMLElement>(".diff-line") ?? [])];
  const place = () => {
    let y = 0;
    for (const section of sections()) {
      const top = y;
      if (!at.has(section)) at.set(section, top);
      y += HEADER;
      for (const r of section.querySelectorAll<HTMLElement>(".diff-line")) {
        if (!at.has(r)) at.set(r, y);
        y += ROW;
        r.getBoundingClientRect = () => rect(BOX.top + (at.get(r) ?? 0) - body.scrollTop, ROW);
      }
      const height = y - top;
      section.getBoundingClientRect = () => rect(BOX.top + (at.get(section) ?? 0) - body.scrollTop, height);
      Object.defineProperty(section, "offsetTop", { configurable: true, get: () => at.get(section) ?? 0 });
    }
  };
  place();
  /** `n` rows come above row `from` of the first file: everything from it down moves down. */
  const insert = (from: number, n: number) => {
    for (const [el, top] of at) if (top >= HEADER + from * ROW) at.set(el, top + n * ROW);
  };
  return { place, insert, rows, sections };
}

function Harness({
  onLive,
  pinned,
}: {
  onLive: (l: ReturnType<typeof useLive>) => void;
  pinned?: { current: number | undefined };
}) {
  const body = useRef<HTMLDivElement>(null);
  const column = useRef<HTMLDivElement>(null);
  const live = useLive(body, column, HEADER, () =>
    pinned?.current === undefined
      ? undefined
      : body.current?.querySelector<HTMLElement>(`section[data-file="${pinned.current}"]`),
  );
  onLive(live);
  return (
    <div ref={body} data-body="">
      <div ref={column}>
        {[0, 1].map((f) => (
          <section key={f} data-file={f}>
            {Array.from({ length: 100 }, (_, i) => (
              <div key={i} className="diff-line" data-l={`0:${i}`} />
            ))}
          </section>
        ))}
      </div>
    </div>
  );
}

function setup(pinned?: { current: number | undefined }) {
  let live: ReturnType<typeof useLive> | undefined;
  const node = () => <Harness pinned={pinned} onLive={(l) => (live = l)} />;
  const { container, root } = render(node());
  // Drawn again, as the dialog is with each new list of files: with a new `pinned` each time.
  const again = () => act(() => root.render(node()));
  const body = container.querySelector("[data-body]") as HTMLElement;
  const fake = layout(body);
  const scroll = (top: number) =>
    act(() => {
      body.scrollTop = top;
      body.dispatchEvent(new Event("scroll"));
    });
  const top = (i: number, s = 0) => fake.rows(s)[i]?.getBoundingClientRect().top;
  return { body, fake, scroll, top, again, live: () => live as ReturnType<typeof useLive> };
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

  it("keeps the file the reader went to at the top until they scroll, as the dialog gets its size too", () => {
    const pinned = { current: 1 as number | undefined };
    const { body, fake, live, scroll } = setup(pinned);
    // The second file starts below the first's 100 rows.
    live().view.hold();
    expect(body.scrollTop).toBe(HEADER + 100 * ROW);
    fake.insert(10, 3);
    live().view.hold();
    expect(body.scrollTop).toBe(HEADER + 103 * ROW);
    // Once they scroll, it is their place that holds.
    pinned.current = undefined;
    scroll(500);
    fake.insert(0, 1);
    live().view.hold();
    expect(body.scrollTop).toBe(520);
  });

  it("doesn't take the same line of another file for the line it held, once its own file went", () => {
    const { body, fake, scroll, live } = setup();
    scroll(500);
    // The file goes (the agent put it back as it was); the next one takes its place in the list.
    const [first, second] = fake.sections();
    first?.remove();
    second?.setAttribute("data-file", "0");
    live().view.hold();
    expect(body.scrollTop).toBe(500);
  });

  it("holds its place through a render that comes before it held, with a new list of files", () => {
    const { body, fake, scroll, top, again, live } = setup();
    scroll(500);
    fake.insert(10, 3);
    again();
    live().view.hold();
    expect(body.scrollTop).toBe(560);
    expect(top(25)).toBe(140);
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

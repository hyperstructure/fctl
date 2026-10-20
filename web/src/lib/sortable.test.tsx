import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import {
  type Box,
  gapOf,
  limitsOf,
  moveItem,
  scrollSpeed,
  shiftOf,
  targetIndex,
  useSortable,
} from "./sortable";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// Rows 32 px tall, 2 px apart, from y = 100: middles at 116, 150, 184, 218.
const rows: Box[] = [0, 1, 2, 3].map((i) => ({ top: 100 + i * 34, height: 32 }));

describe("targetIndex", () => {
  it("stays put until the leading edge passes a neighbour's middle", () => {
    expect(targetIndex(rows, 0, 100)).toBe(0);
    // Its bottom at 150, the middle of the second row: not past it yet.
    expect(targetIndex(rows, 0, 118)).toBe(0);
    expect(targetIndex(rows, 0, 119)).toBe(1);
    expect(targetIndex(rows, 3, 202)).toBe(3);
    expect(targetIndex(rows, 3, 183)).toBe(2);
  });

  it("passes as many as it crosses, down or up", () => {
    expect(targetIndex(rows, 0, 187)).toBe(3);
    expect(targetIndex(rows, 3, 100)).toBe(0);
    expect(targetIndex(rows, 1, 115)).toBe(0);
    expect(targetIndex(rows, 1, 153)).toBe(2);
  });

  it("goes no further than the ends", () => {
    expect(targetIndex(rows, 1, -500)).toBe(0);
    expect(targetIndex(rows, 1, 900)).toBe(3);
  });

  it("works with items of different heights, as workspaces with their projects", () => {
    // A workspace of three rows, one folded, one of two rows; 12 px apart.
    const sections: Box[] = [
      { top: 0, height: 100 },
      { top: 112, height: 32 },
      { top: 156, height: 66 },
    ];
    // Moving the first down: its bottom passes the folded one's middle (128) at top 29.
    expect(targetIndex(sections, 0, 28)).toBe(0);
    expect(targetIndex(sections, 0, 29)).toBe(1);
    // The last up: its top passes the folded one's middle at 127.
    expect(targetIndex(sections, 2, 128)).toBe(2);
    expect(targetIndex(sections, 2, 127)).toBe(1);
  });
});

describe("shiftOf", () => {
  it("moves the ones it passed by its height and the gap, toward where it came from", () => {
    expect(gapOf(rows)).toBe(2);
    // The first over the third: the second and third move up.
    expect([0, 1, 2, 3].map((i) => shiftOf(rows, 0, 2, i))).toEqual([0, -34, -34, 0]);
    // The last over the second: the second and third move down.
    expect([0, 1, 2, 3].map((i) => shiftOf(rows, 3, 1, i))).toEqual([0, 34, 34, 0]);
    expect([0, 1, 2, 3].map((i) => shiftOf(rows, 2, 2, i))).toEqual([0, 0, 0, 0]);
  });
});

describe("limitsOf", () => {
  it("keeps a dragged item within its list", () => {
    expect(limitsOf(rows, 0)).toEqual([0, 102]);
    expect(limitsOf(rows, 2)).toEqual([-68, 34]);
    expect(limitsOf([{ top: 10, height: 20 }], 0)).toEqual([0, 0]);
  });
});

describe("scrollSpeed", () => {
  // A list from y = 100 to 500 whose items may move 300 px either way.
  const limits = [-300, 300] as const;

  it("scrolls toward the edge the item is dragged to, faster closer to it", () => {
    expect(scrollSpeed(300, 200, 100, 500, 100, limits)).toBe(0);
    expect(scrollSpeed(476, 200, 100, 500, 276, limits)).toBe(7);
    expect(scrollSpeed(520, 200, 100, 500, 280, limits)).toBe(14);
    expect(scrollSpeed(124, 300, 100, 500, -176, limits)).toBe(-7);
  });

  it("doesn't scroll away from where the item is going", () => {
    // Picked up near the bottom, dragged up.
    expect(scrollSpeed(470, 480, 100, 500, -10, limits)).toBe(0);
    expect(scrollSpeed(110, 100, 100, 500, 10, limits)).toBe(0);
  });

  it("stops once the item can go no further that way", () => {
    expect(scrollSpeed(490, 200, 100, 500, 300, limits)).toBe(0);
    expect(scrollSpeed(110, 400, 100, 500, -300, limits)).toBe(0);
  });
});

describe("moveItem", () => {
  it("moves one item and keeps the others in order", () => {
    expect(moveItem(["a", "b", "c", "d"], 0, 2)).toEqual(["b", "c", "a", "d"]);
    expect(moveItem(["a", "b", "c", "d"], 3, 0)).toEqual(["d", "a", "b", "c"]);
    expect(moveItem(["a", "b"], 1, 1)).toEqual(["a", "b"]);
  });
});

// A list of links people sort, laid out as the sidebar's rows: 32 px tall, 2 px apart, from y = 100.
function List({ initial, onReorder }: { initial: string[]; onReorder: (ids: string[]) => void }) {
  const [ids, setIds] = useState(initial);
  const sort = useSortable({
    ids,
    onReorder: (next) => {
      onReorder(next);
      setIds(next);
    },
    label: (id) => id.toUpperCase(),
    describedBy: "help",
  });
  return (
    <div style={{ overflowY: "auto" }} data-scroller="">
      {ids.map((id) => (
        <a
          key={id}
          href={`#${id}`}
          ref={sort.itemRef(id)}
          {...sort.handleProps(id)}
          data-row=""
          data-lifted={sort.lifted === id ? "" : undefined}
        >
          {id}
        </a>
      ))}
    </div>
  );
}

const rect = HTMLElement.prototype.getBoundingClientRect;
beforeEach(() => {
  vi.useFakeTimers();
  // Where a row shows: its place in the list, moved by its transform.
  vi.spyOn(HTMLElement.prototype, "getBoundingClientRect").mockImplementation(function (this: HTMLElement) {
    // The list scrolls within the window.
    if (this.hasAttribute("data-scroller")) return { top: 0, bottom: 768, height: 768 } as DOMRect;
    if (!this.hasAttribute("data-row")) return rect.call(this);
    const index = [...(this.parentElement?.children ?? [])].indexOf(this);
    const shift = Number(/translateY\((-?[\d.]+)px\)/.exec(this.style.transform)?.[1] ?? 0);
    const top = 100 + index * 34 + shift;
    return { top, bottom: top + 32, height: 32, left: 0, right: 200, width: 200, x: 0, y: top } as DOMRect;
  });
});

const unmounts: (() => void)[] = [];
afterEach(() => {
  for (const unmount of unmounts.splice(0)) unmount();
  vi.restoreAllMocks();
  vi.useRealTimers();
});

function render(initial: string[]) {
  const onReorder = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<List initial={initial} onReorder={onReorder} />));
  unmounts.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const row = (id: string) => container.querySelector<HTMLElement>(`a[href="#${id}"]`) as HTMLElement;
  const order = () => [...container.querySelectorAll("a")].map((a) => a.textContent);
  return { onReorder, row, order };
}

const pointer = (target: EventTarget, type: string, y: number, pointerType = "mouse") =>
  act(() => {
    target.dispatchEvent(
      new PointerEvent(type, {
        bubbles: true,
        cancelable: true,
        clientX: 50,
        clientY: y,
        button: 0,
        pointerId: 1,
        pointerType,
      }),
    );
  });

describe("useSortable", () => {
  it("moves the focused item with Alt and an arrow, keeping its focus", () => {
    const { onReorder, row, order } = render(["a", "b", "c"]);
    row("a").focus();
    act(() => {
      row("a").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true }));
    });
    expect(onReorder).toHaveBeenCalledWith(["b", "a", "c"]);
    expect(order()).toEqual(["b", "a", "c"]);
    expect(document.activeElement).toBe(row("a"));
    expect(document.querySelector("[aria-live]")?.textContent).toBe("Moved A to 2 of 3");
    // Not past the ends, and not without Alt.
    act(() => {
      row("b").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowUp", altKey: true, bubbles: true }));
      row("c").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true }));
      row("a").dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", bubbles: true }));
    });
    expect(onReorder).toHaveBeenCalledTimes(1);
    expect(row("a").getAttribute("aria-describedby")).toBe("help");
  });

  it("moves on from the new order when keys come faster than it shows", () => {
    const onReorder = vi.fn();
    // A list that takes its order from elsewhere, and shows the new one later.
    function Slow() {
      const [ids, setIds] = useState(["a", "b", "c"]);
      const sort = useSortable({
        ids,
        onReorder: (next) => {
          onReorder(next);
          setTimeout(() => setIds(next), 50);
        },
        label: (id) => id,
      });
      return (
        <div>
          {ids.map((id) => (
            <a key={id} href={`#${id}`} ref={sort.itemRef(id)} {...sort.handleProps(id)} data-row="">
              {id}
            </a>
          ))}
        </div>
      );
    }
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    act(() => root.render(<Slow />));
    unmounts.push(() => {
      act(() => root.unmount());
      container.remove();
    });
    const a = container.querySelector('a[href="#a"]') as HTMLElement;
    act(() => {
      for (let i = 0; i < 2; i++)
        a.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowDown", altKey: true, bubbles: true }));
    });
    expect(onReorder).toHaveBeenLastCalledWith(["b", "c", "a"]);
    act(() => vi.advanceTimersByTime(100));
    expect([...container.querySelectorAll("a")].map((x) => x.textContent)).toEqual(["b", "c", "a"]);
  });

  it("drags with a mouse once it moves, and the click that ends it opens nothing", () => {
    const { onReorder, row, order } = render(["a", "b", "c"]);
    pointer(row("a"), "pointerdown", 116);
    pointer(window, "pointermove", 118);
    expect(row("a").hasAttribute("data-lifted")).toBe(false);
    // Down 44 px: its bottom passes the middle of b, not of c.
    pointer(window, "pointermove", 160);
    expect(row("a").hasAttribute("data-lifted")).toBe(true);
    expect(row("a").style.transform).toBe("translateY(44px)");
    expect(row("b").style.transform).toBe("translateY(-34px)");
    expect(row("c").style.transform).toBe("");
    pointer(window, "pointerup", 160);
    expect(onReorder).toHaveBeenCalledWith(["b", "a", "c"]);
    expect(order()).toEqual(["b", "a", "c"]);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    act(() => {
      row("a").dispatchEvent(click);
    });
    expect(click.defaultPrevented).toBe(true);
    act(() => vi.advanceTimersByTime(300));
    expect(row("a").hasAttribute("data-lifted")).toBe(false);
  });

  it("keeps a dragged item within the list", () => {
    const { onReorder, row } = render(["a", "b", "c"]);
    pointer(row("b"), "pointerdown", 150);
    pointer(window, "pointermove", 60);
    expect(row("b").style.transform).toBe("translateY(-34px)");
    pointer(window, "pointerup", 60);
    expect(onReorder).toHaveBeenCalledWith(["b", "a", "c"]);
  });

  it("lets a click through, and puts a drag back on Escape", () => {
    const { onReorder, row, order } = render(["a", "b", "c"]);
    pointer(row("a"), "pointerdown", 116);
    pointer(window, "pointerup", 116);
    const click = new MouseEvent("click", { bubbles: true, cancelable: true });
    row("a").addEventListener("click", (e) => e.preventDefault()); // no navigation in the test
    act(() => {
      row("a").dispatchEvent(click);
    });
    expect(row("a").hasAttribute("data-lifted")).toBe(false);

    pointer(row("a"), "pointerdown", 116);
    pointer(window, "pointermove", 200);
    act(() => {
      window.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    pointer(window, "pointerup", 200);
    act(() => vi.advanceTimersByTime(300));
    expect(onReorder).not.toHaveBeenCalled();
    expect(order()).toEqual(["a", "b", "c"]);
    expect(row("a").style.transform).toBe("");
    expect(row("b").style.transform).toBe("");
  });

  it("lifts under a finger that holds still, and lets a swipe scroll", () => {
    const { onReorder, row } = render(["a", "b", "c"]);
    pointer(row("c"), "pointerdown", 184, "touch");
    pointer(window, "pointermove", 160, "touch");
    act(() => vi.advanceTimersByTime(400));
    expect(row("c").hasAttribute("data-lifted")).toBe(false);
    pointer(window, "pointerup", 160, "touch");

    pointer(row("c"), "pointerdown", 184, "touch");
    act(() => vi.advanceTimersByTime(400));
    expect(row("c").hasAttribute("data-lifted")).toBe(true);
    pointer(window, "pointermove", 110, "touch");
    pointer(window, "pointerup", 110, "touch");
    expect(onReorder).toHaveBeenCalledWith(["c", "a", "b"]);
  });

  it("doesn't sort a list of one", () => {
    const { onReorder, row } = render(["a"]);
    pointer(row("a"), "pointerdown", 116);
    pointer(window, "pointermove", 160);
    pointer(window, "pointerup", 160);
    expect(onReorder).not.toHaveBeenCalled();
    expect(row("a").hasAttribute("aria-describedby")).toBe(false);
  });
});

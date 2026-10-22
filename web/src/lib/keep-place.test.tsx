import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { useKeepPlace } from "./keep-place";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.restoreAllMocks();
});

/**
 * A page 800 px tall with a list of items 100 px tall each, under a 50 px
 * button, scrolled to `top`. `show` lays out the list again: older items
 * push the others down, as the browser would. Measuring lays the page out,
 * and the browser's scroll anchoring moves it, roughly as Chrome's does.
 */
function transcriptPage() {
  let top = 0;
  Object.defineProperty(window, "innerHeight", { configurable: true, get: () => 800 });
  Object.defineProperty(window, "scrollY", { configurable: true, get: () => top });
  cleanups.push(() => {
    for (const k of ["innerHeight", "scrollY"]) Reflect.deleteProperty(window, k);
  });
  const scrollBy = vi.spyOn(window, "scrollBy").mockImplementation(((o: ScrollToOptions) => {
    top += o.top ?? 0;
  }) as typeof window.scrollBy);
  const container = document.createElement("div");
  document.body.append(container);
  const y = (el: Element) => Number((el as HTMLElement).dataset.y ?? 0);
  // What the browser anchors to: the first thing in view in the list (the
  // button or an item), where it was at the layout before.
  let anchor: { el: Element; y: number } | undefined;
  const layOut = () => {
    const l = container.firstElementChild as HTMLElement | null;
    if (!l) return;
    // With the list's anchoring on, the page moves with what it anchors to.
    if (anchor?.el.isConnected && l.style.overflowAnchor !== "none") top += y(anchor.el) - anchor.y;
    const first = [...l.children].find((c) => y(c) - top + 100 > 0);
    anchor = first && { el: first, y: y(first) };
  };
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    layOut();
    const at = y(this) - top;
    return { top: at, bottom: at + 100, left: 0, right: 0, width: 0, height: 100 } as DOMRect;
  });

  const root = createRoot(container);
  let keep: () => void = () => {};
  function List({ items, loading }: { items: string[]; loading: boolean }) {
    const list = useRef<HTMLDivElement>(null);
    keep = useKeepPlace(list, loading);
    return (
      <div ref={list}>
        <button type="button" data-y={0}>
          Show earlier activity
        </button>
        {items.map((it, i) => (
          <div key={it} data-item data-y={50 + i * 100}>
            {it}
          </div>
        ))}
      </div>
    );
  }
  const show = (items: string[], loading = false) =>
    act(() => root.render(<List items={items} loading={loading} />));
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const list = () => container.firstElementChild as HTMLElement;
  /** Where an item is in the view. */
  const at = (name: string) =>
    [...list().children].find((c) => c.textContent === name)?.getBoundingClientRect().top;
  return {
    show,
    keep: () => keep(),
    at,
    list,
    scrollBy,
    scroll: (to: number) => {
      top = to;
    },
  };
}

const items = (from: number, to: number) => Array.from({ length: to - from }, (_, i) => `item ${from + i}`);

describe("keeping the reader's place", () => {
  it("leaves what is in view where it is as older items go in above it", () => {
    const p = transcriptPage();
    p.show(items(400, 800));
    p.scroll(20);
    const before = p.at("item 400");
    p.keep();
    p.show(items(400, 800), true);
    // Without it, the reader would land on the oldest of them.
    p.show(items(0, 800));
    expect(p.at("item 400")).toBe(before);
    expect(p.scrollBy).toHaveBeenCalledTimes(1);
  });

  it("follows the reader's own scrolling while they load", () => {
    const p = transcriptPage();
    p.show(items(400, 800));
    p.keep();
    p.show(items(400, 800), true);
    p.scroll(300);
    const before = p.at("item 405");
    p.show(items(0, 800));
    expect(p.at("item 405")).toBe(before);
  });

  it("keeps it when the button is out of view, as when the load starts ahead of the reader", () => {
    const p = transcriptPage();
    p.show(items(400, 800));
    // The button is 200 px above the view: the browser anchors to an item.
    p.scroll(250);
    const before = p.at("item 402");
    p.keep();
    p.show(items(400, 800), true);
    p.show(items(0, 800));
    // Not twice what went in above, once by the browser and once by the list.
    expect(p.at("item 402")).toBe(before);
    expect(p.scrollBy).toHaveBeenCalledTimes(1);
  });

  it("measures from an item that is still there when the first ones merge with older ones", () => {
    const p = transcriptPage();
    p.show(items(400, 800));
    const before = p.at("item 402");
    p.keep();
    p.show(items(400, 800), true);
    // The first two became part of older ones.
    p.show([...items(0, 400), ...items(402, 800)]);
    expect(p.at("item 402")).toBe(before);
  });

  it("turns the browser's anchoring off in the list until they are in", () => {
    const p = transcriptPage();
    p.show(items(400, 800));
    p.keep();
    p.show(items(400, 800), true);
    expect(p.list().style.overflowAnchor).toBe("none");
    p.show(items(0, 800));
    expect(p.list().style.overflowAnchor).toBe("");
  });

  it("leaves the page alone when nothing loads, and when nobody asked", () => {
    const p = transcriptPage();
    p.show(items(400, 800));
    p.keep();
    p.show(items(400, 800), true);
    // The request failed.
    p.show(items(400, 800));
    expect(p.list().style.overflowAnchor).toBe("");
    // Older items without a press, which can't happen today, and new ones at the end.
    p.show(items(0, 800));
    p.show(items(0, 801));
    expect(p.scrollBy).not.toHaveBeenCalled();
  });
});

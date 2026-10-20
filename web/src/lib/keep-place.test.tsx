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
 * push the others down, as the browser would.
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
  vi.spyOn(Element.prototype, "getBoundingClientRect").mockImplementation(function (this: Element) {
    const y = Number((this as HTMLElement).dataset.y ?? 0) - top;
    return { top: y, bottom: y + 100, left: 0, right: 0, width: 0, height: 100 } as DOMRect;
  });

  const container = document.createElement("div");
  document.body.append(container);
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

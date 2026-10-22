import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { AHEAD, useLoadInView } from "./load-in-view";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** The browser's observers: the test says when the button comes near the view and goes. */
class Observer {
  static all: Observer[] = [];
  watched?: Element;
  connected = true;
  constructor(
    readonly report: (entries: { isIntersecting: boolean }[]) => void,
    readonly options: IntersectionObserverInit,
  ) {
    Observer.all.push(this);
  }
  observe(el: Element) {
    this.watched = el;
  }
  disconnect() {
    this.connected = false;
  }
}

/** The button comes within `AHEAD` px of the view (true) or leaves it (false). */
const near = (seen: boolean) =>
  act(() => {
    for (const o of Observer.all) if (o.connected && o.watched) o.report([{ isIntersecting: seen }]);
  });
const watching = () => Observer.all.filter((o) => o.connected && o.watched);

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.unstubAllGlobals();
  Observer.all = [];
});

/** The button, as the transcript has it while there is earlier activity. */
function earlier() {
  vi.stubGlobal("IntersectionObserver", Observer);
  const load = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  function Button({ loading, failed }: { loading: boolean; failed: boolean }) {
    const button = useRef<HTMLButtonElement>(null);
    useLoadInView(button, load, loading, failed);
    return (
      <button ref={button} type="button">
        Show earlier activity
      </button>
    );
  }
  const show = ({ loading = false, failed = false } = {}) =>
    act(() => root.render(<Button loading={loading} failed={failed} />));
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { load, show, gone: () => act(() => root.unmount()) };
}

describe("loading earlier activity as the reader scrolls up to it", () => {
  it("loads as the button comes near the view", () => {
    const b = earlier();
    b.show();
    expect(watching()).toHaveLength(1);
    expect(watching()[0]?.options.rootMargin).toBe(`${AHEAD}px 0px`);
    // The page opened at its bottom, far below it.
    near(false);
    expect(b.load).not.toHaveBeenCalled();
    near(true);
    expect(b.load).toHaveBeenCalledTimes(1);
  });

  it("leaves the button alone while a load is under way", () => {
    const b = earlier();
    b.show();
    near(true);
    b.show({ loading: true });
    expect(watching()).toHaveLength(0);
    near(false);
    near(true);
    expect(b.load).toHaveBeenCalledTimes(1);
  });

  it("loads again while a load leaves it in view, until the transcript fills the page", () => {
    const b = earlier();
    b.show();
    near(true);
    b.show({ loading: true });
    b.show();
    // A new observer says where the button is now: still in view.
    near(true);
    expect(b.load).toHaveBeenCalledTimes(2);
    b.show({ loading: true });
    b.show();
    // What came in pushed it up out of view.
    near(false);
    expect(b.load).toHaveBeenCalledTimes(2);
  });

  it("waits for the reader to come back to it after a load that failed", () => {
    const b = earlier();
    b.show();
    near(true);
    b.show({ loading: true });
    // Nothing came: trying again at once would go on as fast as it fails.
    b.show({ failed: true });
    near(true);
    expect(b.load).toHaveBeenCalledTimes(1);
    // The reader scrolls down and up again.
    near(false);
    near(true);
    expect(b.load).toHaveBeenCalledTimes(2);
  });

  it("stops watching when the button goes", () => {
    const b = earlier();
    b.show();
    b.gone();
    expect(watching()).toHaveLength(0);
  });
});

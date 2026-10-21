import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Pull, Sheet } from "./sheet";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.useRealTimers();
  vi.unstubAllGlobals();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
}

/** A phone's screen, or from 768 px up. */
function screen(wide: boolean) {
  vi.stubGlobal("matchMedia", (query: string) => ({ matches: wide, media: query }) as MediaQueryList);
}

function open() {
  screen(false);
  const onClose = vi.fn();
  render(
    <Sheet title="Fix the flaky login test" description="It retries the login once." onClose={onClose}>
      <p>Changes</p>
    </Sheet>,
  );
  const dialog = document.querySelector("dialog") as HTMLDialogElement;
  // happy-dom lays nothing out: a sheet 600 px tall.
  Object.defineProperty(dialog, "offsetHeight", { value: 600 });
  const title = dialog.querySelector("h2") as HTMLElement;
  // What scrolls: around the contents.
  const body = dialog.lastElementChild as HTMLElement;
  const closing = () => dialog.hasAttribute("data-closing");
  /** It finished sliding away. (happy-dom's TransitionEvent leaves out `propertyName`.) */
  const slid = () =>
    act(() => {
      const ended = new Event("transitionend");
      Object.defineProperty(ended, "propertyName", { value: "translate" });
      dialog.dispatchEvent(ended);
    });
  return { onClose, dialog, title, body, closing, slid };
}

/** A finger on `el` from `y0` down to each of `ys`, then off the screen (or `cancel`led). */
function pull(el: Element, y0: number, ys: number[], end: "touchend" | "touchcancel" = "touchend") {
  const touch = (y: number) => new Touch({ identifier: 1, target: el, clientX: 100, clientY: y });
  const moves: TouchEvent[] = [];
  act(() => {
    el.dispatchEvent(new TouchEvent("touchstart", { bubbles: true, touches: [touch(y0)] }));
    for (const y of ys) {
      const move = new TouchEvent("touchmove", { bubbles: true, cancelable: true, touches: [touch(y)] });
      moves.push(move);
      el.dispatchEvent(move);
    }
    el.dispatchEvent(new TouchEvent(end, { bubbles: true, touches: [] }));
  });
  return moves;
}

describe("Sheet", () => {
  it("shows its title, description and contents over the page", () => {
    const { dialog } = open();
    expect(dialog.open).toBe(true);
    const label = document.getElementById(dialog.getAttribute("aria-labelledby") ?? "");
    expect(label?.textContent).toBe("Fix the flaky login test");
    expect(dialog.textContent).toContain("It retries the login once.");
    expect(dialog.textContent).toContain("Changes");
  });

  it("is as tall as what it holds, up to most of the screen, and scrolls the rest", () => {
    const { body } = open();
    const classes = body.className.split(" ");
    // happy-dom lays nothing out: the classes that do. With flex-1's 0% basis, Safari 26
    // gave the contents no height, and the sheet showed its title only.
    expect(classes).toEqual(expect.arrayContaining(["flex-auto", "min-h-0", "overflow-y-auto"]));
    expect(classes).not.toContain("flex-1");
  });

  it("focuses itself as it opens, and holds the page still", () => {
    const html = document.documentElement;
    const { dialog } = open();
    // Safari rings what a dialog focuses as it opens, even after a tap.
    expect(dialog.hasAttribute("autofocus")).toBe(true);
    expect(dialog.className).toMatch(/\boutline-none\b/);
    expect(html.style.overflow).toBe("hidden");
    for (const unmount of roots.splice(0)) unmount();
    expect(html.style.overflow).toBe("");
  });

  it("slides away from the X before it closes", () => {
    const { dialog, onClose, closing, slid } = open();
    act(() => dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')?.click());
    expect(closing()).toBe(true);
    expect(dialog.open).toBe(true);
    expect(onClose).not.toHaveBeenCalled();
    slid();
    expect(dialog.open).toBe(false);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closes even when it can't slide", () => {
    vi.useFakeTimers();
    const { dialog, onClose } = open();
    act(() => dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')?.click());
    act(() => vi.advanceTimersByTime(400));
    expect(dialog.open).toBe(false);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("slides away on Escape too", () => {
    const { dialog, closing } = open();
    const cancel = new Event("cancel", { cancelable: true });
    act(() => {
      dialog.dispatchEvent(cancel);
    });
    // Not at once: the dialog would close without sliding.
    expect(cancel.defaultPrevented).toBe(true);
    expect(closing()).toBe(true);
  });

  it("closes with a click beside it, not one that starts in it", () => {
    const { dialog, title, closing } = open();
    const press = (down: Element, up: Element) =>
      act(() => {
        down.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
        up.dispatchEvent(new MouseEvent("click", { bubbles: true }));
      });
    press(title, title);
    // Selecting the title out to the page.
    press(title, dialog);
    expect(closing()).toBe(false);
    press(dialog, dialog);
    expect(closing()).toBe(true);
  });

  it("follows a finger that pulls it down, and settles back", () => {
    const { dialog, title, closing } = open();
    const touch = (y: number) => new Touch({ identifier: 1, target: title, clientX: 100, clientY: y });
    act(() => {
      title.dispatchEvent(new TouchEvent("touchstart", { bubbles: true, touches: [touch(100)] }));
    });
    const move = new TouchEvent("touchmove", { bubbles: true, cancelable: true, touches: [touch(110)] });
    act(() => {
      title.dispatchEvent(move);
    });
    // The finger moves the sheet, not the page.
    expect(move.defaultPrevented).toBe(true);
    expect(dialog.style.translate).toBe("0 10px");
    expect(dialog.hasAttribute("data-pulling")).toBe(true);
    act(() => {
      title.dispatchEvent(new TouchEvent("touchend", { bubbles: true, touches: [] }));
    });
    expect(closing()).toBe(false);
    expect(dialog.style.translate).toBe("");
    expect(dialog.hasAttribute("data-pulling")).toBe(false);
  });

  it("goes away when pulled far enough", () => {
    const { dialog, title, closing, onClose, slid } = open();
    pull(title, 100, [140, 200, 300]);
    expect(closing()).toBe(true);
    // It slides on from where the finger left it.
    expect(dialog.style.translate).toBe("");
    slid();
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("settles back when the browser takes the touch away", () => {
    const { title, closing } = open();
    pull(title, 100, [140, 200, 300], "touchcancel");
    expect(closing()).toBe(false);
  });

  it("leaves a finger that moves up or sideways to the page", () => {
    const { dialog, title } = open();
    const [up] = pull(title, 300, [280, 200]);
    const [sideways] = pull(title, 300, [302]);
    expect(up?.defaultPrevented).toBe(false);
    expect(dialog.style.translate).toBe("");
    // 2 px down, but the touch moved 0 px sideways: still a pull. A real
    // swipe sideways moves more across than down.
    expect(sideways?.defaultPrevented).toBe(true);
  });

  it("scrolls what it holds, once that is scrolled, instead of pulling", () => {
    const { dialog, body, closing } = open();
    body.scrollTop = 120;
    const [first] = pull(body, 300, [340, 400, 600]);
    expect(first?.defaultPrevented).toBe(false);
    expect(closing()).toBe(false);
    expect(dialog.style.translate).toBe("");
    // At its top, a pull there pulls the sheet.
    body.scrollTop = 0;
    pull(body, 300, [340, 400, 600]);
    expect(closing()).toBe(true);
  });

  it("isn't pulled from 768 px, where it comes in from the right", () => {
    const { title, closing } = open();
    screen(true);
    const [first] = pull(title, 100, [200, 300, 500]);
    expect(first?.defaultPrevented).toBe(false);
    expect(closing()).toBe(false);
  });
});

describe("Pull", () => {
  it("follows the finger down, and not up past where it started", () => {
    const p = new Pull(200, 0);
    p.moved(260, 16);
    expect(p.distance).toBe(60);
    p.moved(150, 32);
    expect(p.distance).toBe(0);
  });

  it("puts the sheet away once pulled down a quarter of its height", () => {
    // Slowly: 10 px every 100 ms.
    const slow = (to: number) => {
      const p = new Pull(0, 0);
      for (let y = 10, at = 100; y <= to; y += 10, at += 100) p.moved(y, at);
      return p;
    };
    expect(slow(140).away(1500, 600)).toBe(false);
    expect(slow(160).away(1700, 600)).toBe(true);
  });

  it("puts it away with a flick, unless the finger stops before it lets go", () => {
    const flick = new Pull(0, 0);
    flick.moved(30, 30);
    flick.moved(60, 60);
    expect(flick.away(70, 600)).toBe(true);
    // Held still for a moment: no longer a flick.
    expect(flick.away(300, 600)).toBe(false);
    // A twitch isn't a flick.
    const twitch = new Pull(0, 0);
    twitch.moved(12, 10);
    expect(twitch.away(16, 600)).toBe(false);
  });
});

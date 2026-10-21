import { act, type ReactNode, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Lightbox, type LightboxFile, type LightboxItem, plainClick, Swipe } from "./lightbox";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.useRealTimers();
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
  return (next: ReactNode) => act(() => root.render(next));
}

const shot: LightboxFile = {
  name: "after.png",
  media_type: "image/png",
  size: 245_300,
  width: 1440,
  height: 900,
};

function open(file: LightboxFile = shot, download?: boolean) {
  const onClose = vi.fn();
  render(<Lightbox items={[{ file, src: "/files/after.png", download }]} onClose={onClose} />);
  const dialog = document.querySelector("dialog") as HTMLDialogElement;
  /** A press and release on `el`, ending on `end` if it moves. */
  const press = (el: Element, end: Element = el) =>
    act(() => {
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      end.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
  return { onClose, dialog, press };
}

describe("Lightbox", () => {
  it("shows the picture with its name and size over the page", () => {
    const { dialog } = open();
    expect(dialog.open).toBe(true);
    expect(dialog.getAttribute("aria-label")).toBe("after.png");
    expect(dialog.querySelector("img")?.getAttribute("src")).toBe("/files/after.png");
    expect(dialog.querySelector("figcaption")?.textContent).toBe("after.png1440×900 · 245 KB");
    // The X comes first, at the top right: Tab reaches it first.
    expect(dialog.querySelector("button")?.getAttribute("aria-label")).toBe("Close");
    expect(dialog.querySelector("button")?.className).toMatch(/\bfixed top-3 right-3\b/);
  });

  it("focuses itself as it opens, not the X, and shows no ring", () => {
    // Safari rings what a dialog focuses as it opens, even after a tap, and
    // Chrome rings the X it focuses once the arrow keys are pressed.
    const { dialog } = open();
    expect(dialog.hasAttribute("autofocus")).toBe(true);
    expect(document.activeElement).toBe(dialog);
    // The dialog fills the window: a ring around it would only show at the edges.
    expect(dialog.className).toMatch(/\boutline-none\b/);
  });

  it("closes with the X", () => {
    const { dialog, onClose } = open();
    act(() => dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')?.click());
    expect(dialog.open).toBe(false);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closes with a click outside the picture, not on it", () => {
    const { dialog, onClose, press } = open();
    const img = dialog.querySelector("img") as HTMLImageElement;
    const caption = dialog.querySelector("figcaption") as HTMLElement;
    const stage = dialog.querySelector("figure") as HTMLElement;
    press(img);
    press(caption);
    // Selecting or dragging from the picture out to the page.
    press(img, stage);
    expect(onClose).not.toHaveBeenCalled();
    press(stage);
    expect(dialog.open).toBe(false);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closes with a click on the backdrop", () => {
    const { dialog, onClose, press } = open();
    press(dialog);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("plays videos", () => {
    const { dialog } = open({ name: "bug.mov", media_type: "video/quicktime", size: 3_400_000 });
    const video = dialog.querySelector("video");
    expect(video?.getAttribute("src")).toBe("/files/after.png");
    expect(video?.controls).toBe(true);
    expect(video?.autoplay).toBe(true);
    expect(dialog.querySelector("img")).toBeNull();
    expect(dialog.querySelector("figcaption")?.textContent).toBe("bug.mov3.4 MB");
  });

  it("says when a video can't play or an image didn't load", () => {
    const video = open({ name: "bug.mov", media_type: "video/quicktime" }, true);
    act(() => video.dialog.querySelector("video")?.dispatchEvent(new Event("error")));
    expect(video.dialog.querySelector("p")?.textContent).toBe("This browser can't play bug.mov.");
    // Downloading it still works.
    expect(video.dialog.querySelector('a[aria-label="Download bug.mov"]')).not.toBeNull();
    for (const unmount of roots.splice(0)) unmount();
    const image = open();
    act(() => image.dialog.querySelector("img")?.dispatchEvent(new Event("error")));
    expect(image.dialog.querySelector("p")?.textContent).toBe("after.png didn't load.");
  });

  it("offers the download only when asked", () => {
    expect(open().dialog.querySelector("a[download]")).toBeNull();
    for (const unmount of roots.splice(0)) unmount();
    const link = open(shot, true).dialog.querySelector("a[download]");
    expect(link?.getAttribute("href")).toBe("/files/after.png");
    expect(link?.getAttribute("download")).toBe("after.png");
    // Not on touch screens, where iOS answers it with a sheet of its own.
    expect(link?.className).toMatch(/\bpointer-coarse:hidden\b/);
  });

  it("holds the page still while it is open", () => {
    const html = document.documentElement;
    open();
    expect(html.style.overflow).toBe("hidden");
    for (const unmount of roots.splice(0)) unmount();
    expect(html.style.overflow).toBe("");
  });
});

const pictures: LightboxItem[] = ["before.png", "after.png", "after-dark.png"].map((name) => ({
  file: { name, media_type: "image/png" },
  src: `/files/${name}`,
}));

/** A lightbox over `items` that shows the one it asks for, from `start`. */
function Steps({ items, start }: { items: LightboxItem[]; start: number }) {
  const [index, setIndex] = useState(start);
  return <Lightbox items={items} index={index} onIndex={setIndex} onClose={() => {}} />;
}

function browse(start = 0, items = pictures) {
  render(<Steps items={items} start={start} />);
  const dialog = document.querySelector("dialog") as HTMLDialogElement;
  const track = dialog.querySelector(".lightbox-track") as HTMLElement;
  // happy-dom lays nothing out: a window 400 px wide.
  Object.defineProperty(track, "offsetWidth", { value: 400 });
  const shown = () => dialog.getAttribute("aria-label");
  const key = (key: string, init: KeyboardEventInit = {}) => {
    const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...init });
    act(() => {
      dialog.dispatchEvent(e);
    });
    return e;
  };
  const button = (label: string) =>
    dialog.querySelector(`button[aria-label="${label}"]`) as HTMLButtonElement;
  const sliding = () => track.hasAttribute("data-sliding");
  /** The swiped picture finished sliding. (happy-dom's TransitionEvent leaves out `propertyName`.) */
  const slid = () =>
    act(() => {
      const ended = new Event("transitionend");
      Object.defineProperty(ended, "propertyName", { value: "translate" });
      track.dispatchEvent(ended);
    });
  return { dialog, track, shown, key, button, sliding, slid };
}

/**
 * A finger on `el` at each of `points` (x, and y if it isn't 300) in turn,
 * `ms` apart, then off the screen (or `cancel`led, or `held` there).
 * Returns its moves.
 */
function finger(
  el: Element,
  points: (number | [number, number])[],
  { ms = 100, end = "touchend" }: { ms?: number; end?: "touchend" | "touchcancel" | "held" } = {},
) {
  let at = 1000;
  const event = (type: string, touches: Touch[]) => {
    const e = new TouchEvent(type, { bubbles: true, cancelable: true, touches });
    Object.defineProperty(e, "timeStamp", { value: at });
    at += ms;
    return e;
  };
  const touch = (p: number | [number, number]) => {
    const [x, y] = typeof p === "number" ? [p, 300] : p;
    return new Touch({ identifier: 1, target: el, clientX: x, clientY: y });
  };
  const [first, ...rest] = points;
  const moves = rest.map((p) => event("touchmove", [touch(p)]));
  act(() => {
    el.dispatchEvent(event("touchstart", [touch(first ?? 0)]));
  });
  for (const move of moves)
    act(() => {
      el.dispatchEvent(move);
    });
  if (end !== "held")
    act(() => {
      el.dispatchEvent(event(end, []));
    });
  return moves;
}

/** A finger `held` on `el` comes off the screen, long after it last moved. */
function lift(el: Element) {
  const e = new TouchEvent("touchend", { bubbles: true, touches: [] });
  Object.defineProperty(e, "timeStamp", { value: 60_000 });
  act(() => {
    el.dispatchEvent(e);
  });
}

describe("Lightbox with several pictures", () => {
  it("says which one of how many it shows", () => {
    const { dialog, shown } = browse(1);
    expect(shown()).toBe("after.png");
    // Screen readers hear the name too, as it changes.
    const where = dialog.querySelector("p[aria-live]");
    expect(where?.textContent).toBe("after.png: 2 of 3");
    expect(where?.className).toMatch(/\bfixed top-3 left-3\b/);
  });

  it("steps with the arrow keys, not past the first or last", () => {
    const { shown, key } = browse(0);
    expect(key("ArrowLeft").defaultPrevented).toBe(true);
    expect(shown()).toBe("before.png");
    key("ArrowRight");
    expect(shown()).toBe("after.png");
    key("ArrowRight");
    key("ArrowRight");
    expect(shown()).toBe("after-dark.png");
    key("ArrowLeft");
    expect(shown()).toBe("after.png");
  });

  it("leaves arrows with a modifier key to the browser", () => {
    const { shown, key } = browse(0);
    for (const mod of ["altKey", "ctrlKey", "metaKey", "shiftKey"]) {
      expect(key("ArrowRight", { [mod]: true }).defaultPrevented).toBe(false);
    }
    expect(key("ArrowDown").defaultPrevented).toBe(false);
    expect(shown()).toBe("before.png");
  });

  it("steps with the buttons at its sides, which say when there is none", () => {
    const { dialog, shown, button } = browse(0);
    const previous = button("Previous image");
    const next = button("Next image");
    // Not disabled: it keeps focus.
    expect(previous.getAttribute("aria-disabled")).toBe("true");
    expect(previous.disabled).toBe(false);
    act(() => previous.click());
    expect(shown()).toBe("before.png");
    act(() => next.click());
    expect(shown()).toBe("after.png");
    expect(previous.hasAttribute("aria-disabled")).toBe(false);
    act(() => next.click());
    expect(shown()).toBe("after-dark.png");
    expect(next.getAttribute("aria-disabled")).toBe("true");
    expect(dialog.open).toBe(true);
    // Touch screens swipe instead.
    expect(next.className).toMatch(/\bpointer-coarse:hidden\b/);
    expect(next.getAttribute("aria-keyshortcuts")).toBe("ArrowRight");
  });

  it("keeps the ones beside the picture out of sight and reach, loaded for a swipe", () => {
    const { dialog, key } = browse(1);
    const slides = () =>
      [...dialog.querySelectorAll("figure")].map((f) => [
        f.querySelector("img")?.getAttribute("src"),
        f.style.translate,
        f.hasAttribute("inert"),
      ]);
    expect(slides()).toEqual([
      ["/files/before.png", "-100% 0", true],
      ["/files/after.png", "", false],
      ["/files/after-dark.png", "100% 0", true],
    ]);
    const next = dialog.querySelector('img[src="/files/after-dark.png"]');
    key("ArrowRight");
    expect(slides()).toEqual([
      ["/files/after.png", "-100% 0", true],
      ["/files/after-dark.png", "", false],
    ]);
    // The same picture, already loaded.
    expect(dialog.querySelector('img[src="/files/after-dark.png"]')).toBe(next);
  });

  it("follows a finger that swipes the picture aside, and brings in the next one", () => {
    const { dialog, track, shown, sliding, slid } = browse(1);
    const moves = finger(dialog, [300, 290, 250, 150]);
    // Once it moved more than a tap does, from there.
    expect(moves.map((m) => m.defaultPrevented)).toEqual([true, true, true]);
    expect(sliding()).toBe(true);
    expect(track.style.translate).toBe("-100% 0");
    expect(shown()).toBe("after.png");
    slid();
    expect(shown()).toBe("after-dark.png");
    expect(sliding()).toBe(false);
    expect(track.style.translate).toBe("");
  });

  it("follows the finger as it moves", () => {
    const { dialog, track, sliding } = browse(1);
    finger(dialog, [300, 320, 380], { end: "held" });
    expect(track.style.translate).toBe("60px 0");
    expect(sliding()).toBe(false);
  });

  it("brings in the previous one after a swipe to the right", () => {
    const { dialog, shown, slid } = browse(1);
    finger(dialog, [100, 110, 150, 250]);
    slid();
    expect(shown()).toBe("before.png");
  });

  it("settles back after a short swipe", () => {
    const { dialog, track, shown, sliding, slid } = browse(1);
    // 80 px of 400, slowly.
    finger(dialog, [300, 290, 250, 210]);
    expect(sliding()).toBe(true);
    expect(track.style.translate).toBe("");
    slid();
    expect(shown()).toBe("after.png");
    expect(sliding()).toBe(false);
  });

  it("brings in the next one with a flick", () => {
    const { dialog, shown, slid } = browse(1);
    // 60 px of 400, quickly.
    finger(dialog, [300, 290, 260, 230], { ms: 20 });
    slid();
    expect(shown()).toBe("after-dark.png");
  });

  it("resists a swipe past the first or last picture, and settles back", () => {
    const { dialog, track, shown, sliding, slid } = browse(0);
    // 200 px toward a picture before the first.
    finger(dialog, [100, 110, 310], { end: "held" });
    expect(track.style.translate).toBe("60px 0");
    lift(dialog);
    expect(sliding()).toBe(true);
    expect(track.style.translate).toBe("");
    slid();
    expect(shown()).toBe("before.png");
  });

  it("leaves a finger that moves up or down, or hardly at all, and settles back when the browser takes it", () => {
    const { dialog, track, shown, sliding } = browse(1);
    const [down] = finger(dialog, [
      [300, 300],
      [304, 340],
      [150, 360],
    ]);
    expect(down?.defaultPrevented).toBe(false);
    expect(track.style.translate).toBe("");
    // A tap.
    finger(dialog, [300, 296]);
    expect(sliding()).toBe(false);
    finger(dialog, [300, 290, 150], { end: "touchcancel" });
    expect(sliding()).toBe(true);
    expect(track.style.translate).toBe("");
    expect(shown()).toBe("after.png");
  });

  it("doesn't close from a swipe that starts outside the picture", () => {
    const { dialog, track } = browse(1);
    const stage = track.querySelector("figure:not([inert])") as HTMLElement;
    act(() => {
      stage.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    finger(stage, [300, 290, 250]);
    act(() => {
      stage.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(dialog.open).toBe(true);
  });

  it("puts a sliding picture in place at once for the next swipe or key", () => {
    const { dialog, shown, sliding, key } = browse(0);
    finger(dialog, [300, 290, 100]);
    expect(sliding()).toBe(true);
    key("ArrowRight");
    expect(sliding()).toBe(false);
    expect(shown()).toBe("after-dark.png");
    key("ArrowLeft");
    finger(dialog, [100, 110, 300]);
    expect(sliding()).toBe(true);
    finger(dialog, [300, 290, 100]);
    expect(shown()).toBe("before.png");
  });

  it("slides into place even when the transition doesn't end", () => {
    vi.useFakeTimers();
    const { dialog, shown, sliding } = browse(1);
    finger(dialog, [300, 290, 100]);
    act(() => vi.advanceTimersByTime(400));
    expect(sliding()).toBe(false);
    expect(shown()).toBe("after-dark.png");
  });

  it("puts the picture back in place when the others go as it slides", () => {
    const show = (items: LightboxItem[]) => <Lightbox items={items} onIndex={() => {}} onClose={() => {}} />;
    const rerender = render(show(pictures));
    const dialog = document.querySelector("dialog") as HTMLDialogElement;
    const track = dialog.querySelector(".lightbox-track") as HTMLElement;
    Object.defineProperty(track, "offsetWidth", { value: 400 });
    finger(dialog, [300, 290, 100]);
    expect(track.style.translate).toBe("-100% 0");
    rerender(show(pictures.slice(0, 1)));
    expect(track.style.translate).toBe("");
    expect(track.hasAttribute("data-sliding")).toBe(false);
  });

  it("doesn't step after it closes mid-slide", () => {
    vi.useFakeTimers();
    const onIndex = vi.fn();
    render(<Lightbox items={pictures} onIndex={onIndex} onClose={() => {}} />);
    const dialog = document.querySelector("dialog") as HTMLDialogElement;
    Object.defineProperty(dialog.querySelector(".lightbox-track"), "offsetWidth", { value: 400 });
    finger(dialog, [300, 290, 100]);
    for (const unmount of roots.splice(0)) unmount();
    act(() => vi.advanceTimersByTime(400));
    expect(onIndex).not.toHaveBeenCalled();
  });

  it("has no steps for one file", () => {
    const { dialog } = open();
    expect(dialog.querySelector('button[aria-label="Next image"]')).toBeNull();
    expect(dialog.querySelector("p[aria-live]")).toBeNull();
    expect(dialog.querySelectorAll("figure")).toHaveLength(1);
    const right = new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true, cancelable: true });
    act(() => {
      dialog.dispatchEvent(right);
    });
    expect(right.defaultPrevented).toBe(false);
    // A video's player takes the fingers.
    const [move] = finger(dialog, [300, 290, 100]);
    expect(move?.defaultPrevented).toBe(false);
    expect(dialog.className).not.toMatch(/\btouch-none\b/);
  });
});

describe("Swipe", () => {
  it("follows the finger either way", () => {
    const s = new Swipe(200, 0);
    s.moved(260, 16);
    expect(s.distance).toBe(60);
    s.moved(150, 32);
    expect(s.distance).toBe(-50);
  });

  it("brings in the next or previous picture once swiped a quarter of the width", () => {
    // Slowly: 10 px every 100 ms, then off the screen.
    const slow = (to: number) => {
      const s = new Swipe(0, 0);
      const steps = Math.abs(to) / 10;
      for (let i = 1; i <= steps; i++) s.moved(Math.sign(to) * 10 * i, i * 100);
      return s.step(steps * 100 + 50, 400);
    };
    expect(slow(-90)).toBe(0);
    expect(slow(-110)).toBe(1);
    expect(slow(90)).toBe(0);
    expect(slow(110)).toBe(-1);
  });

  it("brings one in with a flick, unless the finger stops before it lets go", () => {
    const flick = new Swipe(300, 0);
    flick.moved(270, 30);
    flick.moved(240, 60);
    expect(flick.step(70, 400)).toBe(1);
    // Held still for a moment: no longer a flick.
    expect(flick.step(300, 400)).toBe(0);
    // A twitch isn't a flick.
    const twitch = new Swipe(300, 0);
    twitch.moved(288, 10);
    expect(twitch.step(16, 400)).toBe(0);
  });

  it("brings in none with a flick back toward where the finger started", () => {
    const s = new Swipe(300, 0);
    for (let x = 290, at = 100; x >= 100; x -= 10, at += 100) s.moved(x, at);
    expect(s.step(2000, 400)).toBe(1);
    s.moved(130, 2010);
    s.moved(160, 2020);
    expect(s.step(2030, 400)).toBe(0);
  });
});

describe("plainClick", () => {
  const click = (init: MouseEventInit = {}) => new MouseEvent("click", init);
  it("is a click the page may handle itself", () => {
    expect(plainClick(click())).toBe(true);
  });

  it("isn't one the browser opens in a new tab or window", () => {
    expect(plainClick(click({ metaKey: true }))).toBe(false);
    expect(plainClick(click({ ctrlKey: true }))).toBe(false);
    expect(plainClick(click({ shiftKey: true }))).toBe(false);
    expect(plainClick(click({ altKey: true }))).toBe(false);
    expect(plainClick(click({ button: 1 }))).toBe(false);
  });
});

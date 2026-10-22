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
    // Nor is there one to swipe to.
    const [move] = finger(dialog, [300, 290, 100]);
    expect(move?.defaultPrevented).toBe(false);
  });

  it("leaves the fingers on a video alone to its player", () => {
    const { dialog } = open({ name: "bug.mov", media_type: "video/quicktime" });
    const [move] = finger(dialog, [300, 290, 100]);
    expect(move?.defaultPrevented).toBe(false);
    expect(dialog.className).not.toMatch(/\btouch-none\b/);
  });
});

/**
 * happy-dom lays nothing out: a window 400 px wide and 800 px tall, with each
 * image 360 × 200 in its middle, 1440 px wide in its own pixels. Returns the
 * image shown.
 */
function layOut(dialog: HTMLDialogElement) {
  const set = (el: Element, values: Record<string, number | boolean>) => {
    for (const [key, value] of Object.entries(values)) Object.defineProperty(el, key, { value });
  };
  set(dialog, { clientWidth: 400, clientHeight: 800 });
  for (const img of dialog.querySelectorAll("img")) {
    set(img, {
      complete: true,
      naturalWidth: 1440,
      offsetLeft: 20,
      offsetTop: 300,
      offsetWidth: 360,
      offsetHeight: 200,
    });
  }
  return () => dialog.querySelector("figure:not([inert]) img") as HTMLImageElement;
}

/**
 * Fingers on `el`, from `start` ms, a moment every `ms`. Each moment says
 * where each finger is, "x,y", or "_" before it touches or after it lets
 * go, and in each, fingers touch, let go or move. Fingers still on at the
 * end let go, unless `held`; `on` of them were on from before. Returns the
 * moves.
 */
function hand(
  el: Element,
  moments: string[],
  {
    start = 1000,
    ms = 16,
    held = false,
    on = 0,
  }: { start?: number; ms?: number; held?: boolean; on?: number } = {},
) {
  let at = start;
  const moves: TouchEvent[] = [];
  const send = (type: string, touches: Touch[]) => {
    const e = new TouchEvent(type, { bubbles: true, cancelable: true, touches });
    Object.defineProperty(e, "timeStamp", { value: at });
    act(() => {
      el.dispatchEvent(e);
    });
    at += ms;
    return e;
  };
  for (const moment of moments) {
    const touches = moment.split(" ").flatMap((spot, i) => {
      const [x, y] = spot.split(",").map(Number);
      return spot === "_" ? [] : [new Touch({ identifier: i + 1, target: el, clientX: x, clientY: y })];
    });
    const type = touches.length > on ? "touchstart" : touches.length < on ? "touchend" : "touchmove";
    const e = send(type, touches);
    if (type === "touchmove") moves.push(e);
    on = touches.length;
  }
  if (on && !held) send("touchend", []);
  return moves;
}

/** Two fingers that touch at `from` ("x,y x,y"), move to `to` and let go, the first one first. */
function pinch(el: Element, from: string, to: string, options: { held?: boolean; start?: number } = {}) {
  const [first] = from.split(" ");
  const [, second] = to.split(" ");
  return hand(el, [`${first}`, from, to, ...(options.held ? [] : [`_ ${second}`])], options);
}

/** How the image shows: its scale and how far it is moved, or "" for each as it fits. */
const zoomOf = (img: HTMLImageElement) => [img.style.scale, img.style.translate];

/** A zoomed image settled into place. */
function settled(img: HTMLImageElement) {
  act(() => {
    const ended = new Event("transitionend");
    Object.defineProperty(ended, "propertyName", { value: "scale" });
    img.dispatchEvent(ended);
  });
}

describe("Lightbox zooming in", () => {
  it("zooms an image with two fingers, keeping the spot between them between them", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    // 100 px left of the image's middle and 50 px above it, twice as far apart.
    const moves = pinch(dialog, "50,350 150,350", "0,350 200,350", { held: true });
    expect(moves.map((m) => m.defaultPrevented)).toEqual([true]);
    expect(zoomOf(img)).toEqual(["2", "100px 50px"]);
    // Not the page: the dialog takes the fingers.
    expect(dialog.className).toMatch(/\btouch-none\b/);
    // Drawn at each size as it grows, not on a layer of its own.
    expect(img.hasAttribute("data-moving")).toBe(false);
  });

  it("settles an image zoomed out back to how it fits", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    pinch(dialog, "100,400 300,400", "150,400 250,400", { held: true });
    expect(Number(img.style.scale)).toBeCloseTo(0.85);
    // One finger lets go, then the other.
    hand(dialog, ["_ 250,400"], { on: 2, start: 2000 });
    expect(img.hasAttribute("data-settling")).toBe(true);
    expect(zoomOf(img)).toEqual(["1", "0px 0px"]);
    settled(img);
    expect(img.hasAttribute("data-settling")).toBe(false);
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("settles an image two fingers moved aside, not zooming it, back to how it fits", () => {
    const { dialog, track } = browse(1);
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "180,430 280,430", { held: true });
    expect(zoomOf(img)).toEqual(["1", "30px 30px"]);
    // The finger left on doesn't swipe.
    hand(dialog, ["_ 280,430", "_ 150,430"], { on: 2, start: 2000 });
    expect(track.style.translate).toBe("");
    expect(zoomOf(img)).toEqual(["1", "0px 0px"]);
    settled(img);
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("settles an image zoomed in past the most back to it", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    // Eight times as far apart, about the image's middle.
    pinch(dialog, "190,400 210,400", "120,400 280,400");
    expect(img.hasAttribute("data-settling")).toBe(true);
    expect(zoomOf(img)).toEqual(["4", "0px 0px"]);
    settled(img);
    expect(zoomOf(img)).toEqual(["4", "0px 0px"]);
  });

  it("moves a zoomed image about with one finger, not past its edges, and doesn't swipe", () => {
    const { dialog, track, shown } = browse(1);
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    expect(zoomOf(img)).toEqual(["2", "0px 0px"]);
    const moves = hand(dialog, ["200,400", "230,450", "260,300"], { start: 2000, ms: 200, held: true });
    expect(moves.map((m) => m.defaultPrevented)).toEqual([true, true]);
    expect(zoomOf(img)).toEqual(["2", "60px -100px"]);
    // On a layer of its own while the finger moves it, and drawn anew once it lets go.
    expect(img.hasAttribute("data-moving")).toBe(true);
    lift(dialog);
    expect(img.hasAttribute("data-moving")).toBe(false);
    expect(track.style.translate).toBe("");
    expect(shown()).toBe("after.png");
    // Past its bottom edge, 100 px up from there, it resists, and settles back.
    hand(dialog, ["200,400", "200,200"], { start: 4000, held: true });
    expect(img.style.translate).toBe("60px -230px");
    lift(dialog);
    expect(zoomOf(img)).toEqual(["2", "60px -200px"]);
  });

  it("moves an image on with the finger left after a pinch", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400", { held: true });
    hand(dialog, ["_ 300,400", "_ 280,420"], { on: 2, start: 2000, ms: 200 });
    expect(zoomOf(img)).toEqual(["2", "-20px 20px"]);
  });

  it("glides on after a flick, until a finger catches it", () => {
    vi.useFakeTimers();
    const { dialog } = open();
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    // 1 px/ms to the left.
    hand(dialog, ["200,400", "184,400", "168,400"], { start: 2000 });
    expect(img.style.translate).toBe("-32px 0px");
    act(() => vi.advanceTimersToNextFrame());
    expect(img.style.translate).toBe("-48px 0px");
    act(() => vi.advanceTimersToNextFrame());
    const gliding = parseFloat(img.style.translate);
    expect(gliding).toBeLessThan(-48);
    expect(img.hasAttribute("data-moving")).toBe(true);
    hand(dialog, ["200,400"], { start: 3000, held: true });
    act(() => vi.advanceTimersByTime(500));
    expect(parseFloat(img.style.translate)).toBe(gliding);
    // Flicked again, it glides on to its edge, and stops there.
    hand(dialog, ["200,400", "184,400", "168,400"], { on: 1, start: 4000 });
    act(() => vi.advanceTimersByTime(2000));
    expect(img.style.translate).toBe("-160px 0px");
    expect(img.hasAttribute("data-moving")).toBe(false);
  });

  it("doesn't glide with reduced motion", () => {
    vi.useFakeTimers();
    vi.stubGlobal("matchMedia", (query: string) => ({ matches: query.includes("reduce"), media: query }));
    try {
      const { dialog } = open();
      const img = layOut(dialog)();
      pinch(dialog, "150,400 250,400", "100,400 300,400");
      hand(dialog, ["200,400", "184,400", "168,400"], { start: 2000 });
      act(() => vi.advanceTimersByTime(500));
      expect(img.style.translate).toBe("-32px 0px");
    } finally {
      vi.unstubAllGlobals();
    }
  });

  it("zooms in three times about a double tap on the image, and back out", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    hand(img, ["100,350"], { start: 1000, ms: 80 });
    expect(zoomOf(img)).toEqual(["", ""]);
    // About where the second tap lands.
    hand(img, ["104,352"], { start: 1250, ms: 80 });
    expect(img.hasAttribute("data-settling")).toBe(true);
    expect(zoomOf(img)).toEqual(["3", "192px 96px"]);
    settled(img);
    hand(img, ["300,400"], { start: 2000, ms: 80 });
    hand(img, ["300,400"], { start: 2200, ms: 80 });
    expect(zoomOf(img)).toEqual(["1", "0px 0px"]);
    settled(img);
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("leaves taps too far apart, held, or beside the image", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    // Too long between them.
    hand(img, ["100,350"], { start: 1000, ms: 80 });
    hand(img, ["100,350"], { start: 1500, ms: 80 });
    // Too far from each other.
    hand(img, ["100,350"], { start: 3000, ms: 80 });
    hand(img, ["200,450"], { start: 3200, ms: 80 });
    // Held.
    hand(img, ["100,350"], { start: 5000, ms: 80 });
    hand(img, ["100,350"], { start: 5200, ms: 400 });
    // Beside the image.
    const stage = dialog.querySelector("figure") as HTMLElement;
    hand(stage, ["10,100"], { start: 7000, ms: 80 });
    hand(stage, ["10,100"], { start: 7200, ms: 80 });
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("swipes on past a zoomed image's left or right edge, to the next one as it fits", () => {
    const { dialog, track, shown, sliding, slid } = browse(1);
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    // 720 px wide: 160 px to its right edge, then 140 px on.
    hand(dialog, ["350,400", "250,400", "50,400"], { start: 2000, ms: 50, held: true });
    expect(img.style.translate).toBe("-160px 0px");
    expect(track.style.translate).toBe("-140px 0");
    lift(dialog);
    expect(sliding()).toBe(true);
    expect(track.style.translate).toBe("-100% 0");
    slid();
    expect(shown()).toBe("after-dark.png");
    // The one it left shows as it fits.
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("follows a zoomed image back from past its edge, and settles the pictures back after a short swipe", () => {
    const { dialog, track, shown, sliding } = browse(1);
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    // 40 px past its right edge, and back.
    hand(dialog, ["350,400", "150,400"], { start: 2000, ms: 200, held: true });
    expect(img.style.translate).toBe("-160px 0px");
    expect(track.style.translate).toBe("-40px 0");
    hand(dialog, ["250,400"], { on: 1, start: 2400, ms: 200, held: true });
    expect(img.style.translate).toBe("-100px 0px");
    expect(track.style.translate).toBe("");
    // 40 px past it again, slowly.
    hand(dialog, ["150,400"], { on: 1, start: 2600, ms: 200, held: true });
    expect(track.style.translate).toBe("-40px 0");
    lift(dialog);
    expect(sliding()).toBe(true);
    expect(track.style.translate).toBe("");
    expect(shown()).toBe("after.png");
    expect(img.style.translate).toBe("-160px 0px");
  });

  it("zooms out the image another one takes the place of", () => {
    const { dialog, key } = browse(1);
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    expect(img.style.scale).toBe("2");
    key("ArrowRight");
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("keeps the image where it is when a second finger joins a swipe, and zooms it", () => {
    const { dialog, track, sliding } = browse(1);
    const img = layOut(dialog)();
    hand(dialog, ["300,400", "250,400", "200,400"], { held: true });
    expect(track.style.translate).toBe("-50px 0");
    // The pictures go back under the image at once, and it stays 50 px to the left.
    hand(dialog, ["200,400 300,400"], { on: 1, start: 2000, held: true });
    expect(track.style.translate).toBe("");
    expect(sliding()).toBe(false);
    expect(zoomOf(img)).toEqual(["1", "-50px 0px"]);
    // Twice as far apart, about the spot between them.
    hand(dialog, ["150,400 350,400"], { on: 2, start: 2100, held: true });
    expect(zoomOf(img)).toEqual(["2", "-150px 0px"]);
  });

  it("draws an image two fingers zoom anew, though one touched it first", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    hand(dialog, ["200,400"], { start: 2000, held: true });
    expect(img.hasAttribute("data-moving")).toBe(true);
    hand(dialog, ["200,400 300,400"], { on: 1, start: 2100, held: true });
    expect(img.hasAttribute("data-moving")).toBe(false);
  });

  it("moves an image on from where a pinch left it, past its edge, without a jump or a swipe", () => {
    const { dialog, track, shown } = browse(1);
    const img = layOut(dialog)();
    // Twice as far apart, moving right: 40 px past its left edge.
    pinch(dialog, "150,400 250,400", "300,400 500,400", { held: true });
    expect(zoomOf(img)).toEqual(["2", "200px 0px"]);
    // The first finger lets go; the other moves on 1 px, then 1 px more.
    hand(dialog, ["_ 500,400", "_ 501,400"], { on: 2, start: 2000, held: true });
    expect(img.style.translate).toBe("200px 0px");
    expect(track.style.translate).toBe("1px 0");
    hand(dialog, ["_ 502,400"], { on: 1, start: 2032 });
    expect(shown()).toBe("after.png");
    // Back within its edges.
    expect(zoomOf(img)).toEqual(["2", "160px 0px"]);
  });

  it("moves a lone image on from where a pinch left it, past its edge, without a jump", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "300,400 500,400", { held: true });
    hand(dialog, ["_ 500,400", "_ 510,400"], { on: 2, start: 2000, held: true });
    expect(img.style.translate).toBe("203px 0px");
  });

  it("doesn't swipe on with a finger moving up along a zoomed image's edge", () => {
    const { dialog, track, shown, sliding } = browse(1);
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    // To its left edge.
    hand(dialog, ["100,400", "260,400"], { start: 2000, ms: 200 });
    expect(img.style.translate).toBe("160px 0px");
    // A flick up, a little to the right.
    hand(dialog, ["200,600", "210,563", "220,525", "230,488", "240,450"], {
      start: 3000,
      ms: 20,
      held: true,
    });
    expect(track.style.translate).toBe("");
    lift(dialog);
    expect(sliding()).toBe(false);
    expect(shown()).toBe("after.png");
  });

  it("doesn't glide after a tap on a zoomed image", () => {
    vi.useFakeTimers();
    const { dialog } = open();
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    hand(img, ["200,400", "202,400", "203,400"], { start: 2000 });
    act(() => vi.advanceTimersByTime(1000));
    expect(img.style.translate).toBe("3px 0px");
  });

  it("leaves an image two fingers hardly zoomed as it fits, and the finger left doesn't move it", () => {
    const { dialog, track } = browse(1);
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "180,430 280.1,430", { held: true });
    expect(Number(img.style.scale)).toBeCloseTo(1.001);
    hand(dialog, ["_ 280.1,430", "_ 200,430"], { on: 2, start: 2000, held: true });
    expect(track.style.translate).toBe("");
    expect(zoomOf(img)).toEqual(["1", "0px 0px"]);
  });

  it("settles an image a pinch left out of place when another picture arrives", () => {
    const show = (items: LightboxItem[]) => <Lightbox items={items} onIndex={() => {}} onClose={() => {}} />;
    const rerender = render(show(pictures.slice(0, 1)));
    const dialog = document.querySelector("dialog") as HTMLDialogElement;
    const img = layOut(dialog)();
    pinch(dialog, "100,400 300,400", "150,400 250,400", { held: true });
    expect(Number(img.style.scale)).toBeCloseTo(0.85);
    rerender(show(pictures));
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("doesn't close after a pinch that starts outside the image", () => {
    const { dialog, onClose } = open();
    layOut(dialog);
    const stage = dialog.querySelector("figure") as HTMLElement;
    act(() => {
      stage.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    pinch(stage, "150,400 250,400", "100,400 300,400");
    act(() => {
      stage.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
    expect(onClose).not.toHaveBeenCalled();
  });

  it("zooms nothing before the image loads", () => {
    const { dialog } = open();
    const img = dialog.querySelector("img") as HTMLImageElement;
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("shows the image as it fits again when the window turns", () => {
    const { dialog } = open();
    const img = layOut(dialog)();
    pinch(dialog, "150,400 250,400", "100,400 300,400");
    expect(img.style.scale).toBe("2");
    act(() => {
      window.dispatchEvent(new Event("resize"));
    });
    expect(zoomOf(img)).toEqual(["", ""]);
  });

  it("keeps the caption over a zoomed image", () => {
    const { dialog } = open();
    // Positioned, it comes after the zoomed image.
    expect(dialog.querySelector("figcaption")?.className).toMatch(/\brelative\b/);
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

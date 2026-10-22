import { describe, expect, it } from "vitest";
import { FITS, stopPinchZoom, Trail, Zoom } from "./zoom";

/** Pinches as Safari does, and says whether the page cancelled it. */
function pinch(target: EventTarget, type = "gesturestart") {
  const e = new Event(type, { cancelable: true });
  target.dispatchEvent(e);
  return e.defaultPrevented;
}

describe("stopPinchZoom", () => {
  it("cancels Safari's pinches on a touch screen, until undone", () => {
    const page = new EventTarget();
    const undo = stopPinchZoom(page, true);
    expect(pinch(page, "gesturestart")).toBe(true);
    expect(pinch(page, "gesturechange")).toBe(true);
    undo();
    expect(pinch(page)).toBe(false);
  });

  it("leaves a Mac's trackpad zooming", () => {
    const page = new EventTarget();
    stopPinchZoom(page, false);
    expect(pinch(page)).toBe(false);
  });
});

// A window 400 px wide and 800 px tall, with a picture 360 × 200 in its middle.
const win = { width: 400, height: 800 };
const fit = { left: 20, top: 300, width: 360, height: 200 };
const p = (x: number, y: number) => ({ x, y });

describe("Zoom", () => {
  it("zooms in four times, or until the picture shows at its own size", () => {
    expect(new Zoom(win, fit, 720).max).toBe(4);
    expect(new Zoom(win, fit, 2880).max).toBe(8);
  });

  it("keeps the spot between two fingers between them as they zoom", () => {
    const zoom = new Zoom(win, fit, 1440);
    // 100 px left of the picture's middle and 50 px above it, twice as far apart.
    expect(zoom.pinch(FITS, [p(50, 350), p(150, 350)], [p(0, 350), p(200, 350)])).toEqual({
      scale: 2,
      x: 100,
      y: 50,
    });
    // Moving together, they move the picture.
    expect(
      zoom.pinch({ scale: 2, x: 100, y: 50 }, [p(0, 350), p(200, 350)], [p(30, 370), p(230, 370)]),
    ).toEqual({ scale: 2, x: 130, y: 70 });
  });

  it("resists zooming out past the fitting size or in past the most", () => {
    const zoom = new Zoom(win, fit, 1440);
    expect(zoom.pinch(FITS, [p(100, 400), p(300, 400)], [p(150, 400), p(250, 400)]).scale).toBeCloseTo(0.85);
    const most = { scale: 4, x: 0, y: 0 };
    expect(zoom.pinch(most, [p(150, 400), p(250, 400)], [p(100, 400), p(300, 400)]).scale).toBeCloseTo(5.2);
  });

  it("lets a picture larger than the window cover it, and a smaller one stay in it", () => {
    const zoom = new Zoom(win, fit, 1440);
    // 720 × 400: wider than the window, not as tall.
    expect(zoom.bounds(2)).toEqual({ x: [-160, 160], y: [-200, 200] });
    expect(zoom.bounds(1)).toEqual({ x: [-20, 20], y: [-300, 300] });
  });

  it("lets a zoomed picture stay where it fits", () => {
    // 20 px above the window's middle: four times as tall, it leaves 20 px below it.
    const zoom = new Zoom(win, { ...fit, top: 280 }, 1440);
    expect(zoom.bounds(4).y).toEqual([0, 20]);
  });

  it("follows a finger, and resists it past the picture's edges", () => {
    const zoom = new Zoom(win, fit, 1440);
    const view = { scale: 2, x: 100, y: 0 };
    expect(zoom.pan(view, p(-50, 120))).toEqual({ scale: 2, x: 50, y: 120 });
    // 40 px past the left edge, and 100 px past the top.
    const past = zoom.pan(view, p(100, 300));
    expect(past.x).toBeCloseTo(172);
    expect(past.y).toBeCloseTo(230);
  });

  it("settles a picture within its edges, zoomed in no further than the most", () => {
    const zoom = new Zoom(win, fit, 1440);
    expect(zoom.settle({ scale: 2, x: 300, y: -250 }, p(0, 0))).toEqual({ scale: 2, x: 160, y: -200 });
    expect(zoom.settle({ scale: 2, x: 30, y: 40 }, p(0, 0))).toEqual({ scale: 2, x: 30, y: 40 });
    // Twice too far, about the spot 100 px left of the picture's middle.
    expect(zoom.settle({ scale: 8, x: 0, y: 0 }, p(100, 400))).toEqual({ scale: 4, x: -50, y: 0 });
  });

  it("settles a picture zoomed out, or hardly in, as it fits", () => {
    const zoom = new Zoom(win, fit, 1440);
    expect(zoom.settle({ scale: 0.8, x: 40, y: 0 }, p(200, 400))).toBe(FITS);
    expect(zoom.settle({ scale: 1.005, x: 2, y: 0 }, p(200, 400))).toBe(FITS);
  });

  it("zooms in three times about a double tap, within the edges, and back out", () => {
    const zoom = new Zoom(win, fit, 1440);
    expect(zoom.toggle(FITS, p(100, 350))).toEqual({ scale: 3, x: 200, y: 100 });
    // By the right edge: the picture's edge stays on the window's.
    expect(zoom.toggle(FITS, p(390, 400))).toEqual({ scale: 3, x: -340, y: 0 });
    expect(zoom.toggle({ scale: 3, x: 200, y: 100 }, p(100, 350))).toBe(FITS);
  });

  it("glides on, slowing down, and stops at the edges", () => {
    const zoom = new Zoom(win, fit, 1440);
    const on = zoom.glide({ scale: 2, x: 0, y: 0 }, p(1, -0.5), 16);
    expect(on.view).toEqual({ scale: 2, x: 16, y: -8 });
    expect(on.speed.x).toBeCloseTo(0.923);
    expect(on.speed.y).toBeCloseTo(-0.4615);
    const edge = zoom.glide({ scale: 2, x: 150, y: 0 }, p(1, 0.5), 16);
    expect(edge.view).toEqual({ scale: 2, x: 160, y: 8 });
    expect(edge.speed.x).toBe(0);
    expect(edge.speed.y).toBeGreaterThan(0);
  });

  it("stops a glide once it is slow", () => {
    const zoom = new Zoom(win, fit, 1440);
    let glide = { view: { scale: 2, x: -150, y: 0 }, speed: p(1, 0) };
    let frames = 0;
    while ((glide.speed.x || glide.speed.y) && frames < 1000) {
      glide = zoom.glide(glide.view, glide.speed, 16);
      frames++;
    }
    // About 200 px for each px/ms it let go at, in under a second.
    expect(glide.view.x).toBeGreaterThan(-150 + 180);
    expect(glide.view.x).toBeLessThan(-150 + 220);
    expect(frames * 16).toBeLessThan(1000);
    const still = zoom.glide({ scale: 2, x: 5, y: 5 }, p(0.01, 0), 16);
    expect(still).toEqual({ view: { scale: 2, x: 5, y: 5 }, speed: { x: 0, y: 0 } });
  });
});

describe("Trail", () => {
  it("measures a finger's speed over its last moments", () => {
    const trail = new Trail();
    trail.moved(p(0, 0), 0);
    trail.moved(p(10, 0), 100);
    trail.moved(p(40, 20), 180);
    trail.moved(p(80, 40), 200);
    expect(trail.speed(200)).toEqual({ x: 0.7, y: 0.4 });
  });

  it("measures none once the finger held still", () => {
    const trail = new Trail();
    trail.moved(p(0, 0), 0);
    trail.moved(p(50, 0), 20);
    expect(trail.speed(400)).toEqual({ x: 0, y: 0 });
    expect(new Trail().speed(0)).toEqual({ x: 0, y: 0 });
  });
});

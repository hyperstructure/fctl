import { describe, expect, it } from "vitest";
import { stopPinchZoom } from "./zoom";

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

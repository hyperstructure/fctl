import { describe, expect, it } from "vitest";
import { syncShimmers } from "./shimmer";

/** A label on the page with these CSS animations, each started a while after the page opened. */
function label(page: Element, ...names: string[]) {
  const el = document.createElement("span");
  const animations = names.map((animationName) => ({ animationName, startTime: 1250 as number | null }));
  el.getAnimations = () => animations as unknown as Animation[];
  page.append(el);
  return { el, animations };
}

/** Starts an animation of `el`, as the browser reports it: the event bubbles up to the page. */
function start(el: Element, animationName: string) {
  el.dispatchEvent(Object.assign(new Event("animationstart", { bubbles: true }), { animationName }));
}

describe("syncShimmers", () => {
  it("starts a label's shimmer in step with every other one, until undone", () => {
    const page = document.createElement("div");
    const undo = syncShimmers(page);
    const thinking = label(page, "text-shimmer");
    start(thinking.el, "text-shimmer");
    expect(thinking.animations[0]?.startTime).toBe(0);

    undo();
    const working = label(page, "text-shimmer");
    start(working.el, "text-shimmer");
    expect(working.animations[0]?.startTime).toBe(1250);
  });

  it("leaves the label's other animations as they are", () => {
    const page = document.createElement("div");
    syncShimmers(page);
    const step = label(page, "step-in", "text-shimmer");
    start(step.el, "step-in");
    expect(step.animations.map((a) => a.startTime)).toEqual([1250, 1250]);
    start(step.el, "text-shimmer");
    expect(step.animations.map((a) => a.startTime)).toEqual([1250, 0]);
  });
});

import { describe, expect, it } from "vitest";
import { drags, Slide } from "./drawer-drag";

describe("drags", () => {
  it("waits until the finger has moved further than a tap", () => {
    expect(drags(false, 5, 2)).toBeUndefined();
    expect(drags(true, -5, 2)).toBeUndefined();
  });

  it("takes a finger moving sideways toward where the drawer goes", () => {
    expect(drags(false, 12, 4)).toBe(true);
    expect(drags(true, -12, 4)).toBe(true);
  });

  it("leaves a finger moving the other way, or up and down, to scroll", () => {
    expect(drags(false, -12, 4)).toBe(false);
    expect(drags(true, 12, 4)).toBe(false);
    expect(drags(false, 8, 12)).toBe(false);
    expect(drags(true, -8, -12)).toBe(false);
  });
});

describe("Slide", () => {
  it("brings a closed drawer in as far as the finger moved, and no further than open", () => {
    const s = new Slide(false, 244, 10, 0);
    expect(s.offset).toBe(-244);
    expect(s.shown).toBe(0);
    s.moved(71, 50);
    expect(s.offset).toBe(-183);
    expect(s.shown).toBeCloseTo(0.25);
    s.moved(400, 100);
    expect(s.offset).toBe(0);
    expect(s.shown).toBe(1);
  });

  it("takes an open drawer out as far as the finger moved, and no further than closed", () => {
    const s = new Slide(true, 244, 200, 0);
    s.moved(300, 20);
    expect(s.offset).toBe(0);
    s.moved(139, 50);
    expect(s.offset).toBe(-61);
    s.moved(-100, 100);
    expect(s.offset).toBe(-244);
    expect(s.shown).toBe(0);
  });

  it("leaves it open once the finger has taken it more than halfway open", () => {
    const opening = new Slide(false, 244, 10, 0);
    opening.moved(120, 1000);
    expect(opening.opens(1500)).toBe(false);
    opening.moved(140, 2000);
    expect(opening.opens(2500)).toBe(true);
    const closing = new Slide(true, 244, 200, 0);
    closing.moved(100, 1000);
    expect(closing.opens(1500)).toBe(true);
    closing.moved(70, 2000);
    expect(closing.opens(2500)).toBe(false);
  });

  it("follows a flick, wherever the drawer is", () => {
    const opening = new Slide(false, 244, 10, 0);
    opening.moved(30, 1000);
    opening.moved(70, 1050);
    expect(opening.opens(1060)).toBe(true);
    const closing = new Slide(true, 244, 200, 0);
    closing.moved(180, 1000);
    closing.moved(140, 1050);
    expect(closing.opens(1060)).toBe(false);
    // Back the other way at the end.
    const back = new Slide(false, 244, 10, 0);
    back.moved(200, 1000);
    back.moved(160, 1050);
    expect(back.opens(1060)).toBe(false);
  });

  it("takes a finger that stopped before it let go for one at rest", () => {
    // 15 px in 5 ms, then still until it lets go.
    const opening = new Slide(false, 244, 10, 0);
    opening.moved(25, 1000);
    opening.moved(40, 1005);
    expect(opening.opens(1010)).toBe(true);
    expect(opening.opens(1095)).toBe(false);
    const closing = new Slide(true, 244, 200, 0);
    closing.moved(185, 1000);
    closing.moved(170, 1005);
    expect(closing.opens(1010)).toBe(false);
    expect(closing.opens(1095)).toBe(true);
  });
});

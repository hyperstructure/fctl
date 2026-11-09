import { afterEach, describe, expect, it } from "vitest";
import { atEdge, GUARD, glide, scrollerOf, tap } from "./edge-swipes";

afterEach(() => {
  document.body.replaceChildren();
});

describe("atEdge", () => {
  it("takes a touch near the left or right edge of the screen, and none further in", () => {
    expect(atEdge(0, 390)).toBe(true);
    expect(atEdge(GUARD - 1, 390)).toBe(true);
    expect(atEdge(GUARD, 390)).toBe(false);
    expect(atEdge(195, 390)).toBe(false);
    expect(atEdge(390 - GUARD, 390)).toBe(false);
    expect(atEdge(390 - GUARD + 1, 390)).toBe(true);
    expect(atEdge(389, 390)).toBe(true);
  });
});

describe("glide", () => {
  it("slows down as an iPhone's scroll views do, going on about half a second's worth of its speed", () => {
    let speed = 2;
    let by = 0;
    for (let i = 0; i < 1000 && speed; i++) {
      const g = glide(speed, 16);
      by += g.by;
      speed = g.speed;
    }
    expect(speed).toBe(0);
    expect(by).toBeGreaterThan(950);
    expect(by).toBeLessThan(1000);
  });

  it("goes the way it was flicked, slower each step, as far whatever the frame rate", () => {
    const g = glide(-1, 16);
    expect(g.by).toBeLessThan(0);
    expect(g.by).toBeGreaterThan(-16);
    expect(g.speed).toBeGreaterThan(-1);
    expect(g.speed).toBeLessThan(0);
    const twice = glide(glide(1, 8).speed, 8);
    expect(glide(1, 8).by + twice.by).toBeCloseTo(glide(1, 16).by, 9);
  });

  it("stops once it is still", () => {
    expect(glide(0.01, 16)).toEqual({ by: 0, speed: 0 });
    expect(glide(0, 16)).toEqual({ by: 0, speed: 0 });
  });
});

describe("scrollerOf", () => {
  /** An element that scrolls along `axis` (with room to), with `inner` in it. */
  const scroller = (axis: "x" | "y", inner: Element, room = true) => {
    const el = document.createElement("div");
    el.style.setProperty(axis === "x" ? "overflow-x" : "overflow-y", "auto");
    for (const [k, v] of Object.entries({ scrollWidth: 800, clientWidth: room ? 400 : 800 })) {
      Object.defineProperty(el, k, { value: v });
    }
    for (const [k, v] of Object.entries({ scrollHeight: 900, clientHeight: room ? 300 : 900 })) {
      Object.defineProperty(el, k, { value: v });
    }
    el.append(inner);
    return el;
  };

  it("finds the nearest thing that scrolls the way the finger moves", () => {
    const text = document.createElement("span");
    const row = scroller("x", text);
    const list = scroller("y", row);
    document.body.append(list);
    expect(scrollerOf(text, "x")).toBe(row);
    expect(scrollerOf(text, "y")).toBe(list);
  });

  it("scrolls the page up and down when nothing else scrolls, but never sideways", () => {
    const text = document.createElement("span");
    document.body.append(scroller("y", text, false));
    expect(scrollerOf(text, "y")).toBe(document.scrollingElement);
    expect(scrollerOf(text, "x")).toBeUndefined();
  });

  it("scrolls what is in a dialog, but not the page under it", () => {
    const text = document.createElement("span");
    const dialog = document.createElement("dialog");
    const list = scroller("y", text);
    dialog.append(list);
    document.body.append(dialog);
    expect(scrollerOf(text, "y")).toBe(list);
    const other = document.createElement("span");
    dialog.append(other);
    expect(scrollerOf(other, "y")).toBeUndefined();
  });

  it("scrolls nothing where touch-action keeps fingers to the page's own handling", () => {
    const text = document.createElement("span");
    const track = document.createElement("div");
    track.style.touchAction = "none";
    track.append(text);
    document.body.append(scroller("y", track));
    expect(scrollerOf(text, "y")).toBeUndefined();
    track.style.touchAction = "pan-y";
    expect(scrollerOf(text, "y")).not.toBeUndefined();
    expect(scrollerOf(text, "x")).toBeUndefined();
  });
});

describe("tap", () => {
  it("clicks what it touched, as a finger", () => {
    const button = document.createElement("button");
    const label = document.createElement("span");
    button.append(label);
    document.body.append(button);
    const clicks: MouseEvent[] = [];
    button.addEventListener("click", (e) => clicks.push(e));
    tap(label, { x: 10, y: 300 });
    expect(clicks).toHaveLength(1);
    expect(clicks[0]).toMatchObject({ target: label, clientX: 10, clientY: 300, detail: 1, button: 0 });
    expect((clicks[0] as PointerEvent).pointerType).toBe("touch");
    expect(document.activeElement).not.toBe(button);
  });

  it("follows a link it touched", () => {
    const a = document.createElement("a");
    a.href = "#tapped";
    document.body.append(a);
    let followed = false;
    a.addEventListener("click", () => {
      followed = true;
    });
    tap(a, { x: 380, y: 100 });
    expect(followed).toBe(true);
  });

  it("presses the mouse down and up before it clicks, as a tap does", () => {
    const button = document.createElement("button");
    document.body.append(button);
    const heard: string[] = [];
    for (const type of ["mousedown", "mouseup", "click"])
      button.addEventListener(type, () => heard.push(type));
    tap(button, { x: 380, y: 10 });
    expect(heard).toEqual(["mousedown", "mouseup", "click"]);
  });

  it("closes a dialog that closes on a press and a click beside it", () => {
    // As the new session dialog and the shortcuts sheet do.
    const dialog = document.createElement("dialog");
    document.body.append(dialog);
    let pressedOutside = false;
    let closed = false;
    dialog.addEventListener("mousedown", (e) => {
      pressedOutside = e.target === dialog;
    });
    dialog.addEventListener("click", (e) => {
      if (e.target === dialog && pressedOutside) closed = true;
    });
    tap(dialog, { x: 4, y: 500 });
    expect(closed).toBe(true);
  });

  it("takes the focus from a text field when it taps something else, but not where the page keeps it", () => {
    const field = document.createElement("textarea");
    const button = document.createElement("button");
    const keeps = document.createElement("button");
    keeps.addEventListener("mousedown", (e) => e.preventDefault());
    document.body.append(field, button, keeps);
    field.focus();
    tap(keeps, { x: 380, y: 10 });
    expect(document.activeElement).toBe(field);
    tap(button, { x: 380, y: 10 });
    expect(document.activeElement).not.toBe(field);
    expect(document.activeElement).not.toBe(button);
  });

  it("focuses a text field it touched, and not a checkbox", () => {
    const field = document.createElement("textarea");
    const box = document.createElement("input");
    box.type = "checkbox";
    document.body.append(field, box);
    tap(box, { x: 5, y: 5 });
    expect(document.activeElement).not.toBe(box);
    expect(box.checked).toBe(true);
    tap(field, { x: 5, y: 50 });
    expect(document.activeElement).toBe(field);
  });
});

import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { CodeInput } from "./code-input";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function Code({ onComplete, state }: { onComplete: (v: string) => void; state?: "error" }) {
  const [value, setValue] = useState("");
  return <CodeInput value={value} onChange={setValue} onComplete={onComplete} state={state} label="Code" />;
}

function setup(state?: "error") {
  const onComplete = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<Code onComplete={onComplete} state={state} />));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const input = container.querySelector("input");
  if (!input) throw new Error("no input");
  // What the browser does when someone types or pastes: a new value, then an input event.
  const enter = (text: string) =>
    act(() => {
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const boxes = () =>
    [...container.querySelectorAll("span[aria-hidden] > span.animate-pop")].map((b) => b.textContent);
  return { input, enter, boxes, onComplete };
}

describe("CodeInput", () => {
  it("takes one-time-code suggestions on its single input", () => {
    const s = setup();
    expect(s.input.getAttribute("autocomplete")).toBe("one-time-code");
    expect(s.input.getAttribute("inputmode")).toBe("numeric");
    expect(s.input.getAttribute("aria-label")).toBe("Code");
  });

  it("shows each digit in its box and finishes on the sixth", () => {
    const s = setup();
    s.enter("12");
    expect(s.boxes()).toEqual(["1", "2"]);
    expect(s.onComplete).not.toHaveBeenCalled();
    s.enter("123456");
    expect(s.boxes()).toEqual(["1", "2", "3", "4", "5", "6"]);
    expect(s.onComplete).toHaveBeenCalledTimes(1);
    expect(s.onComplete).toHaveBeenCalledWith("123456");
  });

  it("keeps the digits of a pasted code and no more than six", () => {
    const s = setup();
    s.enter("123 456");
    expect(s.input.value).toBe("123456");
    expect(s.onComplete).toHaveBeenCalledWith("123456");
    const t = setup();
    t.enter("9876543210");
    expect(t.input.value).toBe("987654");
    t.enter("code: ab");
    expect(t.input.value).toBe("");
  });

  it("says when a code was wrong", () => {
    const s = setup("error");
    expect(s.input.getAttribute("aria-invalid")).toBe("true");
  });
});

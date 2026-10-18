import { act, type ReactNode, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { Select, type SelectOption, step, typeahead } from "./select";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const EFFORT: SelectOption[] = [
  { value: "", label: "Claude's default" },
  { value: "low", label: "low" },
  { value: "medium", label: "medium" },
  { value: "high", label: "high", disabled: true },
  { value: "max", label: "max" },
];

describe("step", () => {
  it("passes over disabled options and stops at the ends", () => {
    expect(step(EFFORT, 2, 1)).toBe(4);
    expect(step(EFFORT, 4, -1)).toBe(2);
    expect(step(EFFORT, 4, 1)).toBe(4);
    expect(step(EFFORT, 0, -1)).toBe(0);
    expect(step(EFFORT, 1, 10)).toBe(4);
  });

  it("starts from the first or the last option when none is active", () => {
    expect(step(EFFORT, -1, 1)).toBe(0);
    expect(step(EFFORT, -1, -1)).toBe(4);
    expect(step([{ disabled: true }], -1, 1)).toBe(-1);
  });
});

describe("typeahead", () => {
  // The last one is disabled.
  const labels = ["Viewer", "Editor", "Maintainer", "Owner", "Member", null];

  it("finds the option that starts with what was typed", () => {
    expect(typeahead(labels, "e", -1)).toBe(1);
    expect(typeahead(labels, "MA", 0)).toBe(2);
    expect(typeahead(labels, "me", 0)).toBe(4);
  });

  it("keeps the option while it still matches", () => {
    expect(typeahead(labels, "ma", 2)).toBe(2);
  });

  it("moves on to the next option with a letter typed again, wrapping", () => {
    expect(typeahead(labels, "m", 2)).toBe(4);
    expect(typeahead(labels, "mm", 4)).toBe(2);
  });

  it("finds nothing when nothing matches", () => {
    expect(typeahead(labels, "x", 0)).toBe(-1);
  });
});

const roots: (() => void)[] = [];
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.useRealTimers();
});

/** The text people read in a trigger: the sizers and the outgoing value are hidden. */
const shownIn = (el: Element) =>
  el.querySelector('[role="combobox"] span:not([aria-hidden]) > span:not([aria-hidden])')?.textContent;

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

function Effort({ initial, onChange }: { initial: string; onChange: (v: string) => void }) {
  const [value, setValue] = useState(initial);
  return (
    <Select
      aria-label="Effort"
      value={value}
      options={EFFORT}
      onChange={(v) => {
        setValue(v);
        onChange(v);
      }}
    />
  );
}

function setup(initial = "low") {
  const onChange = vi.fn();
  const container = render(<Effort initial={initial} onChange={onChange} />);
  const trigger = container.querySelector<HTMLButtonElement>('[role="combobox"]');
  if (!trigger) throw new Error("no trigger");
  const press = (key: string) =>
    act(() => {
      trigger.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
    });
  const list = () => container.querySelector('[role="listbox"]');
  const options = () => [...container.querySelectorAll<HTMLElement>('[role="option"]')];
  const active = () => {
    const id = trigger.getAttribute("aria-activedescendant");
    return id ? document.getElementById(id)?.textContent : undefined;
  };
  const shown = () => shownIn(container);
  // In steps, so timers that effects start along the way run too.
  const wait = (ms: number) => {
    for (let t = 0; t < ms; t += 50) act(() => vi.advanceTimersByTime(50));
  };
  return { onChange, trigger, press, list, options, active, shown, wait };
}

describe("Select", () => {
  it("shows the label of its value", () => {
    const s = setup("medium");
    expect(s.shown()).toBe("medium");
    expect(s.trigger.getAttribute("aria-expanded")).toBe("false");
    expect(s.list()).toBeNull();
  });

  it("opens on the selection, moves past disabled options and picks with Enter", () => {
    const s = setup("medium");
    s.press("ArrowDown");
    expect(s.trigger.getAttribute("aria-expanded")).toBe("true");
    expect(s.list()?.getAttribute("role")).toBe("listbox");
    expect(s.active()).toBe("medium");
    s.press("ArrowDown");
    expect(s.active()).toBe("max");
    s.press("Enter");
    expect(s.onChange).toHaveBeenCalledWith("max");
    expect(s.shown()).toBe("max");
    // The picked option flashes, then the list closes.
    s.wait(1000);
    expect(s.trigger.getAttribute("aria-expanded")).toBe("false");
    expect(s.list()).toBeNull();
  });

  it("marks the selected option", () => {
    const s = setup("low");
    s.press("Enter");
    const selected = s.options().filter((o) => o.getAttribute("aria-selected") === "true");
    expect(selected.map((o) => o.textContent)).toEqual(["low"]);
    expect(s.options()[3]?.getAttribute("aria-disabled")).toBe("true");
  });

  it("closes on Escape without changing the value", () => {
    const s = setup("low");
    s.press(" ");
    s.press("End");
    expect(s.active()).toBe("max");
    s.press("Escape");
    expect(s.trigger.getAttribute("aria-expanded")).toBe("false");
    s.wait(1000);
    expect(s.list()).toBeNull();
    expect(s.onChange).not.toHaveBeenCalled();
    expect(s.shown()).toBe("low");
  });

  it("closes on a press elsewhere, but not on one in the list", () => {
    const s = setup("low");
    s.press("Enter");
    act(() => {
      s.options()[0]?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(s.trigger.getAttribute("aria-expanded")).toBe("true");
    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
    });
    expect(s.trigger.getAttribute("aria-expanded")).toBe("false");
    expect(s.onChange).not.toHaveBeenCalled();
  });

  it("jumps to what people type", () => {
    const s = setup("low");
    s.press("m");
    expect(s.trigger.getAttribute("aria-expanded")).toBe("true");
    expect(s.active()).toBe("medium");
    s.press("m");
    expect(s.active()).toBe("max");
    s.wait(1000);
    s.press("c");
    expect(s.active()).toBe("Claude's default");
    s.press("Enter");
    expect(s.onChange).toHaveBeenCalledWith("");
  });

  it("picks an option that is clicked, and not one that is disabled", () => {
    const s = setup("low");
    act(() => s.trigger.click());
    expect(s.trigger.getAttribute("aria-expanded")).toBe("true");
    act(() => s.options()[3]?.click());
    expect(s.onChange).not.toHaveBeenCalled();
    act(() => s.options()[4]?.click());
    expect(s.onChange).toHaveBeenCalledWith("max");
    s.wait(1000);
    expect(s.list()).toBeNull();
  });

  it("does not report the value it already has", () => {
    const s = setup("low");
    s.press("Enter");
    s.press("Enter");
    expect(s.onChange).not.toHaveBeenCalled();
  });

  it("shows the empty value muted, as a placeholder", () => {
    const container = render(<Select aria-label="Effort" value="" options={EFFORT} onChange={() => {}} />);
    const label = container.querySelector(
      '[role="combobox"] span:not([aria-hidden]) > span:not([aria-hidden])',
    );
    expect(label?.textContent).toBe("Claude's default");
    expect(label?.classList.contains("text-ink-4")).toBe(true);
  });

  it("shows a value none of its options has", () => {
    const container = render(
      <Select aria-label="Effort" value="ultra" options={EFFORT} onChange={() => {}} />,
    );
    expect(shownIn(container)).toBe("ultra");
  });
});

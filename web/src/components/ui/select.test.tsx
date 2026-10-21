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

/**
 * A tap as Safari on iOS delivers it. Safari doesn't focus buttons on click, so the
 * tap's mousedown, unless cancelled, takes focus away from the button.
 */
function tap(el: HTMLElement) {
  const mouse = (type: string) => new MouseEvent(type, { bubbles: true, cancelable: true, detail: 1 });
  act(() => {
    el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "touch" }));
    el.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, pointerType: "touch" }));
  });
  act(() => {
    if (el.dispatchEvent(mouse("mousedown"))) (document.activeElement as HTMLElement | null)?.blur();
  });
  act(() => {
    el.dispatchEvent(mouse("mouseup"));
    el.dispatchEvent(mouse("click"));
  });
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

  it("opens on a tap and closes on another, keeping focus", () => {
    const s = setup("low");
    tap(s.trigger);
    expect(s.trigger.getAttribute("aria-expanded")).toBe("true");
    expect(document.activeElement).toBe(s.trigger);
    tap(s.trigger);
    expect(s.trigger.getAttribute("aria-expanded")).toBe("false");
    s.wait(1000);
    expect(s.list()).toBeNull();
    expect(document.activeElement).toBe(s.trigger);
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

  it("lists options in sections by their item, and shows the full label once picked", () => {
    const options: SelectOption[] = [
      {
        value: "fctl",
        label: "hyperstructure / fctl",
        item: "fctl",
        section: { key: "h", label: "hyperstructure" },
      },
      {
        value: "design",
        label: "hyperstructure / design",
        item: "design",
        section: { key: "h", label: "hyperstructure" },
      },
      {
        value: "notes",
        label: "Ada's lab / notes",
        item: "notes",
        section: { key: "a", label: "Ada's lab" },
      },
    ];
    function Picker() {
      const [value, setValue] = useState("fctl");
      return <Select aria-label="Project" value={value} options={options} onChange={setValue} />;
    }
    const container = render(<Picker />);
    const trigger = container.querySelector<HTMLButtonElement>('[role="combobox"]');
    expect(shownIn(container)).toBe("hyperstructure / fctl");
    act(() => trigger?.click());
    const listed = [...container.querySelectorAll('[role="option"]')].map((o) => o.textContent);
    expect(listed).toEqual(["fctl", "design", "notes"]);
    const headings = [...container.querySelectorAll('[role="listbox"] [role="presentation"]')].map(
      (h) => h.textContent,
    );
    expect(headings).toEqual(["hyperstructure", "Ada's lab"]);
    // Typing goes by what the list shows.
    act(() => {
      trigger?.dispatchEvent(new KeyboardEvent("keydown", { key: "n", bubbles: true, cancelable: true }));
    });
    act(() => {
      trigger?.dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true, cancelable: true }));
    });
    act(() => vi.advanceTimersByTime(1000));
    expect(shownIn(container)).toBe("Ada's lab / notes");
  });

  it("shows a value none of its options has", () => {
    const container = render(
      <Select aria-label="Effort" value="ultra" options={EFFORT} onChange={() => {}} />,
    );
    expect(shownIn(container)).toBe("ultra");
  });
});

describe("Select on a phone", () => {
  /** What a phone shows in a `narrow="short"` trigger, for the eyes only. */
  const shortIn = (el: Element) =>
    el.querySelector('[role="combobox"] > span[aria-hidden] > span:not([aria-hidden])');
  const has = (el: Element | null | undefined, cls: string) => !!el?.classList.contains(cls);

  it("shows short labels, without the chevron, and the labels to screen readers", () => {
    const options: SelectOption[] = [
      { value: "", label: "Default model", short: "Default" },
      { value: "opus", label: "Opus" },
    ];
    function Model() {
      const [value, setValue] = useState("");
      return <Select aria-label="Model" narrow="short" value={value} options={options} onChange={setValue} />;
    }
    const container = render(<Model />);
    const trigger = container.querySelector<HTMLButtonElement>('[role="combobox"]');
    const label = trigger?.querySelector(":scope > span:not([aria-hidden])");
    expect(shownIn(container)).toBe("Default model");
    expect(has(label, "max-sm:sr-only")).toBe(true);
    const short = shortIn(container);
    expect(short?.textContent).toBe("Default");
    expect(has(short, "text-ink-4")).toBe(true);
    expect(has(short?.parentElement, "sm:hidden")).toBe(true);
    expect(has(trigger?.querySelector(":scope > svg"), "max-sm:hidden")).toBe(true);
    // An option without a short label shows its label.
    act(() => trigger?.click());
    act(() => container.querySelectorAll<HTMLElement>('[role="option"]')[1]?.click());
    act(() => vi.advanceTimersByTime(1000));
    expect(shortIn(container)?.textContent).toBe("Opus");
    expect(shownIn(container)).toBe("Opus");
  });

  it("leaves the icon alone for an empty short label", () => {
    const options: SelectOption[] = [{ value: "max", label: "Max effort", short: "", icon: <svg /> }];
    const container = render(
      <Select aria-label="Effort" narrow="short" value="max" options={options} onChange={() => {}} />,
    );
    expect(shortIn(container)).toBeNull();
    expect(container.querySelector('[role="combobox"] > span > svg')).not.toBeNull();
    expect(shownIn(container)).toBe("Max effort");
  });

  it("is as wide as its value with fit, not as its widest option", () => {
    const sizers = (narrow?: "fit") => {
      const container = render(
        <Select aria-label="Effort" narrow={narrow} value="low" options={EFFORT} onChange={() => {}} />,
      );
      const spans = container.querySelectorAll('[role="combobox"] .invisible');
      return [spans.length, [...spans].every((s) => has(s, "max-sm:hidden"))];
    };
    expect(sizers("fit")).toEqual([EFFORT.length, true]);
    expect(sizers()).toEqual([EFFORT.length, false]);
  });
});

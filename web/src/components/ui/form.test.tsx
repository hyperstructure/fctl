import { act, createRef, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Button } from "./button";
import { fieldHeight, Input, SecretInput, Textarea, TextareaControls } from "./form";
import { Select } from "./select";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
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
  return { container, rerender: (next: ReactNode) => act(() => root.render(next)) };
}

describe("Input", () => {
  it("is as tall as the Selects and the buttons beside it", () => {
    const { container } = render(
      <>
        <Input aria-label="Email" />
        <Select aria-label="Role" value="a" options={[{ value: "a", label: "Admin" }]} onChange={() => {}} />
        <Button size="field">Add</Button>
      </>,
    );
    const heights = [...container.querySelectorAll("input, button")].map((el) =>
      el.className.split(" ").filter((c) => c.startsWith("h-")),
    );
    expect(heights).toEqual([[fieldHeight], [fieldHeight], [fieldHeight]]);
  });
});

describe("SecretInput", () => {
  it("masks a text field, which browsers don't fill saved passwords into or spellcheck", () => {
    const container = document.createElement("div");
    document.body.append(container);
    const root = createRoot(container);
    act(() => root.render(<SecretInput id="value" className="font-mono" />));
    const input = container.querySelector("input");
    expect(input?.type).toBe("text");
    expect(input?.className).toContain("[-webkit-text-security:disc]");
    expect(input?.className).toContain("font-mono");
    expect(input?.getAttribute("autocomplete")).toBe("off");
    expect(input?.getAttribute("autocapitalize")).toBe("off");
    expect(input?.getAttribute("autocorrect")).toBe("off");
    expect(input?.getAttribute("spellcheck")).toBe("false");
    act(() => root.unmount());
    container.remove();
  });
});

describe("Textarea", () => {
  // Nothing is laid out here: the text is as tall as `scrollHeight` says.
  const style = { lineHeight: "20px", paddingTop: "16px", paddingBottom: "4px" };

  it("grows with its text up to maxRows lines, plus its padding", () => {
    const { container, rerender } = render(<Textarea value="a" readOnly maxRows={3} style={style} />);
    const el = container.querySelector("textarea");
    if (!el) throw new Error("no textarea");
    Object.defineProperty(el, "scrollHeight", { configurable: true, value: 70 });
    rerender(<Textarea value="ab" readOnly maxRows={3} style={style} />);
    expect(el.style.height).toBe("70px");
    Object.defineProperty(el, "scrollHeight", { configurable: true, value: 500 });
    rerender(<Textarea value="abc" readOnly maxRows={3} style={style} />);
    expect(el.style.height).toBe(`${3 * 20 + 16 + 4}px`);
  });

  it("keeps room for minRows lines, plus its padding", () => {
    const { container } = render(<Textarea value="" readOnly minRows={2} style={style} />);
    expect(container.querySelector("textarea")?.style.height).toBe(`${2 * 20 + 16 + 4}px`);
  });
});

describe("TextareaControls", () => {
  function setup() {
    const ref = createRef<HTMLTextAreaElement>();
    const { container } = render(
      <>
        <Textarea ref={ref} bare aria-label="Task" />
        <TextareaControls textarea={ref}>
          <button type="button">Attach files</button>
          <span>⌘↵ to start</span>
        </TextareaControls>
      </>,
    );
    const get = <T extends Element>(selector: string) => {
      const el = container.querySelector<T>(selector);
      if (!el) throw new Error(`no ${selector}`);
      return el;
    };
    const press = (el: Element, button = 0) => {
      const e = new MouseEvent("mousedown", { bubbles: true, cancelable: true, button });
      act(() => {
        el.dispatchEvent(e);
      });
      return e;
    };
    return {
      textarea: get<HTMLTextAreaElement>("textarea"),
      button: get("button"),
      hint: get("span"),
      press,
    };
  }

  it("focuses the textarea on a press between the controls, and keeps the focus from leaving", () => {
    const s = setup();
    expect(s.press(s.hint).defaultPrevented).toBe(true);
    expect(document.activeElement).toBe(s.textarea);
  });

  it("leaves presses on a control, and with other buttons, alone", () => {
    const s = setup();
    expect(s.press(s.button).defaultPrevented).toBe(false);
    expect(s.press(s.hint, 2).defaultPrevented).toBe(false);
    expect(document.activeElement).not.toBe(s.textarea);
  });

  it("leaves presses in a Select's list alone, so they pick", () => {
    const ref = createRef<HTMLTextAreaElement>();
    const onChange = vi.fn();
    const { container } = render(
      <>
        <Textarea ref={ref} bare aria-label="Task" />
        <TextareaControls textarea={ref}>
          <Select
            aria-label="Project"
            value="fctl"
            options={[
              { value: "fctl", label: "fctl" },
              { value: "design", label: "design" },
            ]}
            onChange={onChange}
          />
        </TextareaControls>
      </>,
    );
    const trigger = container.querySelector<HTMLButtonElement>('[role="combobox"]');
    act(() => trigger?.click());
    const option = container.querySelectorAll('[role="option"]')[1];
    if (!trigger || !option) throw new Error("no list");
    act(() => {
      option.dispatchEvent(new MouseEvent("mousedown", { bubbles: true, cancelable: true }));
    });
    expect(document.activeElement).toBe(trigger);
    expect(trigger.getAttribute("aria-expanded")).toBe("true");
    act(() => {
      option.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, detail: 1 }));
    });
    expect(onChange).toHaveBeenCalledWith("design");
  });
});

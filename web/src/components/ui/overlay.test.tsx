import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Menu, MenuItem, MenuSeparator } from "./overlay";

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
  return container;
}

/** A menu like the sidebar's account menu. */
function setup(props: { side?: "top" | "bottom"; align?: "start" | "end" | "stretch" } = {}) {
  const signOut = vi.fn();
  const container = render(
    <Menu
      {...props}
      trigger={(toggle, open) => (
        <button type="button" aria-haspopup="menu" aria-expanded={open} onClick={toggle}>
          Dev Owner
        </button>
      )}
    >
      {(close) => (
        <>
          <MenuItem onClick={close}>Claude and tokens</MenuItem>
          <MenuSeparator />
          <MenuItem
            onClick={() => {
              close();
              signOut();
            }}
          >
            Sign out
          </MenuItem>
        </>
      )}
    </Menu>,
  );
  const trigger = container.querySelector("button") as HTMLButtonElement;
  const menu = () => container.querySelector<HTMLElement>('[role="menu"]');
  const item = (name: string) =>
    [...container.querySelectorAll<HTMLElement>('[role="menuitem"]')].find((i) => i.textContent === name);
  const open = () => act(() => trigger.click());
  return { signOut, trigger, menu, item, open };
}

describe("Menu", () => {
  it("opens and closes from its trigger, which says whether it is open", () => {
    const m = setup();
    expect(m.menu()).toBeNull();
    expect(m.trigger.getAttribute("aria-expanded")).toBe("false");
    m.open();
    expect(m.menu()).not.toBeNull();
    expect(m.trigger.getAttribute("aria-expanded")).toBe("true");
    act(() => m.trigger.click());
    expect(m.menu()).toBeNull();
    expect(m.trigger.getAttribute("aria-expanded")).toBe("false");
  });

  it("runs an item and closes", () => {
    const m = setup();
    m.open();
    act(() => m.item("Sign out")?.click());
    expect(m.signOut).toHaveBeenCalledOnce();
    expect(m.menu()).toBeNull();
  });

  it("closes on Escape and on a press elsewhere, but not on one in the menu", () => {
    const m = setup();
    m.open();
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(m.menu()).toBeNull();
    m.open();
    act(() => {
      m.item("Sign out")?.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(m.menu()).not.toBeNull();
    act(() => {
      document.body.dispatchEvent(new MouseEvent("mousedown", { bubbles: true }));
    });
    expect(m.menu()).toBeNull();
    expect(m.signOut).not.toHaveBeenCalled();
  });

  // happy-dom lays nothing out: these check the classes that place the menu.
  it("opens under its trigger, or over it when it is at the bottom of the window", () => {
    const under = setup();
    under.open();
    expect(under.menu()?.className).toMatch(/\btop-full\b/);
    expect(under.menu()?.className).not.toMatch(/\bbottom-full\b/);
    const over = setup({ side: "top" });
    over.open();
    expect(over.menu()?.className).toMatch(/\bbottom-full\b/);
    expect(over.menu()?.className).not.toMatch(/\btop-full\b/);
  });

  it("lines up with an edge of its trigger, or both", () => {
    const end = setup();
    end.open();
    expect(end.menu()?.className).toMatch(/\bright-0\b/);
    const start = setup({ align: "start" });
    start.open();
    expect(start.menu()?.className).toMatch(/\bleft-0\b/);
    const stretch = setup({ align: "stretch" });
    stretch.open();
    expect(stretch.menu()?.className).toMatch(/\binset-x-0\b/);
  });
});

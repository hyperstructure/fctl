import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { IconPicker, type PickerProject } from "./icon-picker";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function setup(project: PickerProject) {
  const onSave = vi.fn<(icon: G.ProjectIcon | null) => void>();
  const onClose = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<IconPicker open project={project} onSave={onSave} onClose={onClose} />));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const dialog = () => document.querySelector("dialog") as HTMLDialogElement;
  const button = (name: string) =>
    [...dialog().querySelectorAll("button")].find((b) => b.textContent?.trim().startsWith(name));
  const option = (label: string) =>
    dialog().querySelector<HTMLButtonElement>(`[role="option"][aria-label="${label}"]`);
  const options = () => [...dialog().querySelectorAll<HTMLButtonElement>('[role="option"]')];
  const search = (text: string) =>
    act(() => {
      const input = dialog().querySelector<HTMLInputElement>('input[aria-label="Search icons"]');
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const key = (el: Element | null | undefined, k: string, init: KeyboardEventInit = {}) =>
    act(() => {
      el?.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true, ...init }));
    });
  return { onSave, onClose, dialog, button, option, options, search, key };
}

describe("choosing a project's icon", () => {
  it("starts on the icon that fits, and saves nothing until something changes", () => {
    const p = setup({ id: "prj_shop", name: "Shop", description: "🥕 The web shop" });
    expect(p.dialog().textContent).toContain("Icon for Shop");
    expect(p.option("Store")?.getAttribute("aria-selected")).toBe("false");
    expect(p.button("Save")?.disabled).toBe(true);
    act(() => p.button("Cancel")?.click());
    expect(p.onClose).toHaveBeenCalled();
    expect(p.onSave).not.toHaveBeenCalled();
  });

  it("saves the icon, color and style chosen", () => {
    const p = setup({ id: "prj_shop", name: "Shop" });
    act(() => p.option("Rocket")?.click());
    act(() => p.dialog().querySelector<HTMLButtonElement>('[role="radio"][aria-label="Pink"]')?.click());
    act(() =>
      [...p.dialog().querySelectorAll<HTMLButtonElement>('[role="radio"]')]
        .find((b) => b.title === "Solid")
        ?.click(),
    );
    expect(p.option("Rocket")?.getAttribute("aria-selected")).toBe("true");
    act(() => p.button("Save")?.click());
    expect(p.onSave).toHaveBeenCalledWith({ name: "rocket", color: "pink", style: "solid" });
  });

  it("goes back to the icon that fits", () => {
    const p = setup({
      id: "prj_shop",
      name: "Shop",
      icon: { name: "rocket", color: "pink", style: "solid" },
    });
    expect(p.option("Rocket")?.getAttribute("aria-selected")).toBe("true");
    act(() => p.button("Use the one that fits")?.click());
    act(() => p.button("Save")?.click());
    expect(p.onSave).toHaveBeenCalledWith(null);
  });

  it("searches, and Enter in the search picks the best match", () => {
    const p = setup({ id: "prj_shop", name: "Shop" });
    p.search("serv");
    expect(p.dialog().textContent).toMatch(/\d+ icons/);
    expect(p.options()[0]?.getAttribute("aria-label")).toBe("Server rack"); // server-stack
    p.key(p.dialog().querySelector('input[aria-label="Search icons"]'), "Enter");
    expect(p.options()[0]?.getAttribute("aria-selected")).toBe("true");
    p.search("no such icon at all");
    expect(p.dialog().textContent).toContain("No icon here is called");
  });

  it("moves through the icons with the arrows, and picks one with Enter", () => {
    const p = setup({ id: "prj_shop", name: "Shop" });
    const listbox = p.dialog().querySelector('[role="listbox"]');
    const [first, second] = p.options();
    act(() => first?.focus());
    p.key(listbox, "ArrowRight");
    expect(document.activeElement).toBe(second);
    p.key(listbox, "Enter");
    expect(second?.getAttribute("aria-selected")).toBe("true");
    // ⌘Enter saves from the grid, once.
    p.key(listbox, "Enter", { metaKey: true });
    expect(p.onSave).toHaveBeenCalledTimes(1);
    // The second icon that fits a shop.
    expect(p.onSave).toHaveBeenCalledWith({ name: "bag", color: "ink", style: "outline" });
  });
});

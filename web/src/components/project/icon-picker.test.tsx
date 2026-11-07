import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { IconPicker, type PickerProject } from "./icon-picker";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
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
  const radio = (label: string) =>
    dialog().querySelector<HTMLButtonElement>(`[role="radio"][aria-label^="${label}"]`);
  // The popover for a color of its own, which joins the top layer from inside the dialog.
  const popover = () => dialog().querySelector<HTMLElement>('[role="dialog"][aria-label="Custom color"]');
  const hex = () => popover()?.querySelector<HTMLInputElement>('input[aria-label="Hex color"]');
  const typeHex = (text: string) =>
    act(() => {
      const input = hex();
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const save = () => button("Save");
  return {
    onSave,
    onClose,
    dialog,
    button,
    option,
    options,
    search,
    key,
    radio,
    popover,
    hex,
    typeHex,
    save,
  };
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

  it("fits the screen: the icons' list gives way, so that Save stays in view", () => {
    const p = setup({ id: "prj_shop", name: "Shop" });
    // The dialog is a column no taller than the screen, which its body fills.
    expect(p.dialog().className.split(" ")).toEqual(expect.arrayContaining(["open:flex", "open:flex-col"]));
    const list = p.dialog().querySelector('[role="listbox"]');
    expect(list?.className.split(" ")).toEqual(expect.arrayContaining(["h-[min(44vh,340px)]", "min-h-28"]));
    // Starting from what it holds: Safari 26 takes flex-1's 0% basis for 0 there, which left the
    // dialog as tall as its padding.
    const column = list?.parentElement?.className.split(" ");
    expect(column).toEqual(expect.arrayContaining(["min-h-0", "flex-auto", "flex-col"]));
    expect(column).not.toContain("flex-1");
    // On a phone, the dialog's × closes it in Cancel's place.
    expect(p.button("Cancel")?.className).toContain("max-sm:hidden");
    expect(p.dialog().querySelector('button[aria-label="Close"]')).not.toBeNull();
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

describe("a color of the icon's own", () => {
  it("is chosen in a popover beside the workspace colors, and saved as its hex", () => {
    const p = setup({ id: "prj_shop", name: "Shop" });
    expect(p.radio("Custom color")?.getAttribute("aria-checked")).toBe("false");
    expect(p.popover()).toBeNull();
    act(() => p.radio("Custom color")?.click());
    // From an icon in ink, it starts from a warm orange, and the icon takes it at once.
    expect(p.popover()?.dataset.state).toBe("open");
    expect(p.hex()?.value).toBe("FF8A1F");
    expect(p.radio("Custom color")?.getAttribute("aria-checked")).toBe("true");
    expect(p.radio("Ink")?.getAttribute("aria-checked")).toBe("false");
    // Six digits are a color as they're typed, pasted with their # or not.
    p.typeHex("#2A9D8F");
    expect(p.radio("Custom color")?.getAttribute("aria-label")).toBe("Custom color: teal, #2A9D8F");
    act(() => p.save()?.click());
    expect(p.onSave).toHaveBeenCalledWith({ name: "store", style: "outline", hex: "#2a9d8f" });
  });

  it("closes on Escape and Enter, which leave the picker open", () => {
    const p = setup({ id: "prj_shop", name: "Shop" });
    act(() => p.radio("Custom color")?.click());
    p.key(p.hex(), "Escape");
    expect(p.popover()?.dataset.state).toBe("closed");
    expect(p.onClose).not.toHaveBeenCalled();
    expect(document.activeElement).toBe(p.radio("Custom color"));
    // Opened from the keyboard, the focus goes to its hex, where Enter closes it.
    act(() => p.radio("Custom color")?.dispatchEvent(new MouseEvent("click", { bubbles: true, detail: 0 })));
    expect(document.activeElement).toBe(p.hex());
    p.typeHex("f0a");
    p.key(p.hex(), "Enter");
    expect(p.popover()?.dataset.state).toBe("closed");
    expect(p.radio("Custom color")?.getAttribute("aria-label")).toBe("Custom color: magenta, #FF00AA");
    expect(p.onClose).not.toHaveBeenCalled();
    // A press on its padding gives the popover itself the focus, so that Escape still closes it alone.
    act(() => p.radio("Custom color")?.click());
    expect(p.popover()?.tabIndex).toBe(-1);
    act(() => p.popover()?.focus());
    expect(document.activeElement).toBe(p.popover());
    p.key(p.popover(), "Escape");
    expect(p.popover()?.dataset.state).toBe("closed");
    expect(p.onClose).not.toHaveBeenCalled();
  });

  it("goes back to the color it had when a hex isn't one", () => {
    const p = setup({ id: "prj_shop", name: "Shop", icon: { name: "rocket", hex: "#2a9d8f" } });
    act(() => p.radio("Custom color")?.click());
    p.typeHex("2a9");
    act(() => p.hex()?.blur());
    // Three digits are #rgb once they're done.
    expect(p.hex()?.value).toBe("22AA99");
    act(() => p.hex()?.focus());
    p.typeHex("2a");
    act(() => p.hex()?.blur());
    expect(p.hex()?.value).toBe("22AA99");
  });

  it("is kept while a workspace color is tried, to go back to", () => {
    const p = setup({
      id: "prj_shop",
      name: "Shop",
      icon: { name: "rocket", color: "blue", hex: "#2a9d8f" },
    });
    expect(p.radio("Custom color: teal")?.getAttribute("aria-checked")).toBe("true");
    expect(p.radio("Blue")?.getAttribute("aria-checked")).toBe("false");
    expect(p.save()?.disabled).toBe(true);
    act(() => p.radio("Pink")?.click());
    expect(p.radio("Pink")?.getAttribute("aria-checked")).toBe("true");
    expect(p.save()?.disabled).toBe(false);
    act(() => p.radio("Custom color: teal")?.click());
    expect(p.hex()?.value).toBe("2A9D8F");
    expect(p.save()?.disabled).toBe(true);
    act(() => p.radio("Pink")?.click());
    act(() => p.save()?.click());
    expect(p.onSave).toHaveBeenCalledWith({ name: "rocket", color: "pink", style: "outline" });
  });

  it("starts from the workspace color the icon is in", () => {
    const p = setup({ id: "prj_shop", name: "Shop", icon: { name: "rocket", color: "yellow" } });
    act(() => p.radio("Custom color")?.click());
    expect(p.hex()?.value).toBe("FFFF5D");
  });

  it("moves with the arrows, a tenth at a time with Shift", () => {
    const p = setup({ id: "prj_shop", name: "Shop", icon: { name: "rocket", hex: "#ff0000" } });
    act(() => p.radio("Custom color")?.click());
    const hue = p.popover()?.querySelector('[role="slider"][aria-label="Hue"]');
    p.key(hue, "ArrowRight", { shiftKey: true });
    expect(hue?.getAttribute("aria-valuenow")).toBe("10");
    expect(p.hex()?.value).toBe("FF2A00");
    const area = p.popover()?.querySelector('[role="slider"][aria-label="Saturation and brightness"]');
    p.key(area, "ArrowDown", { shiftKey: true });
    expect(area?.getAttribute("aria-valuetext")).toBe("red: saturation 100%, brightness 90%");
  });

  it("offers the colors of their own that the workspace's other projects have", () => {
    const project = (id: string, name: string, workspace: string, hex?: string) =>
      ({
        id,
        name,
        workspace: { id: workspace },
        icon: hex ? { name: "cube", hex } : undefined,
      }) as G.Project;
    queryClient.setQueryData(keys.projects, {
      projects: [
        project("prj_web", "Web", "ws_acme", "#ff8a1f"),
        project("prj_app", "App", "ws_acme", "#2a9d8f"),
        project("prj_api", "API", "ws_acme", "#2a9d8f"),
        project("prj_other", "Other", "ws_else", "#e63946"),
        project("prj_shop", "Shop", "ws_acme", "#123456"),
      ],
    } as G.ProjectList);
    const p = setup({ id: "prj_shop", name: "Shop", workspace: { id: "ws_acme", name: "Acme" } });
    act(() => p.radio("Custom color")?.click());
    expect(p.popover()?.textContent).toContain("In Acme");
    const chips = [...(p.popover()?.querySelectorAll<HTMLButtonElement>("fieldset button") ?? [])];
    expect(chips.map((c) => c.title)).toEqual(["#2A9D8F: App, API", "#FF8A1F: Web"]);
    act(() => chips[0]?.click());
    expect(p.hex()?.value).toBe("2A9D8F");
    expect(chips[0]?.getAttribute("aria-pressed")).toBe("true");
  });
});

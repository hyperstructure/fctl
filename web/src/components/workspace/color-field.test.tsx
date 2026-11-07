import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { type ColorChange, WorkspaceColorField } from "./color-field";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

type W = Pick<G.Workspace, "id" | "name" | "color" | "hex" | "avatar_url">;
const acme: W = { id: "ws_acme", name: "Acme", color: "yellow" };

function setup(w: W) {
  const save = vi.fn<(change: ColorChange) => Promise<unknown>>(() => Promise.resolve());
  const onPreview = vi.fn<(hex: string | undefined) => void>();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = (next: W) =>
    act(() => root.render(<WorkspaceColorField w={next} save={save} onPreview={onPreview} />));
  render(w);
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const button = (label: string) =>
    container.querySelector<HTMLButtonElement>(`button[aria-label^="${label}"]`) ?? undefined;
  const custom = () => button("Custom color");
  const popover = () => document.querySelector<HTMLElement>('[role="dialog"][aria-label="Custom color"]');
  const hex = () => popover()?.querySelector<HTMLInputElement>('input[aria-label="Hex color"]');
  const typeHex = (text: string) =>
    act(() => {
      const input = hex();
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input?.dispatchEvent(new Event("input", { bubbles: true }));
    });
  const key = (el: Element | null | undefined, k: string) =>
    act(() => {
      el?.dispatchEvent(new KeyboardEvent("keydown", { key: k, bubbles: true }));
    });
  // Saves settle on later ticks, one after another.
  const settle = () => act(async () => {});
  return { save, onPreview, render, button, custom, popover, hex, typeHex, key, settle };
}

describe("a workspace's color", () => {
  it("saves a brand color as it's clicked", async () => {
    const f = setup(acme);
    expect(f.button("Yellow")?.getAttribute("aria-pressed")).toBe("true");
    expect(f.custom()?.getAttribute("aria-pressed")).toBe("false");
    act(() => f.button("Pink")?.click());
    await f.settle();
    expect(f.save).toHaveBeenCalledWith({ color: "pink" });
  });

  it("shows a color of its own as it's chosen, and saves it as the popover closes", async () => {
    const f = setup(acme);
    act(() => f.custom()?.click());
    expect(f.popover()?.dataset.state).toBe("open");
    // It starts from the brand's yellow, which isn't a change yet.
    expect(f.hex()?.value).toBe("FFFF5D");
    expect(f.onPreview).toHaveBeenLastCalledWith(undefined);
    f.typeHex("#2a9d8f");
    expect(f.onPreview).toHaveBeenLastCalledWith("#2a9d8f");
    expect(f.custom()?.getAttribute("aria-label")).toBe("Custom color: teal, #2A9D8F");
    expect(f.custom()?.getAttribute("aria-pressed")).toBe("true");
    expect(f.button("Yellow")?.getAttribute("aria-pressed")).toBe("false");
    expect(f.popover()?.textContent).toContain("Its initials in black");
    expect(f.save).not.toHaveBeenCalled();
    f.key(f.hex(), "Escape");
    expect(f.popover()?.dataset.state).toBe("closed");
    await f.settle();
    expect(f.save).toHaveBeenCalledExactlyOnceWith({ hex: "#2a9d8f" });
    // Once the workspace has it, the avatar above shows the workspace's own.
    f.render({ ...acme, color: "blue", hex: "#2a9d8f" });
    expect(f.onPreview).toHaveBeenLastCalledWith(undefined);
    expect(f.custom()?.getAttribute("aria-pressed")).toBe("true");
  });

  it("saves nothing when the popover closes without a change", async () => {
    const f = setup({ ...acme, color: "blue", hex: "#1d3557" });
    expect(f.custom()?.getAttribute("aria-label")).toBe("Custom color: dark blue, #1D3557");
    act(() => f.custom()?.click());
    expect(f.hex()?.value).toBe("1D3557");
    expect(f.popover()?.textContent).toContain("Its initials in white");
    f.key(f.hex(), "Escape");
    await f.settle();
    expect(f.save).not.toHaveBeenCalled();
  });

  it("takes a brand color pressed while the popover is open instead", async () => {
    const f = setup(acme);
    act(() => f.custom()?.click());
    f.typeHex("2a9d8f");
    act(() => {
      f.button("Pink")?.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      f.button("Pink")?.click();
    });
    await f.settle();
    expect(f.popover()?.dataset.state).toBe("closed");
    expect(f.save).toHaveBeenCalledExactlyOnceWith({ color: "pink" });
    expect(f.onPreview).toHaveBeenLastCalledWith(undefined);
  });

  it("goes back to a color of its own it had in one press", async () => {
    const f = setup({ ...acme, color: "blue", hex: "#2a9d8f" });
    f.render({ ...acme, color: "pink" });
    // Not chosen, but kept on its swatch.
    expect(f.custom()?.getAttribute("aria-pressed")).toBe("false");
    expect(f.custom()?.getAttribute("aria-label")).toBe("Custom color: teal, #2A9D8F");
    act(() => f.custom()?.click());
    expect(f.onPreview).toHaveBeenLastCalledWith("#2a9d8f");
    f.key(f.hex(), "Enter");
    await f.settle();
    expect(f.save).toHaveBeenCalledExactlyOnceWith({ hex: "#2a9d8f" });
  });

  it("offers the colors of their own its projects have", () => {
    const project = (id: string, workspace: string, hex?: string) =>
      ({
        id,
        name: id,
        workspace: { id: workspace },
        icon: hex ? { name: "store", hex } : undefined,
      }) as G.Project;
    queryClient.setQueryData<G.ProjectList>(keys.projects, {
      projects: [
        project("shop", "ws_acme", "#e76f51"),
        project("api", "ws_acme", "#e76f51"),
        project("docs", "ws_acme"),
        project("lab", "ws_other", "#264653"),
      ],
    });
    const f = setup(acme);
    act(() => f.custom()?.click());
    expect(f.popover()?.textContent).toContain("In Acme");
    const chips = [...(f.popover()?.querySelectorAll<HTMLButtonElement>("fieldset button") ?? [])];
    expect(chips.map((c) => c.title)).toEqual(["#E76F51: shop, api"]);
    act(() => chips[0]?.click());
    expect(f.onPreview).toHaveBeenLastCalledWith("#e76f51");
  });

  it("says that an image shows instead", () => {
    const f = setup({ ...acme, avatar_url: "/api/v1/workspaces/ws_acme/avatar?v=1" });
    expect(f.custom()?.closest("fieldset")?.textContent).toContain("Its image shows instead");
  });
});

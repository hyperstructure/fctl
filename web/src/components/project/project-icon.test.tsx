import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { Glyph, ProjectIcon, ProjectMark } from "./project-icon";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: React.ReactNode) {
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

describe("a project's icon", () => {
  it("draws a glyph's shapes, and a solid one's mask with an id of its own", () => {
    const one = render(<Glyph name="rocket" style="solid" />);
    const two = render(<Glyph name="rocket" style="solid" />);
    const id = (c: HTMLElement) => c.querySelector("mask")?.id;
    expect(id(one)).toMatch(/^pi/);
    expect(id(one)).not.toBe(id(two));
    expect(one.querySelector("[mask]")?.getAttribute("mask")).toBe(`url(#${id(one)})`);
  });

  it("draws an icon from a newer control plane rather than failing", () => {
    // A color and a style this version doesn't know, from the server.
    const icon = { name: "rocket", color: "teal", style: "neon" } as unknown as G.ProjectIcon;
    const c = render(<ProjectMark project={{ name: "Shop", icon }} tile={24} />);
    expect(c.querySelector("svg path")).not.toBeNull();
    const c2 = render(
      <ProjectIcon
        icon={
          { name: "rocket", color: "teal", style: "neon" } as unknown as G.ProjectIcon & {
            name: string;
            color: G.WorkspaceColor;
            style: G.IconStyle;
          }
        }
        tile={24}
      />,
    );
    expect(c2.querySelector("svg path")).not.toBeNull();
  });
});

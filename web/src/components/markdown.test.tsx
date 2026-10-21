import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Gallery } from "@/components/ui/gallery";
import { ImageSizes, Markdown } from "./markdown";

vi.mock("@/components/ui", async () => ({
  ...(await vi.importActual<typeof import("@/components/ui/gallery")>("@/components/ui/gallery")),
  CopyButton: () => null,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

/**
 * `sizes`: the images whose size the page knows, as a session's published
 * screenshots. `around`: what the page has around the answer.
 */
function render(
  text: string,
  sizes = new Map<string, { width?: number; height?: number }>(),
  around = (answer: ReactNode) => answer,
) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      around(
        <ImageSizes value={sizes}>
          <Markdown text={text} />
        </ImageSizes>,
      ),
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

const src = "https://app.fctl.test/api/v1/published/abc/after.png";

describe("images in the agent's answers", () => {
  it("show at full size over the page when clicked", () => {
    const c = render(`The new header:\n\n![after.png](${src})`);
    const button = c.querySelector<HTMLButtonElement>('button[aria-label="View after.png"]');
    expect(button?.querySelector("img")?.getAttribute("src")).toBe(src);
    expect(document.querySelector("dialog")).toBeNull();
    act(() => button?.click());
    expect(document.querySelector('dialog[aria-label="after.png"] img')?.getAttribute("src")).toBe(src);
  });

  it("step from one to the next in the gallery around them", () => {
    const before = src.replace("after", "before");
    const c = render(`![before.png](${before}) ![after.png](${src})`, undefined, (answer) => (
      <Gallery>{answer}</Gallery>
    ));
    act(() => c.querySelector<HTMLButtonElement>('button[aria-label="View before.png"]')?.click());
    const dialog = document.querySelector("dialog") as HTMLDialogElement;
    expect(dialog.querySelector("p[aria-live]")?.textContent).toBe("before.png: 1 of 2");
    act(() => {
      dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(dialog.getAttribute("aria-label")).toBe("after.png");
  });

  it("follow their link when they are one", () => {
    const c = render(`[![after.png](${src})](https://github.com/acme/web/pull/7)`);
    expect(c.querySelector("button")).toBeNull();
    expect(c.querySelector("a img")?.getAttribute("src")).toBe(src);
  });

  it("take the room of a published screenshot before it loads, and load as the reader nears it", () => {
    const sizes = new Map([[src, { width: 1440, height: 900 }]]);
    for (const text of [
      `![after.png](${src})`,
      `[![after.png](${src})](https://github.com/acme/web/pull/7)`,
    ]) {
      const img = render(text, sizes).querySelector("img");
      expect([img?.getAttribute("width"), img?.getAttribute("height")]).toEqual(["1440", "900"]);
      expect(img?.getAttribute("loading")).toBe("lazy");
    }
  });

  it("load with the page when their size isn't known, so that they can't push it as the reader scrolls up", () => {
    const other = render("![a diagram](https://app.fctl.test/diagram.png)").querySelector("img");
    expect(other?.getAttribute("loading")).toBeNull();
    expect(other?.hasAttribute("width")).toBe(false);
    // Without both sides, the browser can't keep the room.
    const half = render(`![after.png](${src})`, new Map([[src, { width: 1440 }]])).querySelector("img");
    expect(half?.getAttribute("loading")).toBeNull();
    expect(half?.hasAttribute("width")).toBe(false);
  });

  it("show only their text from anywhere but https", () => {
    const c = render("![after.png](http://example.com/after.png)");
    expect(c.querySelector("img")).toBeNull();
    expect(c.textContent).toBe("after.png");
  });
});

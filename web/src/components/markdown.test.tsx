import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Markdown } from "./markdown";

vi.mock("@/components/ui", () => ({
  CopyButton: () => null,
  Lightbox: ({ file, src }: { file: { name: string }; src: string }) => (
    <dialog open aria-label={file.name} data-src={src} />
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(text: string) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<Markdown text={text} />));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const src = "https://app.fctl.test/api/v1/published/abc/after.png";

describe("images in the agent's answers", () => {
  it("show at full size over the page when clicked", () => {
    const c = render(`The new header:\n\n![after.png](${src})`);
    const button = c.querySelector<HTMLButtonElement>('button[aria-label="View after.png"]');
    expect(button?.querySelector("img")?.getAttribute("src")).toBe(src);
    expect(c.querySelector("dialog")).toBeNull();
    act(() => button?.click());
    expect(c.querySelector('dialog[aria-label="after.png"]')?.getAttribute("data-src")).toBe(src);
  });

  it("follow their link when they are one", () => {
    const c = render(`[![after.png](${src})](https://github.com/acme/web/pull/7)`);
    expect(c.querySelector("button")).toBeNull();
    expect(c.querySelector("a img")?.getAttribute("src")).toBe(src);
  });

  it("show only their text from anywhere but https", () => {
    const c = render("![after.png](http://example.com/after.png)");
    expect(c.querySelector("img")).toBeNull();
    expect(c.textContent).toBe("after.png");
  });
});

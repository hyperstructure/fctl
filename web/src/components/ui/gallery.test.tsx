import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Gallery, useLightbox } from "./gallery";
import type { LightboxItem } from "./lightbox";

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
  return (next: ReactNode) => act(() => root.render(next));
}

const picture = (name: string): LightboxItem => ({
  file: { name, media_type: "image/png" },
  src: `/files/${name}`,
});

/** A thumbnail on the page that shows `name` at full size, unless it has nothing to show. */
function Thumb({ name, empty }: { name: string; empty?: boolean }) {
  const { ref, open, lightbox } = useLightbox<HTMLButtonElement>(empty ? undefined : picture(name));
  return (
    <>
      <button ref={ref} type="button" onClick={open} aria-label={`View ${name}`} />
      {lightbox}
    </>
  );
}

const dialog = () => document.querySelector("dialog");
const shown = () => dialog()?.getAttribute("aria-label");
/** Which of how many it shows, as screen readers hear it. */
const where = () => dialog()?.querySelector("p[aria-live]")?.textContent;
const view = (name: string) =>
  act(() => document.querySelector<HTMLButtonElement>(`button[aria-label="View ${name}"]`)?.click());
const key = (key: string) =>
  act(() => {
    dialog()?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  });

describe("Gallery", () => {
  it("steps through its pictures in the order the page shows them", () => {
    // b.png comes after the others, between them.
    const page = (b: boolean) => (
      <Gallery>
        <Thumb name="a.png" />
        {b && <Thumb name="b.png" />}
        <Thumb name="c.png" />
      </Gallery>
    );
    const rerender = render(page(false));
    rerender(page(true));
    view("a.png");
    expect(shown()).toBe("a.png");
    expect(where()).toBe("a.png: 1 of 3");
    key("ArrowRight");
    expect(shown()).toBe("b.png");
    key("ArrowRight");
    expect(where()).toBe("c.png: 3 of 3");
  });

  it("takes in pictures that come while it is open, and lets go of those that go", () => {
    const page = (names: string[]) => (
      <Gallery>
        {names.map((n) => (
          <Thumb key={n} name={n} />
        ))}
      </Gallery>
    );
    const rerender = render(page(["a.png", "b.png"]));
    view("b.png");
    expect(where()).toBe("b.png: 2 of 2");
    rerender(page(["a.png", "b.png", "c.png"]));
    expect(where()).toBe("b.png: 2 of 3");
    key("ArrowRight");
    expect(shown()).toBe("c.png");
    rerender(page(["b.png", "c.png"]));
    expect(where()).toBe("c.png: 2 of 2");
  });

  it("leaves out what has nothing to show", () => {
    render(
      <Gallery>
        <Thumb name="a.png" />
        <Thumb name="b.png" empty />
        <Thumb name="c.png" />
      </Gallery>,
    );
    view("b.png");
    expect(dialog()).toBeNull();
    view("a.png");
    expect(where()).toBe("a.png: 1 of 2");
  });

  it("keeps showing a picture the page shows somewhere else now, or alone once it is gone", () => {
    // A message that was sending, sent: its pictures show again, in new places.
    const page = (b?: string) => (
      <Gallery>
        <Thumb name="a.png" />
        {b && <Thumb key={b} name="b.png" />}
        <Thumb name="c.png" />
      </Gallery>
    );
    const rerender = render(page("sending"));
    view("b.png");
    rerender(page("sent"));
    expect(where()).toBe("b.png: 2 of 3");
    key("ArrowLeft");
    expect(shown()).toBe("a.png");
    key("ArrowRight");
    rerender(page());
    expect(shown()).toBe("b.png");
    expect(where()).toBeUndefined();
  });

  it("closes, and opens again on another picture", () => {
    render(
      <Gallery>
        <Thumb name="a.png" />
        <Thumb name="b.png" />
      </Gallery>,
    );
    view("a.png");
    act(() => dialog()?.close());
    expect(dialog()).toBeNull();
    view("b.png");
    expect(where()).toBe("b.png: 2 of 2");
  });

  it("shows a picture on its own outside a gallery", () => {
    render(
      <>
        <Thumb name="a.png" />
        <Thumb name="b.png" />
      </>,
    );
    view("a.png");
    expect(shown()).toBe("a.png");
    expect(where()).toBeUndefined();
    key("ArrowRight");
    expect(shown()).toBe("a.png");
    act(() => dialog()?.close());
    expect(dialog()).toBeNull();
  });
});

import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Lightbox, type LightboxFile, plainClick } from "./lightbox";

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
}

const shot: LightboxFile = {
  name: "after.png",
  media_type: "image/png",
  size: 245_300,
  width: 1440,
  height: 900,
};

function open(file: LightboxFile = shot, download?: boolean) {
  const onClose = vi.fn();
  render(<Lightbox file={file} src="/files/after.png" download={download} onClose={onClose} />);
  const dialog = document.querySelector("dialog") as HTMLDialogElement;
  /** A press and release on `el`, ending on `end` if it moves. */
  const press = (el: Element, end: Element = el) =>
    act(() => {
      el.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true }));
      end.dispatchEvent(new MouseEvent("click", { bubbles: true }));
    });
  return { onClose, dialog, press };
}

describe("Lightbox", () => {
  it("shows the picture with its name and size over the page", () => {
    const { dialog } = open();
    expect(dialog.open).toBe(true);
    expect(dialog.getAttribute("aria-label")).toBe("after.png");
    expect(dialog.querySelector("img")?.getAttribute("src")).toBe("/files/after.png");
    expect(dialog.querySelector("figcaption")?.textContent).toBe("after.png1440×900 · 245 KB");
    // The X comes first, at the top right: Tab reaches it first.
    expect(dialog.querySelector("button")?.getAttribute("aria-label")).toBe("Close");
    expect(dialog.querySelector("button")?.className).toMatch(/\bfixed top-3 right-3\b/);
  });

  it("focuses itself as it opens, not the X, and shows no ring", () => {
    // Safari rings what a dialog focuses as it opens, even after a tap.
    const { dialog } = open();
    expect(dialog.hasAttribute("autofocus")).toBe(true);
    // The dialog fills the window: a ring around it would only show at the edges.
    expect(dialog.className).toMatch(/\boutline-none\b/);
  });

  it("closes with the X", () => {
    const { dialog, onClose } = open();
    act(() => dialog.querySelector<HTMLButtonElement>('button[aria-label="Close"]')?.click());
    expect(dialog.open).toBe(false);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closes with a click outside the picture, not on it", () => {
    const { dialog, onClose, press } = open();
    const img = dialog.querySelector("img") as HTMLImageElement;
    const caption = dialog.querySelector("figcaption") as HTMLElement;
    const stage = dialog.querySelector("figure") as HTMLElement;
    press(img);
    press(caption);
    // Selecting or dragging from the picture out to the page.
    press(img, stage);
    expect(onClose).not.toHaveBeenCalled();
    press(stage);
    expect(dialog.open).toBe(false);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("closes with a click on the backdrop", () => {
    const { dialog, onClose, press } = open();
    press(dialog);
    expect(onClose).toHaveBeenCalledOnce();
  });

  it("plays videos", () => {
    const { dialog } = open({ name: "bug.mov", media_type: "video/quicktime", size: 3_400_000 });
    const video = dialog.querySelector("video");
    expect(video?.getAttribute("src")).toBe("/files/after.png");
    expect(video?.controls).toBe(true);
    expect(video?.autoplay).toBe(true);
    expect(dialog.querySelector("img")).toBeNull();
    expect(dialog.querySelector("figcaption")?.textContent).toBe("bug.mov3.4 MB");
  });

  it("says when a video can't play or an image didn't load", () => {
    const video = open({ name: "bug.mov", media_type: "video/quicktime" }, true);
    act(() => video.dialog.querySelector("video")?.dispatchEvent(new Event("error")));
    expect(video.dialog.querySelector("p")?.textContent).toBe("This browser can't play bug.mov.");
    // Downloading it still works.
    expect(video.dialog.querySelector('a[aria-label="Download bug.mov"]')).not.toBeNull();
    for (const unmount of roots.splice(0)) unmount();
    const image = open();
    act(() => image.dialog.querySelector("img")?.dispatchEvent(new Event("error")));
    expect(image.dialog.querySelector("p")?.textContent).toBe("after.png didn't load.");
  });

  it("offers the download only when asked", () => {
    expect(open().dialog.querySelector("a[download]")).toBeNull();
    for (const unmount of roots.splice(0)) unmount();
    const link = open(shot, true).dialog.querySelector("a[download]");
    expect(link?.getAttribute("href")).toBe("/files/after.png");
    expect(link?.getAttribute("download")).toBe("after.png");
    // Not on touch screens, where iOS answers it with a sheet of its own.
    expect(link?.className).toMatch(/\bpointer-coarse:hidden\b/);
  });

  it("holds the page still while it is open", () => {
    const html = document.documentElement;
    open();
    expect(html.style.overflow).toBe("hidden");
    for (const unmount of roots.splice(0)) unmount();
    expect(html.style.overflow).toBe("");
  });
});

describe("plainClick", () => {
  const click = (init: MouseEventInit = {}) => new MouseEvent("click", init);
  it("is a click the page may handle itself", () => {
    expect(plainClick(click())).toBe(true);
  });

  it("isn't one the browser opens in a new tab or window", () => {
    expect(plainClick(click({ metaKey: true }))).toBe(false);
    expect(plainClick(click({ ctrlKey: true }))).toBe(false);
    expect(plainClick(click({ shiftKey: true }))).toBe(false);
    expect(plainClick(click({ altKey: true }))).toBe(false);
    expect(plainClick(click({ button: 1 }))).toBe(false);
  });
});

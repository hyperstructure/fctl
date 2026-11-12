import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { Thumbnail, thumbnailSize } from "./thumbnail";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("thumbnailSize", () => {
  it("is as tall as the row and as wide as the image's proportions make it", () => {
    expect(thumbnailSize({ width: 1440, height: 900 }, 96, 288)).toEqual({ width: 154, height: 96 });
    expect(thumbnailSize({ width: 800, height: 800 }, 144, 320)).toEqual({ width: 144, height: 144 });
    // Small ones too, as screenshots of a part of a page are.
    expect(thumbnailSize({ width: 60, height: 40 }, 96, 288)).toEqual({ width: 144, height: 96 });
  });

  it("shows a wide image whole at the widest, shorter than the row", () => {
    // Three phone screenshots side by side.
    expect(thumbnailSize({ width: 2160, height: 450 }, 96, 288)).toEqual({ width: 288, height: 60 });
    expect(thumbnailSize({ width: 2160, height: 450 }, 144, 320)).toEqual({ width: 320, height: 67 });
    expect(thumbnailSize({ width: 2880, height: 960 }, 96, 288)).toEqual({ width: 288, height: 96 });
  });

  it("cuts an image down to a third of the row on its short side", () => {
    expect(thumbnailSize({ width: 5000, height: 400 }, 96, 288)).toEqual({ width: 288, height: 32 });
    // A whole page, scrolled through.
    expect(thumbnailSize({ width: 390, height: 6000 }, 144, 320)).toEqual({ width: 48, height: 144 });
    expect(thumbnailSize({ width: 300, height: 900 }, 96, 288)).toEqual({ width: 32, height: 96 });
  });
});

describe("Thumbnail", () => {
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
    return container.querySelector("img") as HTMLImageElement;
  }

  it("takes its room at once when its size is known", () => {
    const img = render(
      <Thumbnail src="/d.png" alt="d.png" width={2160} height={450} row={96} widest={288} />,
    );
    expect(img.style.width).toBe("288px");
    expect(img.style.height).toBe("60px");
    expect(img.getAttribute("loading")).toBe("lazy");
  });

  it("is as tall as the row until it loads, then fits as one whose size is known", () => {
    const img = render(<Thumbnail src="/d.png" alt="d.png" row={144} widest={320} />);
    expect(img.style.width).toBe("");
    expect(img.style.height).toBe("144px");
    Object.defineProperties(img, { naturalWidth: { value: 2160 }, naturalHeight: { value: 450 } });
    act(() => {
      img.dispatchEvent(new Event("load"));
    });
    expect(img.style.width).toBe("320px");
    expect(img.style.height).toBe("67px");
  });
});

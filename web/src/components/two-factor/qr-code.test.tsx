import { act } from "react";
import { createRoot } from "react-dom/client";
import { describe, expect, it } from "vitest";
import { QrCode } from "./qr-code";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

describe("QrCode", () => {
  it("draws every dark module on white, with a quiet zone", () => {
    const rows = ["1110111", "1000001", "1011101", "1011101", "1011101", "1000001", "1111111"];
    const container = document.createElement("div");
    const root = createRoot(container);
    act(() => root.render(<QrCode rows={rows} label="A code" />));
    const svg = container.querySelector("svg");
    expect(svg?.getAttribute("viewBox")).toBe("0 0 15 15");
    expect(svg?.getAttribute("aria-label")).toBe("A code");
    expect(container.querySelector("rect")?.getAttribute("fill")).toBe("#ffffff");
    const modules = [...container.querySelectorAll("path")].map(
      (p) => p.getAttribute("d")?.match(/M/g)?.length ?? 0,
    );
    const dark = rows
      .join("")
      .split("")
      .filter((c) => c === "1").length;
    expect(modules.reduce((a, b) => a + b, 0)).toBe(dark);
    act(() => root.unmount());
  });
});

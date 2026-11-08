import { afterEach, describe, expect, it } from "vitest";
import { setTheme } from "./theme";

describe("themes", () => {
  afterEach(() => document.head.replaceChildren());

  it("gives the browser's title bar the canvas color", () => {
    document.head.innerHTML = '<meta name="theme-color" content="#0a0a0a" />';
    const meta = document.querySelector('meta[name="theme-color"]');
    setTheme("light");
    expect(document.documentElement.dataset.theme).toBe("light");
    expect(meta?.getAttribute("content")).toBe("#fafafa");
    setTheme("dark");
    expect(meta?.getAttribute("content")).toBe("#0a0a0a");
  });
});

import { describe, expect, it } from "vitest";
import {
  colorName,
  contrast,
  distance,
  grayish,
  hexToHsv,
  hsvToHex,
  hueOf,
  over,
  parseHex,
  READABLE,
  readableOn,
  rgbToHex,
  SEEN,
  shades,
  THEME_BACKGROUNDS,
} from "./color";

describe("colors of one's own", () => {
  it("reads #rrggbb and #rgb, with or without the #, and nothing else", () => {
    expect(parseHex(" #FF8800 ")).toBe("#ff8800");
    expect(parseHex("2a9d8f")).toBe("#2a9d8f");
    expect(parseHex("#0af")).toBe("#00aaff");
    for (const bad of ["", "#", "#ff88", "#ff88000", "orange", "#gg8800", "##ff8800"])
      expect(parseHex(bad), bad).toBeUndefined();
  });

  it("goes between hex and hue, saturation and brightness", () => {
    for (const hex of ["#ff8800", "#2a9d8f", "#e63946", "#000000", "#ffffff", "#7f7f7f"])
      expect(hsvToHex(hexToHsv(hex))).toBe(hex);
    expect(hexToHsv("#ff0000")).toEqual({ h: 0, s: 1, v: 1 });
    expect(hexToHsv("#00ff00").h).toBe(120);
    // A gray has no hue and black no saturation: the picker's stay where they were.
    const teal = hexToHsv("#2a9d8f");
    expect(hexToHsv("#808080", teal).h).toBe(teal.h);
    expect(hexToHsv("#000000", teal)).toEqual({ h: teal.h, s: teal.s, v: 0 });
  });

  it("keeps a color that reads on a theme as it is", () => {
    expect(readableOn("#ff8800", THEME_BACKGROUNDS.dark)).toBe("#ff8800");
    expect(readableOn("#000080", THEME_BACKGROUNDS.light)).toBe("#000080");
    // The workspace colors' own shades for the light theme read there already.
    expect(readableOn("#0070e0", THEME_BACKGROUNDS.light)).toBe("#0070e0");
  });

  it("shades one that doesn't read just enough, in its own hue", () => {
    for (const [hex, theme] of [
      ["#ffff5d", "light"],
      ["#ff8800", "light"],
      ["#f4c2c2", "light"],
      ["#ffffff", "light"],
      ["#000080", "dark"],
      ["#111111", "dark"],
      ["#7a1fa2", "dark"],
    ] as const) {
      const bg = THEME_BACKGROUNDS[theme];
      const shade = readableOn(hex, bg);
      expect(contrast(shade, bg), `${hex} on ${theme}`).toBeGreaterThanOrEqual(READABLE);
      // No further than it must: a little less would not read.
      expect(contrast(shade, bg), `${hex} on ${theme}`).toBeLessThan(READABLE + 0.15);
      // As the eye sees it: sRGB has less room for color at some lightnesses, which can move it a little.
      const apart = Math.abs(hueOf(hex) - hueOf(shade)) % 360;
      if (hexToHsv(hex).s > 0.1) expect(Math.min(apart, 360 - apart), `${hex} keeps its hue`).toBeLessThan(6);
    }
    // The workspace yellow goes olive on light, as its own shade there does.
    expect(readableOn("#ffff5d", THEME_BACKGROUNDS.light)).toBe("#787700");
  });

  it("gives each theme an ink and a fill", () => {
    const { dark, light } = shades("#2A9D8F");
    expect(dark.ink).toBe("#2a9d8f");
    expect(contrast(light.ink, THEME_BACKGROUNDS.light)).toBeGreaterThanOrEqual(READABLE);
    expect(dark.soft).toBe("rgb(42 157 143 / 0.15)");
    expect(light.soft).toMatch(/^rgb\(42 157 143 \/ 0\.1\d*\)$/);
    // A pale color's fill is a little stronger on light, where it would hardly show.
    const pale = shades("#ffff5d").light.soft;
    expect(Number(pale.match(/\/ ([\d.]+)\)/)?.[1])).toBeGreaterThan(0.25);
  });

  it("gives every color a tile fill that shows on the light theme, the palest too", () => {
    const bg = THEME_BACKGROUNDS.light;
    const fill = (soft: string) => {
      const [r, g, b, a] = (soft.match(/[\d.]+/g) ?? []).map(Number);
      return over(rgbToHex({ r: r ?? 0, g: g ?? 0, b: b ?? 0 }), a ?? 0, bg);
    };
    for (const hex of [
      "#ffffff",
      "#fffdf5",
      "#f4f1ec",
      "#fafafa",
      "#ffff5d",
      "#f4c2c2",
      "#2a9d8f",
      "#000080",
    ])
      expect(distance(fill(shades(hex).light.soft), bg), hex).toBeGreaterThanOrEqual(SEEN);
    // Too near white to show as it is, white's fill is its shade's, a faint gray as ink's is.
    expect(shades("#ffffff").light.soft).toBe("rgb(115 115 115 / 0.1)");
    expect(grayish("#ffffff")).toBe(true);
    expect(grayish("#2a9d8f")).toBe(false);
  });

  it("names colors plainly", () => {
    expect(colorName("#ff8800")).toBe("orange");
    expect(colorName("#2a9d8f")).toBe("teal");
    expect(colorName("#007eff")).toBe("blue");
    expect(colorName("#ff00d8")).toBe("magenta");
    expect(colorName("#e63946")).toBe("red");
    expect(colorName("#f4c2c2")).toBe("pale pink");
    expect(colorName("#000080")).toBe("dark blue");
    expect(colorName("#8b5a2b")).toBe("brown");
    expect(colorName("#6b6b00")).toBe("olive");
    expect(colorName("#808080")).toBe("gray");
    expect(colorName("#050505")).toBe("black");
    expect(colorName("#ffffff")).toBe("white");
  });
});

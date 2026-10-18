import { inflateSync } from "node:zlib";
import { mark } from "@hyperstructure/design/brand";
import { describe, expect, it } from "vitest";
import { appIcon, badge, brandIcons, favicon, polygons, readMark } from "./brand-icons";

/** Width, height and RGBA pixels of a PNG this module wrote (no filters, one IDAT). */
function decode(file: Buffer) {
  expect([...file.subarray(0, 8)]).toEqual([137, 80, 78, 71, 13, 10, 26, 10]);
  const width = file.readUInt32BE(16);
  const height = file.readUInt32BE(20);
  const idat = file.indexOf("IDAT");
  const raw = inflateSync(file.subarray(idat + 4, idat + 4 + file.readUInt32BE(idat - 4)));
  const pixel = (x: number, y: number) => {
    const i = y * (width * 4 + 1) + 1 + x * 4;
    return [...raw.subarray(i, i + 4)];
  };
  return { width, height, pixel };
}

describe("brand icons", () => {
  it("reads the mark from the favicon, in the brand's colors", () => {
    const m = readMark(favicon());
    expect(m.side).toBe(64);
    expect(m.background).toBe(mark.background);
    expect(m.foreground).toBe(mark.foreground);
    // Two slanted bars, as brand.ts draws them.
    expect(polygons(m.path).map((p) => p.length)).toEqual(polygons(mark.path).map((p) => p.length));
  });

  it("reads straight-line paths", () => {
    expect(polygons("m1 1 h10 v10 h-10 z")).toEqual([
      [
        [1, 1],
        [11, 1],
        [11, 11],
        [1, 11],
      ],
    ]);
    expect(() => polygons("M0 0 C1 1 2 2 3 3")).toThrow(/cannot draw "C"/);
    expect(() => polygons("M0 0 L1 1 Z 4")).toThrow(/cannot draw/);
  });

  it("draws the app icon: yellow bars on blackish", () => {
    const icon = decode(appIcon(64));
    expect([icon.width, icon.height]).toEqual([64, 64]);
    // A corner is full-bleed background; the middle of the first bar is yellow.
    expect(icon.pixel(0, 0)).toEqual([0x17, 0x17, 0x17, 255]);
    expect(icon.pixel(26, 32)).toEqual([0xff, 0xff, 0x5d, 255]);
  });

  it("draws the badge: white bars on nothing", () => {
    const b = decode(badge(96));
    expect(b.pixel(0, 0)[3]).toBe(0);
    expect(b.pixel(48, 48)[3]).toBe(0); // between the bars
    const xs = Array.from({ length: 96 }, (_, x) => b.pixel(x, 48));
    expect(xs.some((p) => p[3] === 255 && p[0] === 255)).toBe(true);
  });

  it("names the files the app serves", () => {
    expect(Object.keys(brandIcons()).sort()).toEqual([
      "apple-touch-icon.png",
      "badge-96.png",
      "icon-192.png",
      "icon-512.png",
    ]);
  });
});

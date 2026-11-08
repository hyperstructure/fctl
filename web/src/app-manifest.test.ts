// @vitest-environment node
// The installable app: the manifest and index.html point at icons the build
// serves (BRAND_ASSETS in vite.config.ts), at the sizes they claim.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { themes } from "@hyperstructure/design/brand";
import { describe, expect, it } from "vitest";
import { BRAND_ASSETS } from "../vite.config";

type Icon = { src: string; sizes: string; purpose: string };

const require = createRequire(import.meta.url);
const web = new URL("..", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("public/manifest.webmanifest", web), "utf8"));
const html = readFileSync(new URL("index.html", web), "utf8");

/** An icon from @hyperstructure/design, as the build serves it. */
function brandAsset(file: string): Buffer {
  return readFileSync(require.resolve(`@hyperstructure/design/${BRAND_ASSETS[file]}`));
}

/** Width and height of an icon from @hyperstructure/design, as in a manifest. */
function pngSize(file: string): string {
  const png = brandAsset(file);
  return `${png.readUInt32BE(16)}x${png.readUInt32BE(20)}`;
}

describe("the installable app", () => {
  it("lists icons the build serves, at their real sizes", () => {
    for (const icon of manifest.icons as Icon[]) {
      const file = icon.src.slice(1);
      expect(Object.keys(BRAND_ASSETS)).toContain(file);
      expect(pngSize(file), file).toBe(icon.sizes);
    }
  });

  it("has what browsers need to offer installing it", () => {
    const sizes = (purpose: string) =>
      (manifest.icons as Icon[]).filter((i) => i.purpose === purpose).map((i) => i.sizes);
    expect(sizes("any")).toEqual(expect.arrayContaining(["192x192", "512x512"]));
    expect(sizes("maskable")).not.toEqual([]);
    expect(manifest).toMatchObject({ name: "fctl", start_url: "/", display: "standalone" });
  });

  it("keeps the screen's size on phones, without zooming", () => {
    const viewport = html.match(/<meta name="viewport" content="([^"]+)"/)?.[1] ?? "";
    expect(viewport.split(/,\s*/)).toEqual(
      expect.arrayContaining([
        "width=device-width",
        "initial-scale=1",
        "maximum-scale=1",
        "user-scalable=no",
      ]),
    );
  });

  it("links the manifest and the home screen icon", () => {
    expect(html).toContain('<link rel="manifest" href="/manifest.webmanifest" />');
    expect(html).toContain('<link rel="apple-touch-icon" href="/apple-touch-icon.png" />');
    expect(pngSize("apple-touch-icon.png")).toBe("180x180");
  });

  // Google Search takes no SVG favicon, and crops one that is opaque to its
  // edges into its circle. Browsers keep the SVG for tabs: Chrome prefers an
  // icon link without sizes there, and Firefox an SVG.
  it("links a full-bleed PNG favicon for search results beside the SVG one", () => {
    expect(html).toContain('<link rel="icon" href="/favicon.svg" type="image/svg+xml" />');
    expect(html).toContain(
      '<link rel="icon" href="/icon-maskable-512.png" type="image/png" sizes="512x512" />',
    );
    expect(pngSize("icon-maskable-512.png")).toBe("512x512");
  });

  // Feed readers, link previews and tools that save a site's icon fetch
  // /favicon.ico without reading the page's links: without the file, the app
  // would answer them with its page.
  it("serves an icon at /favicon.ico, in tab and bookmark sizes", () => {
    expect(Object.keys(BRAND_ASSETS)).toContain("favicon.ico");
    const ico = brandAsset("favicon.ico");
    expect([ico.readUInt16LE(0), ico.readUInt16LE(2)]).toEqual([0, 1]);
    const sizes = Array.from({ length: ico.readUInt16LE(4) }, (_, i) => ico[6 + 16 * i] || 256);
    expect(sizes).toEqual(expect.arrayContaining([16, 32]));
  });

  // What paints before the app's styles do takes the dark canvas from the
  // design package: the title bar, the installed app's splash and the
  // offline page, so that none of them shows a band of another dark.
  it("gives what paints before the app the dark theme's canvas", () => {
    const canvas = themes.dark.canvas;
    const file = (path: string) => readFileSync(new URL(path, web), "utf8");
    expect(manifest).toMatchObject({ background_color: canvas, theme_color: canvas });
    expect(html).toContain(`<meta name="theme-color" content="${canvas}" />`);
    expect(file("public/theme.js")).toContain(`dark ? "${canvas}" : "${themes.light.canvas}"`);
    expect(file("public/offline.html")).toMatch(new RegExp(`:root \\{\\s*--canvas: ${canvas};`));
  });
});

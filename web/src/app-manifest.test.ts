// @vitest-environment node
// The installable app: the manifest and index.html point at icons the build
// serves (BRAND_ASSETS in vite.config.ts), at the sizes they claim.
import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { describe, expect, it } from "vitest";
import { BRAND_ASSETS } from "../vite.config";

type Icon = { src: string; sizes: string; purpose: string };

const require = createRequire(import.meta.url);
const web = new URL("..", import.meta.url);
const manifest = JSON.parse(readFileSync(new URL("public/manifest.webmanifest", web), "utf8"));
const html = readFileSync(new URL("index.html", web), "utf8");

/** Width and height of an icon from @hyperstructure/design, as in a manifest. */
function pngSize(file: string): string {
  const png = readFileSync(require.resolve(`@hyperstructure/design/${BRAND_ASSETS[file]}`));
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

  it("links the manifest and the home screen icon", () => {
    expect(html).toContain('<link rel="manifest" href="/manifest.webmanifest" />');
    expect(html).toContain('<link rel="apple-touch-icon" href="/apple-touch-icon.png" />');
    expect(pngSize("apple-touch-icon.png")).toBe("180x180");
  });
});

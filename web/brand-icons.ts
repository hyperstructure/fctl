// PNG renditions of fctl's mark, drawn at build time from the favicon of
// @hyperstructure/design: the app icons for the manifest and the Home Screen,
// and the icon and badge on notifications. (Vite loads this with its config,
// where Node cannot import the package's TypeScript, so it reads the SVG.)

import { readFileSync } from "node:fs";
import { createRequire } from "node:module";
import { deflateSync } from "node:zlib";

type Point = [number, number];
/** Red, green and blue (0–255) and opacity (0–1). */
type Color = [number, number, number, number];
type Shape = (x: number, y: number) => boolean;

/** The polygons of an SVG path drawn with straight lines (M, L, H, V and Z, absolute or relative). */
export function polygons(d: string): Point[][] {
  const tokens = d.match(/[a-zA-Z]|-?(?:\d+\.?\d*|\.\d+)(?:e[-+]?\d+)?/g) ?? [];
  const out: Point[][] = [];
  let current: Point[] = [];
  let [x, y] = [0, 0];
  let cmd = "";
  let i = 0;
  const num = () => {
    const t = tokens[i++];
    if (t === undefined || /[a-zA-Z]/.test(t)) throw new Error(`mark path: a number is missing in "${d}"`);
    return Number(t);
  };
  const close = () => {
    if (current.length > 2) out.push(current);
    [x, y] = current[0] ?? [x, y];
    current = [];
  };
  while (i < tokens.length) {
    const t = tokens[i] ?? "";
    if (/[a-zA-Z]/.test(t)) {
      cmd = t;
      i++;
    }
    const rel = cmd === cmd.toLowerCase();
    switch (cmd.toUpperCase()) {
      case "M": {
        close();
        const [nx, ny] = [num(), num()];
        [x, y] = rel ? [x + nx, y + ny] : [nx, ny];
        current = [[x, y]];
        // Pairs after a moveto are linetos.
        cmd = rel ? "l" : "L";
        break;
      }
      case "L": {
        const [nx, ny] = [num(), num()];
        [x, y] = rel ? [x + nx, y + ny] : [nx, ny];
        current.push([x, y]);
        break;
      }
      case "H": {
        const n = num();
        x = rel ? x + n : n;
        current.push([x, y]);
        break;
      }
      case "V": {
        const n = num();
        y = rel ? y + n : n;
        current.push([x, y]);
        break;
      }
      case "Z":
        close();
        // A number after Z has no command.
        cmd = "";
        break;
      default:
        throw new Error(`mark path: cannot draw "${cmd || t}"; teach web/brand-icons.ts to`);
    }
  }
  close();
  return out;
}

function inPolygon(p: Point[], x: number, y: number): boolean {
  let inside = false;
  for (let i = 0, j = p.length - 1; i < p.length; j = i++) {
    const [xi, yi] = p[i] ?? [0, 0];
    const [xj, yj] = p[j] ?? [0, 0];
    if (yi > y !== yj > y && x < ((xj - xi) * (y - yi)) / (yj - yi) + xi) inside = !inside;
  }
  return inside;
}

function hex(color: string, alpha = 1): Color {
  const n = Number.parseInt(color.replace("#", ""), 16);
  return [(n >> 16) & 255, (n >> 8) & 255, n & 255, alpha];
}

/**
 * Draws shapes, bottom first, into a size × size RGBA image. `view` is the
 * square of mark units (32 × 32) the image shows; each pixel averages 4 × 4
 * samples.
 */
function draw(size: number, layers: [Shape, Color][], view: { x: number; y: number; side: number }) {
  const n = 4;
  const rgba = new Uint8Array(size * size * 4);
  for (let py = 0; py < size; py++) {
    for (let px = 0; px < size; px++) {
      let [r, g, b, a] = [0, 0, 0, 0];
      for (let s = 0; s < n * n; s++) {
        const x = view.x + ((px + ((s % n) + 0.5) / n) / size) * view.side;
        const y = view.y + ((py + (Math.floor(s / n) + 0.5) / n) / size) * view.side;
        // Premultiplied "over", layer by layer.
        let [sr, sg, sb, sa] = [0, 0, 0, 0];
        for (const [inside, [lr, lg, lb, la]] of layers) {
          if (!inside(x, y)) continue;
          [sr, sg, sb, sa] = [
            lr * la + sr * (1 - la),
            lg * la + sg * (1 - la),
            lb * la + sb * (1 - la),
            la + sa * (1 - la),
          ];
        }
        [r, g, b, a] = [r + sr, g + sg, b + sb, a + sa];
      }
      const i = (py * size + px) * 4;
      if (a > 0) {
        rgba[i] = Math.round(r / a);
        rgba[i + 1] = Math.round(g / a);
        rgba[i + 2] = Math.round(b / a);
      }
      rgba[i + 3] = Math.round((a / (n * n)) * 255);
    }
  }
  return rgba;
}

const CRC = Array.from({ length: 256 }, (_, k) => {
  let c = k;
  for (let bit = 0; bit < 8; bit++) c = c & 1 ? 0xedb88320 ^ (c >>> 1) : c >>> 1;
  return c >>> 0;
});

function crc32(bytes: Uint8Array): number {
  let c = 0xffffffff;
  for (const b of bytes) c = (CRC[(c ^ b) & 255] ?? 0) ^ (c >>> 8);
  return (c ^ 0xffffffff) >>> 0;
}

function chunk(type: string, data: Uint8Array): Buffer {
  const out = Buffer.alloc(12 + data.length);
  out.writeUInt32BE(data.length, 0);
  out.write(type, 4, "ascii");
  out.set(data, 8);
  out.writeUInt32BE(crc32(out.subarray(4, 8 + data.length)), 8 + data.length);
  return out;
}

/** An 8-bit RGBA PNG. */
export function png(size: number, rgba: Uint8Array): Buffer {
  const header = Buffer.alloc(13);
  header.writeUInt32BE(size, 0);
  header.writeUInt32BE(size, 4);
  header[8] = 8;
  header[9] = 6;
  const row = size * 4;
  const raw = Buffer.alloc(size * (row + 1));
  for (let y = 0; y < size; y++) raw.set(rgba.subarray(y * row, (y + 1) * row), y * (row + 1) + 1);
  return Buffer.concat([
    Buffer.from([137, 80, 78, 71, 13, 10, 26, 10]),
    chunk("IHDR", header),
    chunk("IDAT", deflateSync(raw)),
    chunk("IEND", new Uint8Array()),
  ]);
}

/** fctl's mark as the favicon draws it: bars on a square of `side` units. */
export interface Mark {
  side: number;
  background: string;
  foreground: string;
  path: string;
}

const require = createRequire(import.meta.url);

/** The favicon of @hyperstructure/design. */
export function favicon(): string {
  return readFileSync(require.resolve("@hyperstructure/design/assets/favicon.svg"), "utf8");
}

/** The mark in a favicon: its viewBox, the square's fill, and the bars' path and fill. */
export function readMark(svg: string): Mark {
  const side = Number(/viewBox="0 0 ([\d.]+) [\d.]+"/.exec(svg)?.[1]);
  const background = /<rect[^>]*fill="(#[0-9a-fA-F]{6})"/.exec(svg)?.[1];
  const path = /<path[^>]*\sd="([^"]+)"/.exec(svg)?.[1];
  const foreground = /<path[^>]*fill="(#[0-9a-fA-F]{6})"/.exec(svg)?.[1];
  if (!side || !background || !path || !foreground) {
    throw new Error("favicon.svg: expected a viewBox, a filled square and a filled path");
  }
  return { side, background, foreground, path };
}

function barsOf(mark: Mark): Shape {
  const bars = polygons(mark.path);
  return (x, y) => bars.some((p) => inPolygon(p, x, y));
}

/** The app icon: the bars on a full-bleed square that the system rounds (fine as a maskable icon). */
export function appIcon(size: number, mark = readMark(favicon())): Buffer {
  return png(
    size,
    draw(
      size,
      [
        [() => true, hex(mark.background)],
        [barsOf(mark), hex(mark.foreground)],
      ],
      { x: 0, y: 0, side: mark.side },
    ),
  );
}

/** A notification badge: the bars in white on nothing (Android draws only their shape). */
export function badge(size: number, mark = readMark(favicon())): Buffer {
  const points = polygons(mark.path).flat();
  const xs = points.map(([x]) => x);
  const ys = points.map(([, y]) => y);
  const [x0, x1, y0, y1] = [Math.min(...xs), Math.max(...xs), Math.min(...ys), Math.max(...ys)];
  const side = Math.max(x1 - x0, y1 - y0) * 1.2;
  return png(
    size,
    draw(size, [[barsOf(mark), hex("#ffffff")]], { x: (x0 + x1 - side) / 2, y: (y0 + y1 - side) / 2, side }),
  );
}

/** The files the web app serves at its root. */
export function brandIcons(): Record<string, Buffer> {
  const mark = readMark(favicon());
  return {
    "icon-192.png": appIcon(192, mark),
    "icon-512.png": appIcon(512, mark),
    "apple-touch-icon.png": appIcon(180, mark),
    "badge-96.png": badge(96, mark),
  };
}

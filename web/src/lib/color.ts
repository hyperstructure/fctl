// Colors people choose themselves (a project icon's own color): reading and writing #rrggbb, the
// hue, saturation and brightness a color picker moves through, and the shade of a color that
// reads on each theme's background. That shade keeps the color's hue and moves its lightness
// (in OKLCH, where lightness is what the eye sees) only as far as it must to stand 4.5:1 against
// the background, as the workspace colors' own shades for each theme do.

export interface Rgb {
  r: number;
  g: number;
  b: number;
}

/** Hue in degrees (0–360), saturation and value (brightness) from 0 to 1. */
export interface Hsv {
  h: number;
  s: number;
  v: number;
}

/**
 * `#rrggbb` in lowercase from six hex digits, or three for `#rgb`, with or without the `#`;
 * nothing for anything else.
 */
export function parseHex(input: string): string | undefined {
  const s = input.trim().replace(/^#/, "");
  if (!/^([0-9a-f]{3}|[0-9a-f]{6})$/i.test(s)) return;
  const full =
    s.length === 3
      ? s
          .split("")
          .map((c) => c + c)
          .join("")
      : s;
  return `#${full.toLowerCase()}`;
}

/** `#rrggbb` as people read it: in capitals, as design tools show it. */
export const showHex = (hex: string) => hex.toUpperCase();

export function hexToRgb(hex: string): Rgb {
  const n = Number.parseInt((parseHex(hex) ?? "#000000").slice(1), 16);
  return { r: (n >> 16) & 255, g: (n >> 8) & 255, b: n & 255 };
}

export function rgbToHex({ r, g, b }: Rgb): string {
  const c = (x: number) =>
    Math.round(Math.max(0, Math.min(255, x)))
      .toString(16)
      .padStart(2, "0");
  return `#${c(r)}${c(g)}${c(b)}`;
}

export function hsvToHex({ h, s, v }: Hsv): string {
  const f = (n: number) => {
    const k = (n + h / 60) % 6;
    return 255 * (v - v * s * Math.max(0, Math.min(k, 4 - k, 1)));
  };
  return rgbToHex({ r: f(5), g: f(3), b: f(1) });
}

/**
 * A hex color's hue, saturation and value. A gray has no hue and black no saturation either:
 * those keep `keep`'s, so that a picker's hue stays put as its color passes through them.
 */
export function hexToHsv(hex: string, keep?: Hsv): Hsv {
  const { r, g, b } = hexToRgb(hex);
  const [R, G, B] = [r / 255, g / 255, b / 255];
  const max = Math.max(R, G, B);
  const d = max - Math.min(R, G, B);
  let h = keep?.h ?? 0;
  if (d > 0) {
    if (max === R) h = 60 * (((G - B) / d + 6) % 6);
    else if (max === G) h = 60 * ((B - R) / d + 2);
    else h = 60 * ((R - G) / d + 4);
  }
  const s = max === 0 ? (keep?.s ?? 0) : d / max;
  return { h, s, v: max };
}

// sRGB ⇄ OKLab (Björn Ottosson's), through linear light.
const toLinear = (c: number) => {
  const x = c / 255;
  return x <= 0.04045 ? x / 12.92 : ((x + 0.055) / 1.055) ** 2.4;
};
const fromLinear = (x: number) => 255 * (x <= 0.0031308 ? 12.92 * x : 1.055 * x ** (1 / 2.4) - 0.055);

interface Lch {
  l: number;
  c: number;
  h: number;
}

function toOklch({ r, g, b }: Rgb): Lch {
  const [R, G, B] = [toLinear(r), toLinear(g), toLinear(b)];
  const l = Math.cbrt(0.4122214708 * R + 0.5363325363 * G + 0.0514459929 * B);
  const m = Math.cbrt(0.2119034982 * R + 0.6806995451 * G + 0.1073969566 * B);
  const s = Math.cbrt(0.0883024619 * R + 0.2817188376 * G + 0.6299787005 * B);
  const L = 0.2104542553 * l + 0.793617785 * m - 0.0040720468 * s;
  const a = 1.9779984951 * l - 2.428592205 * m + 0.4505937099 * s;
  const bb = 0.0259040371 * l + 0.7827717662 * m - 0.808675766 * s;
  return { l: L, c: Math.hypot(a, bb), h: Math.atan2(bb, a) };
}

/** Whether a hex color has hardly any hue: a gray, near enough, whose tile needs an edge as ink's does. */
export function grayish(hex: string): boolean {
  return toOklch(hexToRgb(hex)).c < 0.04;
}

/** A hex color's perceived hue (OKLCH's), in degrees. */
export function hueOf(hex: string): number {
  const h = (toOklch(hexToRgb(hex)).h * 180) / Math.PI;
  return (h + 360) % 360;
}

/** The linear sRGB of an OKLCH color, which may lie outside 0–1: outside sRGB. */
function linearOf({ l: L, c, h }: Lch): [number, number, number] {
  const a = c * Math.cos(h);
  const b = c * Math.sin(h);
  const l = (L + 0.3963377774 * a + 0.2158037573 * b) ** 3;
  const m = (L - 0.1055613458 * a - 0.0638541728 * b) ** 3;
  const s = (L - 0.0894841775 * a - 1.291485548 * b) ** 3;
  return [
    4.0767416621 * l - 3.3077115913 * m + 0.2309699292 * s,
    -1.2684380046 * l + 2.6097574011 * m - 0.3413193965 * s,
    -0.0041960863 * l - 0.7034186147 * m + 1.707614701 * s,
  ];
}

const inGamut = (x: [number, number, number]) => x.every((v) => v >= -1e-4 && v <= 1 + 1e-4);

/** An OKLCH color in sRGB: as colorful as sRGB has it at that lightness and hue. */
function fromOklch(lch: Lch): Rgb {
  let lo = 0;
  let hi = lch.c;
  if (!inGamut(linearOf(lch))) {
    for (let i = 0; i < 20; i++) {
      const mid = (lo + hi) / 2;
      if (inGamut(linearOf({ ...lch, c: mid }))) lo = mid;
      else hi = mid;
    }
  } else lo = lch.c;
  const [r, g, b] = linearOf({ ...lch, c: lo }).map((v) => fromLinear(Math.max(0, Math.min(1, v))));
  return { r: r ?? 0, g: g ?? 0, b: b ?? 0 };
}

/** WCAG's relative luminance. */
function luminance({ r, g, b }: Rgb): number {
  return 0.2126 * toLinear(r) + 0.7152 * toLinear(g) + 0.0722 * toLinear(b);
}

/** WCAG's contrast between two hex colors, from 1 to 21. */
export function contrast(a: string, b: string): number {
  const [x, y] = [luminance(hexToRgb(a)), luminance(hexToRgb(b))].sort((p, q) => q - p);
  return ((x ?? 0) + 0.05) / ((y ?? 0) + 0.05);
}

/** The backgrounds icons sit on, the hardest to read on of each theme: raised rows, and whiteish. */
export const THEME_BACKGROUNDS = { dark: "#232323", light: "#fafafa" } as const;
export type Theme = keyof typeof THEME_BACKGROUNDS;

/** The contrast the workspace colors' shades keep with the background, which thin lines need. */
export const READABLE = 4.5;

/**
 * `hex` as it reads on `bg`: itself when it stands `min`:1 against it, else as little lighter
 * (on a dark background) or darker (on a light one) as it takes, in its own hue.
 */
export function readableOn(hex: string, bg: string, min = READABLE): string {
  const color = parseHex(hex) ?? "#000000";
  if (contrast(color, bg) >= min) return color;
  const lch = toOklch(hexToRgb(color));
  const lighter = luminance(hexToRgb(bg)) < 0.18;
  // The lightness that reads lies between the color's own and white's, or black's.
  let near = lch.l;
  let far = lighter ? 1 : 0;
  let best = rgbToHex(fromOklch({ ...lch, l: far }));
  for (let i = 0; i < 24; i++) {
    const mid = (near + far) / 2;
    const candidate = rgbToHex(fromOklch({ ...lch, l: mid }));
    if (contrast(candidate, bg) >= min) {
      best = candidate;
      far = mid;
    } else near = mid;
  }
  return best;
}

/** What draws a color of one's own in a theme: its glyph's ink, and its tile's soft fill. */
export interface Shade {
  ink: string;
  soft: string;
}

/** How far a tile's fill must look from the background to show: about as far as the ink tile's does. */
export const SEEN = 0.025;

/** `hex` at `alpha` over `bg`, as one color. */
export function over(hex: string, alpha: number, bg: string): string {
  const [f, b] = [hexToRgb(hex), hexToRgb(bg)];
  const mix = (x: number, y: number) => x * alpha + y * (1 - alpha);
  return rgbToHex({ r: mix(f.r, b.r), g: mix(f.g, b.g), b: mix(f.b, b.b) });
}

const cache = new Map<string, Record<Theme, Shade>>();

/**
 * A color of one's own in each theme. The tile's fill is the color itself, faint, as the
 * workspace colors' are: on dark, the shade that reads, so that a dark color still shows; on
 * light, the color as chosen, a little stronger the paler it is, as the lime is there, unless
 * it's too near white to show at all (white itself, a cream): then the shade that reads, as the
 * ink tile's fill is a faint gray.
 */
export function shades(hex: string): Record<Theme, Shade> {
  const color = parseHex(hex) ?? "#000000";
  const known = cache.get(color);
  if (known) return known;
  const dark = readableOn(color, THEME_BACKGROUNDS.dark);
  const light = readableOn(color, THEME_BACKGROUNDS.light);
  const pale = Math.max(0, toOklch(hexToRgb(color)).l - 0.6) / 0.4;
  const alpha = 0.1 + 0.25 * pale;
  const bg = THEME_BACKGROUNDS.light;
  const shows = distance(over(color, alpha, bg), bg) >= SEEN;
  const out = {
    dark: { ink: dark, soft: withAlpha(dark, 0.15) },
    light: { ink: light, soft: shows ? withAlpha(color, alpha) : withAlpha(light, 0.1) },
  };
  if (cache.size > 500) cache.clear();
  cache.set(color, out);
  return out;
}

export function withAlpha(hex: string, alpha: number): string {
  const { r, g, b } = hexToRgb(hex);
  return `rgb(${r} ${g} ${b} / ${Math.round(alpha * 1000) / 1000})`;
}

/** How near two colors look: their distance in OKLab (0 for the same, about 0.02 hard to tell). */
export function distance(a: string, b: string): number {
  const x = toOklch(hexToRgb(a));
  const y = toOklch(hexToRgb(b));
  const [xa, xb, ya, yb] = [
    x.c * Math.cos(x.h),
    x.c * Math.sin(x.h),
    y.c * Math.cos(y.h),
    y.c * Math.sin(y.h),
  ];
  return Math.hypot(x.l - y.l, xa - ya, xb - yb);
}

// Hues by name, each up to the hue in degrees where the next begins.
const HUES: [number, string][] = [
  [12, "red"],
  [40, "orange"],
  [52, "amber"],
  [68, "yellow"],
  [90, "lime"],
  [150, "green"],
  [180, "teal"],
  [200, "cyan"],
  [245, "blue"],
  [265, "indigo"],
  [285, "violet"],
  [300, "purple"],
  [330, "magenta"],
  [345, "pink"],
  [360, "red"],
];

/** A color's name, plainly and roughly ("dark teal", "pale pink", "gray"), for screen readers. */
export function colorName(hex: string): string {
  const { h, s, v } = hexToHsv(hex);
  if (v < 0.12) return "black";
  if (s < 0.12) return v > 0.92 ? "white" : v < 0.35 ? "dark gray" : v > 0.75 ? "light gray" : "gray";
  const hue = HUES.find(([end]) => h < end)?.[1] ?? "red";
  // Dark oranges and ambers are browns.
  if ((hue === "orange" || hue === "amber") && v < 0.65 && s > 0.3) return v < 0.4 ? "dark brown" : "brown";
  if (hue === "yellow" && v < 0.65) return v < 0.3 ? "dark olive" : "olive";
  if (v < 0.55) return `dark ${hue}`;
  // Pale reds and magentas are pinks.
  if (s < 0.35 && v > 0.8) return `pale ${hue === "red" || hue === "magenta" ? "pink" : hue}`;
  return hue;
}

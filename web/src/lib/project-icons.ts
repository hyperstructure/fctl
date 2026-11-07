// Projects' icons (a design prototype): each project shows as an IconMCP glyph in one of the
// workspace colors, in the sidebar and on its page. Until projects keep an icon on the control
// plane (a `ProjectIcon { name, color, style }` beside the name), the choices live in this
// browser, and a project nobody chose one for gets the one that fits its name and description
// best: its emoji, then the words in it.

import { useSyncExternalStore } from "react";
import { ICONS, type IconDef } from "./project-icon-data";

export type IconStyle = "outline" | "solid" | "duotone";
/** The workspace colors (WORKSPACE_COLORS), for a project's glyph. */
export type IconColor = "ink" | "yellow" | "pink" | "blue";

export interface ProjectIconChoice {
  name: string;
  color: IconColor;
  style: IconStyle;
}

export const ICON_STYLES: { style: IconStyle; label: string }[] = [
  { style: "outline", label: "Outline" },
  { style: "duotone", label: "Duotone" },
  { style: "solid", label: "Solid" },
];

/**
 * Each color as a glyph, and as the soft square behind it: tokens, so that yellow reads as the
 * lime's darker olive on light backgrounds, as it does for text.
 */
export const ICON_COLORS: Record<IconColor, { label: string; ink: string; soft: string }> = {
  ink: { label: "Ink", ink: "var(--c-ink-2)", soft: "var(--c-hover)" },
  yellow: { label: "Yellow", ink: "var(--p-yellow)", soft: "var(--p-yellow-soft)" },
  pink: { label: "Pink", ink: "var(--p-pink)", soft: "var(--c-pink-soft)" },
  blue: { label: "Blue", ink: "var(--p-blue)", soft: "var(--c-blue-soft)" },
};

const byName = new Map(ICONS.map((i) => [i.name, i]));
export const iconDef = (name: string): IconDef | undefined => byName.get(name);
export const ICON_GROUPS = ["Build", "Business", "Life"] as const;
export { ICONS, type IconDef };

/** Icons whose name, label or tags have every word of `query`. */
export function searchIcons(query: string): IconDef[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return ICONS;
  return ICONS.filter((i) => {
    const text = `${i.name} ${i.label} ${i.tags.join(" ")} ${i.set}`.toLowerCase();
    return words.every((w) => text.includes(w));
  });
}

// Emoji people put in projects' descriptions, and the icon each stands for.
const EMOJI: [string, string][] = [
  ["🥕", "carrot"],
  ["📱", "smartphone"],
  ["🚐", "delivery-van"],
  ["🚚", "truck"],
  ["🍞", "bread"],
  ["🌱", "sprout"],
  ["🪴", "potted-plant"],
  ["🚀", "rocket"],
  ["☁", "cloud"],
  ["🎨", "palette"],
  ["📚", "book-stack"],
  ["🛒", "cart"],
  ["🛍", "bag"],
  ["🔒", "shield-check"],
  ["🤖", "robot"],
  ["🧪", "flask"],
  ["🐸", "frog"],
  ["🐱", "cat"],
  ["🐶", "dog"],
  ["🎮", "gamepad"],
  ["🎵", "music-note"],
  ["📷", "camera"],
  ["🍕", "pizza"],
  ["☕", "coffee"],
  ["🌍", "earth"],
  ["🗺", "map"],
  ["✨", "sparkles"],
  ["💡", "lightbulb"],
  ["📈", "line-chart"],
  ["📊", "bar-chart"],
  ["🧗", "climbing"],
];

// Words in a project's name or description, and the icons they suggest, best first.
const WORDS: [RegExp, string[]][] = [
  [/\b(shop|store|storefront|checkout|e-?commerce)\b/, ["store", "bag", "cart"]],
  [/\b(cart|basket|subscriptions?)\b/, ["cart", "bag"]],
  [/\b(payments?|billing|stripe|invoices?)\b/, ["credit-card", "wallet"]],
  [/\b(platform|backend|services?|orders)\b/, ["server-stack", "database", "api"]],
  [/\b(postgres|database|sql|sqlite|d1)\b/, ["database"]],
  [/\b(api|graphql|rest)\b/, ["api"]],
  [/\b(app|ios|android|expo|react native|mobile)\b/, ["smartphone"]],
  [/\b(driver|drivers|deliver|delivery|deliveries|vans?)\b/, ["delivery-van", "truck", "route"]],
  [/\b(routes?|routing|route planner|solver)\b/, ["route", "map"]],
  [
    /\b(infra|infrastructure|terraform|aws|cloudflare|deploys?|kubernetes)\b/,
    ["cloud", "cloud-server", "container-stack"],
  ],
  [/\b(benchmarks?|bench|performance|nightly)\b/, ["gauge", "line-chart", "bar-chart"]],
  [/\b(bread|sourdough|bake|bakery)\b/, ["bread"]],
  [/\b(plants?|soil|garden|pots?)\b/, ["sprout", "potted-plant", "leaf"]],
  [/\b(sensors?|esp32|firmware|embedded)\b/, ["chip", "antenna"]],
  [/\b(website|site|blog|homepage|astro)\b/, ["browser", "earth", "newspaper"]],
  [/\b(bookings?|calendar|reservations?)\b/, ["calendar-week", "ticket"]],
  [/\b(climbing|climb|boulder(ing)?)\b/, ["climbing", "mountain"]],
  [/\b(docs|documentation|handbook|guide)\b/, ["book-stack", "notebook"]],
  [/\b(design|design system|tokens|ui)\b/, ["palette", "pencil-ruler", "brush"]],
  [/\b(ai|agents?|llm|models?|claude)\b/, ["ai-sparkle", "robot", "ai-brain"]],
  [/\b(cli|terminal|shell)\b/, ["terminal-window"]],
  [/\b(factory|software factory)\b/, ["factory"]],
  [/\b(sandbox(es)?|containers?|docker|images?)\b/, ["container-stack", "cube"]],
  [/\b(homebrew|brew|formula|tap|packages?)\b/, ["package-open", "cube"]],
  [/\b(security|auth|sign-?in|passkeys?)\b/, ["shield-check", "api-key"]],
  [/\b(analytics|metrics|dashboards?|data)\b/, ["bar-chart", "pie-chart", "line-chart"]],
  [/\b(tests?|testing|ci)\b/, ["test-pass", "ci-pipeline"]],
  [/\b(games?|gaming)\b/, ["gamepad", "dice-pair"]],
  [/\b(music|audio|podcast)\b/, ["music-note", "headphones"]],
  [/\b(photos?|camera)\b/, ["camera"]],
  [/\b(video|film|movies?)\b/, ["film"]],
  [/\b(mail|email|newsletter)\b/, ["mail-open", "newspaper"]],
  [/\b(chat|messages?|support)\b/, ["message-circle"]],
  [/\b(learn|course|school|education)\b/, ["graduation-cap", "book-stack"]],
  [/\b(launch|rocket|startup)\b/, ["rocket"]],
  [/\b(ideas?|experiments?)\b/, ["lightbulb", "flask"]],
  [/\b(travel|trips?|flights?)\b/, ["plane", "earth"]],
  [/\b(food|recipes?|kitchen|restaurant)\b/, ["ramen", "pizza", "coffee"]],
  [/\b(farms?|vegetables?|produce)\b/, ["carrot", "wheat", "sprout"]],
  [/\b(bot|bots|automation)\b/, ["robot", "automation-bolt"]],
];

/** Generic icons for a project nothing in its name or description suggests one for. */
const FALLBACK = ["cube", "hexagon", "layers", "sparkles", "puzzle-piece", "target", "compass", "infinity"];

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

/** The icons that fit a project, best first: its emoji, then the words in its name and description. */
export function suggestIcons(p: { id: string; name: string; description?: string }, n = 8): string[] {
  const text = `${p.name} ${p.description ?? ""}`;
  const lower = text.toLowerCase();
  const out: string[] = [];
  const add = (name: string) => {
    if (byName.has(name) && !out.includes(name)) out.push(name);
  };
  // The name's words weigh most, then the description's emoji, then its words.
  for (const [re, icons] of WORDS) if (re.test(p.name.toLowerCase())) icons.forEach(add);
  for (const [emoji, icon] of EMOJI) if (text.includes(emoji)) add(icon);
  for (const [re, icons] of WORDS) if (re.test(lower)) icons.forEach(add);
  for (const i of searchIcons(p.name.toLowerCase().split(/\s+/)[0] ?? "")) {
    if (out.length >= n) break;
    if (p.name.length > 2) add(i.name);
  }
  const start = hash(p.id);
  for (let i = 0; out.length < n && i < FALLBACK.length; i++)
    add(FALLBACK[(start + i) % FALLBACK.length] ?? "cube");
  return out.slice(0, n);
}

// The choices made in this browser, by project id.
const KEY = "fctl:project-icons";
const listeners = new Set<() => void>();

function read(): string {
  try {
    return localStorage.getItem(KEY) ?? "{}";
  } catch {
    return "{}";
  }
}

let cachedRaw = "";
let cached: Record<string, ProjectIconChoice> = {};
function parsed(raw: string): Record<string, ProjectIconChoice> {
  if (raw === cachedRaw) return cached;
  cachedRaw = raw;
  try {
    const v = JSON.parse(raw) as unknown;
    cached = v && typeof v === "object" ? (v as Record<string, ProjectIconChoice>) : {};
  } catch {
    cached = {};
  }
  return cached;
}

function subscribe(fn: () => void) {
  listeners.add(fn);
  window.addEventListener("storage", fn);
  return () => {
    listeners.delete(fn);
    window.removeEventListener("storage", fn);
  };
}

/** Every project's chosen icon in this browser. */
export function useIconChoices(): Record<string, ProjectIconChoice> {
  return parsed(useSyncExternalStore(subscribe, read, () => "{}"));
}

/** Keeps `choice` as the project's icon (none: back to the one that fits it). */
export function chooseIcon(project: string, choice: ProjectIconChoice | undefined) {
  const next = { ...parsed(read()) };
  if (choice) next[project] = choice;
  else delete next[project];
  try {
    localStorage.setItem(KEY, JSON.stringify(next));
  } catch {}
  for (const l of listeners) l();
}

/** A project's icon: the one chosen for it, or the one that fits it, outlined in ink. */
export function iconOf(
  p: { id: string; name: string; description?: string },
  choices: Record<string, ProjectIconChoice>,
): ProjectIconChoice & { chosen: boolean } {
  const c = choices[p.id];
  if (c && byName.has(c.name)) return { ...c, chosen: true };
  return { name: suggestIcons(p, 1)[0] ?? "cube", color: "ink", style: "outline", chosen: false };
}

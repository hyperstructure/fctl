// Projects' icons: an IconMCP glyph in a workspace color and a style, which people choose for a
// project (G.ProjectIcon). A project without one shows the glyph that fits it best: the words of
// its name first, then its description's emoji, then its words.

import { useMutation } from "@tanstack/react-query";
import type * as G from "@/gen";
import { api } from "./api";
import { ICONS, type IconDef } from "./project-icon-data";
import { keys, queryClient } from "./queries";

export { ICONS, type IconDef };

/** An icon with its color and style settled: what draws it. */
export type IconChoice = Required<G.ProjectIcon>;

export const ICON_STYLES: { style: G.IconStyle; label: string }[] = [
  { style: "outline", label: "Outline" },
  { style: "duotone", label: "Duotone" },
  { style: "solid", label: "Solid" },
];

/**
 * Each color as a glyph, and as the soft square behind it: tokens, so that yellow reads as the
 * lime's darker olive on light backgrounds, as it does for text.
 */
export const ICON_COLORS: Record<G.WorkspaceColor, { label: string; ink: string; soft: string }> = {
  ink: { label: "Ink", ink: "var(--c-ink-2)", soft: "var(--c-hover)" },
  yellow: { label: "Yellow", ink: "var(--p-yellow)", soft: "var(--p-yellow-soft)" },
  pink: { label: "Pink", ink: "var(--p-pink)", soft: "var(--c-pink-soft)" },
  blue: { label: "Blue", ink: "var(--p-blue)", soft: "var(--c-blue-soft)" },
};
export const COLOR_ORDER: G.WorkspaceColor[] = ["ink", "yellow", "pink", "blue"];

const byName = new Map(ICONS.map((i) => [i.name, i]));
export const iconDef = (name: string): IconDef | undefined => byName.get(name);
export const ICON_GROUPS = ["Build", "Business", "Life"] as const;

/**
 * Icons for `query`, best first: every word must be in the icon's name, label, tags or set. Those
 * whose name or label starts with the first word come before those that only have it somewhere.
 */
export function searchIcons(query: string): IconDef[] {
  const words = query.toLowerCase().split(/\s+/).filter(Boolean);
  if (!words.length) return ICONS;
  const score = (i: IconDef): number => {
    const name = i.name;
    const label = i.label.toLowerCase();
    const tags = i.tags.join(" ").toLowerCase();
    const text = `${name} ${label} ${tags} ${i.set}`;
    if (!words.every((w) => text.includes(w))) return 0;
    const [first = ""] = words;
    if (name === first || label === first) return 4;
    if (name.startsWith(first) || label.startsWith(first)) return 3;
    if (name.includes(first) || label.split(/\s+/).some((w) => w.startsWith(first))) return 2;
    return 1;
  };
  return ICONS.map((i) => [i, score(i)] as const)
    .filter(([, s]) => s > 0)
    .sort((a, b) => b[1] - a[1])
    .map(([i]) => i);
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
  ["🐍", "snake"],
  ["🦀", "crab"],
  ["🐙", "octopus"],
  ["🦊", "fox"],
  ["🦉", "owl"],
  ["🐢", "turtle"],
  ["🐳", "whale"],
  ["🐋", "whale"],
  ["🐰", "rabbit"],
  ["🐇", "rabbit"],
  ["🐧", "penguin"],
  ["🐘", "elephant"],
  ["🐻", "bear"],
  ["🐼", "panda"],
  ["🐨", "koala"],
  ["🦦", "otter"],
  ["🦔", "hedgehog"],
  ["🦥", "sloth"],
  ["🦝", "raccoon"],
  ["🐬", "dolphin"],
  ["🦙", "llama"],
  ["🐒", "monkey"],
  ["🐵", "monkey"],
  ["🦁", "lion"],
  ["🐯", "tiger"],
  ["🦒", "giraffe"],
  ["🦜", "parrot"],
  ["🦆", "duck"],
  ["🐺", "wolf"],
  ["🐿", "squirrel"],
  ["🐌", "snail"],
  ["🐜", "ant"],
  ["🪼", "jellyfish"],
  ["📨", "mail"],
  ["✉", "mail"],
  ["📥", "inbox"],
  ["📡", "broadcast"],
  ["📦", "package-open"],
  ["🧰", "toolbox"],
  ["🧱", "building-bricks"],
  ["🔌", "integration-plug"],
];

// Words in a project's name or description, and the icons they suggest, best first. Where several
// match, each one's best icon comes before any one's second, and the earlier word first.
const WORDS: [RegExp, string[]][] = [
  [/\b(shop|store|storefront|checkout|e-?commerce)\b/, ["store", "bag", "cart"]],
  [/\b(cart|basket|subscriptions?)\b/, ["cart", "bag"]],
  [/\b(payments?|billing|stripe|invoices?)\b/, ["credit-card", "wallet"]],
  [/\b(microservices?|service mesh|gateways?)\b/, ["topology-mesh", "api-gateway", "server-stack"]],
  [/\b(pub ?\/ ?sub|pub-?sub|sns|fan[- ]?outs?|broadcasts?)\b/, ["pub-sub", "parallel-tasks", "broadcast"]],
  [
    /\b(queues?|sqs|messaging|brokers?|rabbitmq|amqp|kafka|nats|consumers?|dlqs?)\b/,
    ["message-queue", "event-bus", "task-queue"],
  ],
  [/\b(event[- ]driven|event ?bus|cloudevents|eventbridge)\b/, ["event-bus"]],
  [
    /\b(lib|libs|library|libraries|sdks?|frameworks?|toolkit)\b/,
    ["library", "package-manager", "building-bricks", "toolbox"],
  ],
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
  [/\b(sensors?|esp32|firmware|embedded)\b/, ["chip", "router-wifi"]],
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
  [/\b(farms?|vegetables?|produce)\b/, ["wheat", "sprout", "carrot"]],
  [/\b(bot|bots|automation)\b/, ["robot", "bot", "automation-bolt"]],
  // What it's built with and what it does along the way, after what it is.
  [/\b(python|asyncio|django|fastapi|pypi)\b/, ["snake"]],
  [/\b(rust)\b/, ["crab"]],
  [
    /\b(heartbeats?|health ?checks?|uptime|monitoring|observability|opentelemetry)\b/,
    ["activity", "health-check"],
  ],
  [/\b(cache|caching|redis|memcached)\b/, ["cache"]],
  [/\b(middlewares?|plugins?|extensions?|integrations?)\b/, ["integration-plug"]],
];

/** Generic icons for a project nothing in its name or description suggests one for. */
const FALLBACK = ["cube", "hexagon", "layers", "sparkles", "puzzle-piece", "target", "compass", "infinity"];

function hash(s: string): number {
  let h = 0;
  for (let i = 0; i < s.length; i++) h = (h * 31 + s.charCodeAt(i)) | 0;
  return Math.abs(h);
}

/** The icons `text`'s words suggest: the best of each word's, then the second of each, and so on. */
function wordsIn(text: string): string[] {
  const lists = WORDS.filter(([re]) => re.test(text)).map(([, icons]) => icons);
  const out: string[] = [];
  for (let rank = 0; lists.some((l) => rank < l.length); rank++)
    for (const icons of lists) {
      const icon = icons[rank];
      if (icon) out.push(icon);
    }
  return out;
}

/** What a project's icon is suggested from. */
export interface Fits {
  /** Picks among the generic icons when nothing else fits; a new project has none yet. */
  id?: string;
  name: string;
  description?: string;
}

/** The icons that fit a project, best first. */
export function suggestIcons(p: Fits, n = 8): string[] {
  const text = `${p.name} ${p.description ?? ""}`;
  const out: string[] = [];
  const add = (name: string) => {
    if (byName.has(name) && !out.includes(name)) out.push(name);
  };
  const name = p.name.toLowerCase();
  wordsIn(name).forEach(add);
  for (const [emoji, icon] of EMOJI) if (text.includes(emoji)) add(icon);
  wordsIn(text.toLowerCase()).forEach(add);
  // Icons named like the name's first word: "Rocket" finds rocket.
  const [word = ""] = name.split(/\s+/);
  if (word.length > 2) for (const i of searchIcons(word).slice(0, 3)) add(i.name);
  const start = hash(p.id ?? p.name);
  for (let i = 0; out.length < n && i < FALLBACK.length; i++)
    add(FALLBACK[(start + i) % FALLBACK.length] ?? "cube");
  return out.slice(0, n);
}

/** The icon that fits a project that has none: its best suggestion, in ink lines. */
export function fittingIcon(p: Fits): IconChoice {
  return { name: suggestIcons(p, 1)[0] ?? "cube", color: "ink", style: "outline" };
}

/**
 * The icon a project shows: the one chosen for it, or the one that fits it. A chosen glyph this
 * version of the app doesn't have keeps its color and style on the one that fits.
 */
export function iconOf(p: Fits & { icon?: G.ProjectIcon }): IconChoice & { chosen: boolean } {
  const own = p.icon;
  const fits = fittingIcon(p);
  if (!own) return { ...fits, chosen: false };
  return {
    name: byName.has(own.name) ? own.name : fits.name,
    // A color or a style from a newer control plane draws as ink lines.
    color: knownColor(own.color),
    style: knownStyle(own.style),
    chosen: true,
  };
}

/** `c` when this version of the app draws it, else ink. */
export const knownColor = (c: string | undefined): G.WorkspaceColor =>
  COLOR_ORDER.includes(c as G.WorkspaceColor) ? (c as G.WorkspaceColor) : "ink";
/** `s` when this version of the app draws it, else outline. */
export const knownStyle = (s: string | undefined): G.IconStyle =>
  ICON_STYLES.some((x) => x.style === s) ? (s as G.IconStyle) : "outline";

/** Whether two icons draw the same. */
export const sameIcon = (a: G.ProjectIcon | undefined, b: G.ProjectIcon | undefined) =>
  a?.name === b?.name &&
  (a?.color ?? "ink") === (b?.color ?? "ink") &&
  (a?.style ?? "outline") === (b?.style ?? "outline");

/**
 * Saves a project's icon (none: the one that fits it again). Lists show it at once, and go back
 * to what they had if the control plane refuses it.
 */
export function useSaveIcon(project: Pick<G.Project, "id" | "slug">) {
  return useMutation({
    mutationFn: (icon: G.ProjectIcon | null) => api.updateProject(project.id, { icon: icon ?? { name: "" } }),
    onMutate: async (icon) => {
      await queryClient.cancelQueries({ queryKey: keys.projects });
      const list = queryClient.getQueryData<G.ProjectList>(keys.projects);
      const one = queryClient.getQueryData<G.Project>(keys.project(project.slug));
      const set = (p: G.Project): G.Project => (p.id === project.id ? { ...p, icon: icon ?? undefined } : p);
      if (list)
        queryClient.setQueryData<G.ProjectList>(keys.projects, { ...list, projects: list.projects.map(set) });
      if (one) queryClient.setQueryData<G.Project>(keys.project(project.slug), set(one));
      return { list, one };
    },
    onError: (_e, _icon, before) => {
      if (before?.list) queryClient.setQueryData(keys.projects, before.list);
      if (before?.one) queryClient.setQueryData(keys.project(project.slug), before.one);
    },
    onSuccess: (p) => {
      queryClient.setQueryData(keys.project(project.slug), p);
      queryClient.setQueryData<G.ProjectList>(
        keys.projects,
        (old) =>
          old && { ...old, projects: old.projects.map((x) => (x.id === p.id ? { ...x, icon: p.icon } : x)) },
      );
    },
  });
}

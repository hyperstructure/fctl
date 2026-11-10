// How much of a conversation Claude keeps in its context: the size a project's sessions compact
// near (ProjectSettings.compact_window), as people choose and write it. Claude Code compacts a
// little before its auto-compact window, which is the model's whole one by default; the Rust side
// (fctl_domain::context) checks the same bounds.

import type * as G from "@/gen";
import { num } from "./format";

/** The smallest window people choose: Claude Code reads smaller ones as this. */
export const MIN_TOKENS = 100_000;
/** The largest: Claude caps the window at the model's own anyway. */
export const MAX_TOKENS = 1_000_000;

/** The sizes offered besides Claude's default and one of people's own. */
export const COMPACT_CHOICES = [250_000, 400_000, 600_000] as const;

/** What the window's list shows: "" for Claude's default, a size offered, or "other" for one typed. */
export type CompactChoice = "" | `${(typeof COMPACT_CHOICES)[number]}` | "other";

/** The choice that shows `tokens`. */
export function compactChoice(tokens?: number | null): CompactChoice {
  if (tokens == null) return "";
  return COMPACT_CHOICES.find((c) => c === tokens) != null ? (`${tokens}` as CompactChoice) : "other";
}

/** A number of tokens as people write it: `400000`, `400k`, `1.5M`, `400 000` or `400,000`. */
export function parseTokens(input: string): number | undefined {
  const s = input.trim().replace(/[ ,_]/g, "");
  const m = /^(\d*)(?:\.(\d*))?([kKmM]?)$/.exec(s);
  if (!m) return undefined;
  const [, whole = "", fraction = "", suffix = ""] = m;
  if (!whole && !fraction) return undefined;
  const unit = suffix === "" ? 1 : suffix.toLowerCase() === "k" ? 1_000 : 1_000_000;
  if (unit === 1 && s.includes(".")) return undefined;
  const digits = fraction.slice(0, String(unit).length - 1);
  const part = digits ? Number(digits.padEnd(String(unit).length - 1, "0")) : 0;
  const n = Number(whole || "0") * unit + part;
  return Number.isSafeInteger(n) ? n : undefined;
}

/** A size in the field people type it in: `300K`, `1.5M`, or the number. */
export function tokensText(tokens: number): string {
  if (tokens >= 1_000_000 && tokens % 100_000 === 0) return `${tokens / 1_000_000}M`;
  if (tokens % 1_000 === 0) return `${tokens / 1_000}K`;
  return String(tokens);
}

/** Whether `tokens` is a window people may choose. */
export function validWindow(tokens: number): boolean {
  return Number.isInteger(tokens) && tokens >= MIN_TOKENS && tokens <= MAX_TOKENS;
}

/** Where a project's sessions compact, for its settings' list: "Compacts near 400K", or nothing. */
export function compactGist(s: Pick<G.ProjectSettings, "compact_window">): string | undefined {
  return s.compact_window != null ? `compacts near ${num(s.compact_window)}` : undefined;
}

// Claude plan usage: how much of each limit of a person's plan (the
// five-hour window, the week, a model's weekly limit) sessions last saw
// used. The same rules as `fctl_domain::usage` in Rust.

import type * as G from "@/gen";
import { clock } from "./format";

/** Most of a limit is used from this share on. */
export const CLOSE = 0.8;
/** Almost nothing of a limit is left from this share on. */
export const CRITICAL = 0.9;

/** Where a limit stands: plenty left, mostly used, nearly used up, used up, or started over
 * since the reading. */
export type Standing = "fine" | "close" | "critical" | "reached" | "reset";

const words = (s: string) => {
  const t = s.replaceAll("_", " ");
  return t.charAt(0).toUpperCase() + t.slice(1);
};

/** "5-hour limit", "Weekly limit", "Weekly Opus limit". */
export function limitLabel(window: string): string {
  if (window === "five_hour") return "5-hour limit";
  if (window === "seven_day") return "Weekly limit";
  // Claude Code calls this one the Fable limit.
  if (window === "seven_day_overage_included") return "Weekly Fable limit";
  if (window.startsWith("seven_day_")) return `Weekly ${words(window.slice("seven_day_".length))} limit`;
  return `${words(window)} limit`;
}

/** For tight spaces: "5-hour", "Weekly", "Weekly Opus". */
export const shortLabel = (window: string) => limitLabel(window).replace(/ limit$/, "");

/** Only the share used and a refusal count. Claude Code warns (`allowed_warning`) when a limit
 * goes faster than its window passes, as with a quarter of the week used in its first day,
 * however much is left. */
export function standing(l: G.PlanLimit, now: number): Standing {
  const used = l.utilization ?? 0;
  if (l.resets_at !== undefined && l.resets_at <= now) return "reset";
  if (l.status === "rejected" || used >= 1) return "reached";
  if (used >= CRITICAL) return "critical";
  if (used >= CLOSE) return "close";
  return "fine";
}

/** The share of the limit used, from 0 to 1, for a meter: nothing once it started over. */
export function share(l: G.PlanLimit, now: number): number {
  const s = standing(l, now);
  if (s === "reset") return 0;
  if (s === "reached") return 1;
  return Math.min(1, Math.max(0, l.utilization ?? 0));
}

/** "42%"; empty when Claude did not say. */
export const percent = (l: G.PlanLimit) =>
  l.utilization === undefined ? "" : `${Math.round(l.utilization * 100)}%`;

/** When a session last reported the usage. */
export const reportedAt = (usage: G.PlanLimit[]) => Math.max(0, ...usage.map((l) => l.observed_at));

const weekday = new Intl.DateTimeFormat(undefined, { weekday: "short" });
const date = new Intl.DateTimeFormat(undefined, { month: "short", day: "numeric" });

/** When a limit resets, to follow "resets": "at 3:40 PM" today, "tomorrow at 9:00 AM",
 * "Thu at 9:00 AM" within a week, then "on Oct 24". */
export function resetTime(ms: number, now: number): string {
  const day = (t: number) => {
    const d = new Date(t);
    return new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  };
  const days = Math.round((day(ms) - day(now)) / 86_400_000);
  if (days <= 0) return `at ${clock(ms)}`;
  if (days === 1) return `tomorrow at ${clock(ms)}`;
  if (days < 7) return `${weekday.format(ms)} at ${clock(ms)}`;
  return `on ${date.format(ms)}`;
}

/** The rate limit tier of a Claude Max 20x login, which Claude renamed Ultra. */
export const ULTRA_TIER = "default_claude_max_20x";

/** Whether a credential is on Claude's Ultra plan: its login's rate limit tier says so, or, when
 * the login doesn't say (a long-lived token, or a login kept before fctl kept the tier), its
 * sessions report a weekly limit but no five-hour one, which only Ultra plans dropped. The same
 * rule as `fctl_domain::usage::is_ultra`. */
export function isUltra(c: Pick<G.ClaudeCredentialInfo, "subscription_type" | "rate_limit_tier" | "usage">) {
  if (c.subscription_type !== undefined && c.subscription_type !== "max") return false;
  if (c.rate_limit_tier !== undefined) return c.rate_limit_tier === ULTRA_TIER;
  const reports = (w: string) => (c.usage ?? []).some((l) => l.window === w);
  return reports("seven_day") && !reports("five_hour");
}

/** The plan to name beside a credential: "Ultra", else its subscription type ("max", "pro"). */
export const planOf = (c: Pick<G.ClaudeCredentialInfo, "subscription_type" | "rate_limit_tier" | "usage">) =>
  isUltra(c) ? "Ultra" : c.subscription_type;

/** A label inside a sentence: "weekly Opus limit". */
export const inSentence = (label: string) => label.charAt(0).toLowerCase() + label.slice(1);

/** The limit to tell people about before they start work, once one is mostly used: one used
 * up, else one nearly used up, else one mostly used. */
export function pressing(usage: G.PlanLimit[], now: number): G.PlanLimit | undefined {
  for (const s of ["reached", "critical", "close"] as const) {
    const l = usage.find((l) => standing(l, now) === s);
    if (l) return l;
  }
  return undefined;
}

// How long sign-ins last and who makes access tokens: the instance's session policies (G.SessionPolicy), the
// choices Admin → Sign-in offers for them, and the sentences it and Settings say about them. The rules mirror
// crates/fctl-domain/src/session_policy.rs, and the sentences the CLI's (`fctl admin sign-in policies`).

import type { SelectOption } from "@/components/ui/select";
import type * as G from "@/gen";

/** A policy as control planes from before them behave. */
export const DEFAULT_POLICY: G.SessionPolicy = {
  browser_max_hours: 0,
  browser_idle_minutes: 30 * 24 * 60,
  cli_max_days: 0,
  access_token_max_days: 0,
  member_access_tokens: true,
};

/** "a minute", "15 minutes", "an hour", "12 hours", "a day", "a week", "30 days", as the CLI says them. */
export function minutes(m: number): string {
  if (m === 1) return "a minute";
  if (m === 60) return "an hour";
  if (m === 1440) return "a day";
  if (m === 10080) return "a week";
  if (m === 525600) return "a year";
  if (m % 1440 === 0) return `${m / 1440} days`;
  if (m % 60 === 0) return `${m / 60} hours`;
  return `${m} minutes`;
}

export const hours = (h: number) => (h === 8760 ? "a year" : minutes(h * 60));
export const days = (d: number) => (d === 365 ? "a year" : minutes(d * 1440));

const capital = (s: string) => s.charAt(0).toUpperCase() + s.slice(1);

/** The choices for one limit: `values` and the one saved, each in words; 0 is `none`, no limit. */
function choices(
  values: number[],
  current: number | undefined,
  words: (n: number) => string,
  none?: string,
  fallback?: number,
): SelectOption[] {
  const all = [...values];
  if (current !== undefined && !all.includes(current)) all.push(current);
  return all
    .sort((a, b) => (a || Number.POSITIVE_INFINITY) - (b || Number.POSITIVE_INFINITY))
    .map((n) => ({
      value: String(n),
      label: n === 0 ? (none ?? "") : capital(words(n)),
      description: n === fallback ? "The default" : undefined,
    }));
}

/** How long a browser stays signed in, however much it's used. */
export const browserMaxChoices = (current?: number) =>
  choices([0, 8, 12, 24, 168, 720], current, hours, "While it's used", 0);

/** How long a browser may go unused. */
export const browserIdleChoices = (current?: number) =>
  choices([15, 60, 480, 1440, 10080, 43200], current, minutes, undefined, 43200);

/** How long a CLI stays signed in. */
export const cliMaxChoices = (current?: number) =>
  choices([0, 1, 7, 30, 90], current, days, "While it's used", 0);

/** The longest an access token lasts. */
export const tokenMaxChoices = (current?: number) =>
  choices([0, 7, 30, 90, 365], current, days, "As its maker chooses", 0);

/** "a", "a and b". */
const and = (items: string[]) =>
  items.length <= 1 ? (items[0] ?? "") : `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;

const count = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n} ${many}`);

/** What saving a policy ends at once, as the CLI says it before it asks. */
export function consequences(p: G.SessionPolicyPreview): string[] {
  const out: string[] = [];
  if (p.browsers > 0) out.push(count(p.browsers, "browser", "browsers"));
  if (p.clis > 0) out.push(count(p.clis, "CLI", "CLIs"));
  const lines: string[] = [];
  if (out.length > 0)
    lines.push(
      `Saving signs out ${and(out)} at once${p.you ? ", yours among them" : ""}: their people sign in again.`,
    );
  else if (p.you) lines.push("Saving signs you out at once: you sign in again.");
  if (p.access_tokens > 0) {
    const members =
      p.member_tokens === 0
        ? ""
        : p.member_tokens === p.access_tokens
          ? p.access_tokens === 1
            ? ", a member's"
            : ", all of them members'"
          : `, ${p.member_tokens} of them members'`;
    lines.push(
      `It ends ${count(p.access_tokens, "access token", "access tokens")}${members}: scripts and CI that use ${p.access_tokens === 1 ? "it" : "them"} need new ones.`,
    );
  }
  if (lines.length === 0) lines.push("Nobody is signed out now: sign-ins end as it says from here on.");
  return lines;
}

/** Whether two policies say the same. */
export const samePolicy = (a: G.SessionPolicy, b: G.SessionPolicy) =>
  a.browser_max_hours === b.browser_max_hours &&
  a.browser_idle_minutes === b.browser_idle_minutes &&
  a.cli_max_days === b.cli_max_days &&
  a.access_token_max_days === b.access_token_max_days &&
  a.member_access_tokens === b.member_access_tokens;

/** What Settings → Devices says of how long its sign-ins last, where the instance limits them more than by default. */
export function devicesNote(p: G.SessionPolicy | undefined): string | undefined {
  if (!p) return undefined;
  const browser: string[] = [];
  if (p.browser_max_hours > 0) browser.push(`${hours(p.browser_max_hours)} after they sign in`);
  if (p.browser_idle_minutes < DEFAULT_POLICY.browser_idle_minutes)
    browser.push(`after ${minutes(p.browser_idle_minutes)} unused`);
  const cli = p.cli_max_days > 0 ? `CLIs ${days(p.cli_max_days)} after they sign in` : undefined;
  if (browser.length === 0) return cli && `This instance signs ${cli.replace(/^CLIs/, "CLIs out")}.`;
  const browsers = `This instance signs browsers out ${browser.join(", or ")}`;
  return cli ? `${browsers}, and ${cli}.` : `${browsers}.`;
}

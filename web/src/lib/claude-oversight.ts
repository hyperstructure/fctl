// What a workspace's admins see of their members' Claude connections (#627): the facts line under
// each person, the summary and the pills above the list, the whole of it when a row opens, and the
// way out. The same words as `fctl workspaces members --claude`, built from `ClaudeOverview` alone:
// never a token, its account or the label its person gave it.

import type * as G from "@/gen";
import { ago, clock } from "./format";
import { inSentence, limitLabel } from "./usage";

const DAY = 86_400_000;

/** A token runs out "soon" within this many days: the People tab's pill, the amber date. */
export const SOON_DAYS = 30;
/** The Agent Floor and Settings' list of pages speak of a token in its last week. */
export const LAST_DAYS = 7;
/** So many limit pauses in a week say a plan's limits are short for the work. */
export const FREQUENT_PAUSES = 5;

/** The pills above the list, Everyone first. */
export type Narrow = "everyone" | "none" | "soon" | "ended" | "pausing";

/** How a fact reads: plainly, as a warning (amber), or as a failure (red). */
export type Tone = "plain" | "warn" | "danger";

/** A day as the locale writes it, "Dec 14", with the year when it isn't `now`'s: "Jan 3, 2027". */
export function shortDay(ms: number, now = Date.now()): string {
  const year = new Date(ms).getFullYear() !== new Date(now).getFullYear();
  return new Date(ms).toLocaleDateString(undefined, {
    month: "short",
    day: "numeric",
    ...(year ? { year: "numeric" } : {}),
  });
}

/** A day with its year, for whole sentences: "Dec 14, 2026". */
export const fullDay = (ms: number) =>
  new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric", year: "numeric" });

/** Whole days from `now` until `at`, rounded up: 0 once it passed. */
export const daysLeft = (at: number, now = Date.now()) => Math.max(0, Math.ceil((at - now) / DAY));

/** Whether a token that runs out at `at` does within `days` of `now` (or did). */
export const within = (at: number | undefined, days: number, now = Date.now()) =>
  at !== undefined && at - now <= days * DAY;

/** "Pro", "Max", "Team", "Enterprise", "Ultra"; undefined while unknown. */
export function planName(plan: string | undefined): string | undefined {
  if (!plan) return undefined;
  return plan.charAt(0).toUpperCase() + plan.slice(1).replaceAll("_", " ");
}

/** "the weekly limit", "the 5-hour limit", for a sentence. */
export const theLimit = (window: string | undefined) =>
  window ? `the ${inSentence(limitLabel(window))}` : "a usage limit";

/** Whether someone's sessions can't run: no connection, or one Claude stopped taking. */
export const blocked = (m: G.MemberClaude) => m.state === "none" || m.state === "ended";

/** Whether someone's token runs out within a month while it still works. */
export const runsOutSoon = (m: G.MemberClaude, now = Date.now()) =>
  !blocked(m) && within(m.runs_out_at, SOON_DAYS, now);

/** Whether someone's sessions wait for a limit now, or paused on one this week. */
export const pausing = (m: G.MemberClaude, now = Date.now()) =>
  (m.paused_until !== undefined && m.paused_until > now) || (m.pauses?.week ?? 0) > 0;

/** Whether someone's plan runs short for the work: many limit pauses this week. */
export const frequent = (m: G.MemberClaude) => (m.pauses?.week ?? 0) >= FREQUENT_PAUSES;

/** Whether a member belongs under a pill. */
export function matches(m: G.MemberClaude, narrow: Narrow, now = Date.now()): boolean {
  switch (narrow) {
    case "everyone":
      return true;
    case "none":
      return m.state === "none";
    case "soon":
      return runsOutSoon(m, now);
    case "ended":
      return m.state === "ended";
    case "pausing":
      return pausing(m, now);
  }
}

/** The pills, with their counts: those at zero left out, unless it's the one chosen. */
export function pills(
  members: G.MemberClaude[],
  overview: Pick<G.ClaudeOverview, "limit_pauses">,
  chosen: Narrow,
  now = Date.now(),
): { narrow: Narrow; label: string; count: number }[] {
  const all: { narrow: Narrow; label: string }[] = [
    { narrow: "everyone", label: "Everyone" },
    { narrow: "none", label: "No Claude" },
    { narrow: "soon", label: "Runs out soon" },
    { narrow: "ended", label: "Stopped working" },
    { narrow: "pausing", label: overview.limit_pauses ? "Pausing on limits" : "Waiting for a limit" },
  ];
  return all
    .map((p) => ({ ...p, count: members.filter((m) => matches(m, p.narrow, now)).length }))
    .filter((p) => p.narrow === "everyone" || p.count > 0 || p.narrow === chosen);
}

/** What a pill that holds nobody says in the list's place. */
export function nobody(narrow: Narrow, workspace: string): string {
  switch (narrow) {
    case "everyone":
      return `Nobody is in ${workspace} yet.`;
    case "none":
      return `Everyone in ${workspace} has a Claude connection.`;
    case "soon":
      return "Nobody's token runs out within a month.";
    case "ended":
      return `Claude takes everyone's connection in ${workspace}.`;
    case "pausing":
      return "Nobody's sessions paused on a usage limit this week.";
  }
}

const count = (n: number, one: string, many: string) => (n === 1 ? `1 ${one}` : `${n} ${many}`);

/**
 * The line above the list: "11 of 12 people have Claude connected for Acme · 2 tokens run out within
 * a month · 1 stopped working".
 */
export function summary(members: G.MemberClaude[], workspace: string, now = Date.now()): string {
  const total = members.length;
  const connected = members.filter((m) => !blocked(m)).length;
  const first =
    total === 1
      ? connected === 1
        ? `Its one person has Claude connected for ${workspace}`
        : `Its one person has no working Claude connection for ${workspace}`
      : `${connected} of ${total} people ${connected === 1 ? "has" : "have"} Claude connected for ${workspace}`;
  const soon = members.filter((m) => runsOutSoon(m, now)).length;
  const ended = members.filter((m) => m.state === "ended").length;
  const waiting = members.filter((m) => m.paused_until !== undefined && m.paused_until > now).length;
  return [
    first,
    soon > 0 && `${count(soon, "token runs", "tokens run")} out within a month`,
    ended > 0 && `${ended} stopped working`,
    waiting > 0 && `${count(waiting, "waits", "wait")} for a usage limit to reset`,
  ]
    .filter(Boolean)
    .join(" · ");
}

/** "Long-lived token, set by an admin token", "Claude login", "Token on their runners". */
export function kindLabel(c: Pick<G.MemberClaudeConnection, "kind" | "set_by_admin_token">): string {
  switch (c.kind) {
    case "token":
      return c.set_by_admin_token ? "Long-lived token, set by an admin token" : "Long-lived token";
    case "login":
      return "Claude login";
    case "on_runners":
      return "Token on their runners";
  }
}

/** When a long-lived token runs out, for a line: "runs out around Dec 14", "ran out on Nov 2". */
export function runsOutWords(at: number, estimated: boolean | undefined, now = Date.now()): string {
  const how = estimated ? "around" : "on";
  if (at <= now)
    return estimated ? `may have run out around ${shortDay(at, now)}` : `ran out on ${shortDay(at, now)}`;
  return `runs out ${how} ${shortDay(at, now)}`;
}

/** When a reset comes, briefly: "14:00" today, else "Thu 14:00"-like ("Nov 26"). */
function resetsWords(at: number, now: number): string {
  if (at - now < DAY && new Date(at).getDate() === new Date(now).getDate()) return clock(at);
  return shortDay(at, now);
}

/** One fact of a line, with its tone. */
export interface Fact {
  text: string;
  tone: Tone;
}

/**
 * The facts line under someone's address. With details: what they run on, its plan, Claude's last
 * answer, when its token runs out (amber within a month) and limit pauses. With readiness alone:
 * whether their sessions can run, and when that ends. The workspace's name goes into the problems.
 */
export function facts(m: G.MemberClaude, workspace: string, now = Date.now()): Fact[] {
  if (m.state === "none")
    return [{ text: `No Claude connection for ${workspace}: their sessions here can't start`, tone: "warn" }];
  if (m.state === "ended") {
    const on = m.ended_at ? ` on ${shortDay(m.ended_at, now)}` : "";
    const what =
      m.connection?.kind === "login"
        ? `Claude ended their login${on}`
        : m.connection
          ? `Claude refused their token${on}`
          : `Claude stopped taking their connection${on}`;
    return [{ text: `${what}: their sessions here can't run`, tone: "danger" }];
  }
  const out: Fact[] = [];
  const c = m.connection;
  if (c) {
    out.push({ text: kindLabel(c), tone: "plain" });
    out.push({ text: planName(c.plan) ?? "plan unknown", tone: "plain" });
    if (c.answered_at) out.push({ text: `Claude answered ${ago(c.answered_at, now)}`, tone: "plain" });
    else if (c.used_at) out.push({ text: `used ${ago(c.used_at, now)}`, tone: "plain" });
    else out.push({ text: "no session ran on it yet", tone: "plain" });
  } else {
    out.push({
      text: m.state === "unused" ? "Connected · no session ran on it yet" : "Claude takes their connection",
      tone: "plain",
    });
  }
  if (m.runs_out_at !== undefined)
    out.push({
      text: runsOutWords(m.runs_out_at, m.runs_out_estimated, now),
      tone: within(m.runs_out_at, SOON_DAYS, now) ? "warn" : "plain",
    });
  if (m.paused_until !== undefined && m.paused_until > now)
    out.push({
      text: `waiting for ${theLimit(m.paused_limit)} to reset at ${resetsWords(m.paused_until, now)}`,
      tone: "plain",
    });
  else if (m.pauses && m.pauses.week > 0)
    out.push({
      text: `${count(m.pauses.week, "limit pause", "limit pauses")} this week`,
      tone: frequent(m) ? "warn" : "plain",
    });
  return out;
}

/** The facts line as one string, for tests and screen readers. */
export const factsText = (m: G.MemberClaude, workspace: string, now = Date.now()) =>
  facts(m, workspace, now)
    .map((f) => f.text)
    .join(" · ");

/** "an admin token set", "they connected". */
const howConnected = (c: G.MemberClaudeConnection) =>
  c.set_by_admin_token ? "an admin token set" : "they connected";

/**
 * The whole of it, as the row says once opened: what their sessions here run on and since when, its
 * plan, Claude's last answer, when it runs out and why fctl can say only "around", whether they wait
 * for a limit, and how often they paused on one. Sentences, each a fact.
 */
export function detail(
  m: G.MemberClaude,
  workspace: string,
  limitPauses: boolean,
  now = Date.now(),
): string[] {
  const name = m.user.name;
  if (m.state === "none")
    return [
      `${name} has no Claude connection for ${workspace}: their sessions here can't start until they connect one.`,
    ];
  const c = m.connection;
  const out: string[] = [];
  if (m.state === "ended") {
    const on = m.ended_at ? ` on ${fullDay(m.ended_at)}` : "";
    out.push(
      c?.kind === "login"
        ? `Claude ended ${name}'s login${on}, so their sessions in ${workspace} can't run until they have another connection.`
        : c
          ? `${m.ended_at ? `On ${fullDay(m.ended_at)}, ` : ""}Claude refused the long-lived token ${name}'s sessions in ${workspace} ran on: it was revoked, or it ran out. Their sessions here can't run until they have another.`
          : `Claude stopped taking ${name}'s connection${on}, so their sessions in ${workspace} can't run until they have another.`,
    );
    return out;
  }
  if (c) {
    const plan = planName(c.plan);
    const onPlan = plan ? `, on a${/^[AEIOU]/.test(plan) ? "n" : ""} ${plan} plan` : "";
    const since = fullDay(c.connected_at);
    out.push(
      c.kind === "login"
        ? `${name}'s sessions in ${workspace} run on a Claude login they connected on ${since}${onPlan}.`
        : c.kind === "on_runners"
          ? `${name}'s sessions in ${workspace} run on a long-lived token they keep on their runners, connected on ${since}${onPlan}.`
          : `${name}'s sessions in ${workspace} run on a long-lived token ${howConnected(c)} on ${since}${onPlan}.`,
    );
    if (c.chosen_here) out.push(`They chose it for ${workspace}.`);
    if (c.answered_at) out.push(`Claude last answered it ${ago(c.answered_at, now)}.`);
    else if (c.used_at) out.push(`A session last took it ${ago(c.used_at, now)}.`);
    else out.push("No session ran on it yet.");
  } else {
    out.push(
      m.state === "unused"
        ? `${name} has a Claude connection for ${workspace}, and no session ran on it yet.`
        : `${name}'s sessions in ${workspace} can run: Claude takes their connection.`,
    );
  }
  if (m.runs_out_at !== undefined) {
    const at = fullDay(m.runs_out_at);
    const past = m.runs_out_at <= now;
    const set = c?.set_by_admin_token ? "set" : "connected";
    out.push(
      m.runs_out_estimated
        ? past
          ? `It may have run out around ${at}, a year after it was ${set}: fctl can't read the date from the token.`
          : `It runs out around ${at}, a year after it was ${set}: fctl can't read the date from the token.`
        : past
          ? `It ran out on ${at}, a year after claude setup-token made it.`
          : `It runs out on ${at}, a year after claude setup-token made it.`,
    );
  } else if (c?.kind === "login") {
    out.push("Claude renews the login: it runs until Claude ends it.");
  } else if (c?.kind === "on_runners") {
    out.push("fctl can't tell when it runs out: it never reads a token kept on runners.");
  }
  if (m.paused_until !== undefined && m.paused_until > now)
    out.push(
      `Their sessions here wait for ${theLimit(m.paused_limit)} to reset, at ${clock(m.paused_until)}${
        m.paused_until - now >= DAY ? ` on ${shortDay(m.paused_until, now)}` : ""
      }.`,
    );
  const p = m.pauses;
  if (limitPauses && p) {
    if (p.week > 0)
      out.push(
        `Their sessions paused on usage limits ${p.week === 1 ? "once" : `${p.week} times`} this week${
          p.month > p.week ? ` and ${p.month} times in 30 days` : ""
        }, the last on ${theLimit(p.last_limit)}.`,
      );
    else if (p.month > 0)
      out.push(
        `Their sessions paused on usage limits ${p.month === 1 ? "once" : `${p.month} times`} in the last 30 days, none this week.`,
      );
    else out.push("Their sessions haven't paused on a usage limit in 30 days.");
    if (frequent(m)) out.push("Their plan's limits are short for this work: a Max or Ultra seat has more.");
  }
  return out;
}

/** Whether admins have something to do about someone: a way out shows when the row opens. */
export const needsAction = (m: G.MemberClaude, now = Date.now()) => blocked(m) || runsOutSoon(m, now);

/** The command that sets someone a new token with an admin token, from one `claude setup-token` printed. */
export const adminTokenCommand = (email: string, workspace: string) =>
  `fctl admin users set ${email} --claude-token - --claude-label "${workspace.replaceAll('"', "'")} seat"`;

/** The link to Settings → Claude, to send someone. */
export const settingsLink = (origin: string) => `${origin}/settings?tab=claude`;

/** Where Claude Team and Enterprise seats are given: Claude's guide to its members settings. */
export const SEATS_HELP =
  "https://support.claude.com/en/articles/13133750-manage-members-on-team-and-enterprise-plans";

/** What a workspace's admins see, as its "You" settings tell its members. */
export function whatAdminsSee(workspace: string, oversight: G.ClaudeOversight, limitPauses: boolean): string {
  switch (oversight) {
    case "off":
      return `${workspace}'s admins see nothing of your Claude connection.`;
    case "readiness":
      return `${workspace}'s admins see whether your sessions here can run on Claude and when your token runs out: never what kind of connection it is, its plan, the token or the account.`;
    case "details":
      return `${workspace}'s admins see what kind of Claude connection your sessions here run on, its plan, when Claude last answered it and when it runs out${
        limitPauses ? ", and how often your sessions paused on usage limits" : ""
      }: never the token or the account.`;
  }
}

/** The levels a workspace chooses from, least last, for its settings. */
export const OVERSIGHT: { value: G.ClaudeOversight; label: string; description: string }[] = [
  {
    value: "details",
    label: "Details",
    description: "Kind of connection, plan, Claude's last answer, when it runs out",
  },
  {
    value: "readiness",
    label: "Whether sessions can run",
    description: "Connected or not, working or not, when a token runs out",
  },
  { value: "off", label: "Nothing", description: "Admins see nothing of members' Claude connections" },
];

const RANK: Record<G.ClaudeOversight, number> = { off: 0, readiness: 1, details: 2 };

/** Whether `a` shows more than `b`. */
export const more = (a: G.ClaudeOversight, b: G.ClaudeOversight) => RANK[a] > RANK[b];

/** The ways admins hear by email. */
export const ADMIN_EMAIL: { value: G.ClaudeAdminEmail; label: string; description: string }[] = [
  {
    value: "weekly",
    label: "Once a week",
    description: "One email while a token runs out within a month or stopped working",
  },
  {
    value: "each",
    label: "Each time",
    description: "30 and 7 days before a token runs out, and when one stops",
  },
  { value: "off", label: "Not by email", description: "The People tab shows it all the same" },
];

/** Your own connections, for the Agent Floor and Settings: a token in its last week, and whether all ended. */
export function yourClaude(claude: G.ClaudeCredentialInfo[], now = Date.now()) {
  const token = (c: G.ClaudeCredentialInfo) => c.kind === "setup_token" && !c.on_runners;
  const ending = claude
    .filter((c) => c.status === "active" && token(c) && c.expires_at !== undefined)
    .filter((c) => within(c.expires_at, LAST_DAYS, now))
    .sort((a, b) => (a.expires_at ?? 0) - (b.expires_at ?? 0))[0];
  const working = claude.filter((c) => c.status === "active");
  const ended =
    claude.length > 0 && working.length === 0 ? claude.find((c) => c.status === "invalid") : undefined;
  return { ending, ended };
}

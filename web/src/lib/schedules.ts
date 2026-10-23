// Schedules (G.Schedule): tasks fctl starts a session for on its own, once at a set time or again and
// again, on the Claude subscription of the person each runs for. The control plane decides when they
// run (fctl_domain::cron and ::schedules); this reads cron expressions as it does, so that the form can
// say in words when one runs and show its next runs as you choose, in any time zone the browser knows.
//
// A time the clock skips as it moves to summer time runs as far past the change as it was meant to be
// (02:30 runs at 03:30), and one the clock goes through twice runs once, the first time, as there.

import type * as G from "@/gen";

const MINUTE = 60_000;
const HOUR = 60 * MINUTE;
const DAY = 24 * HOUR;
/** How far ahead runs are looked for: an expression that names no time within it never runs. */
const HORIZON_DAYS = 5 * 366;
/** How far a zone's clocks move at a change, at most, as far as finding runs goes. */
const MAX_SHIFT = 3 * HOUR;
/** Schedules run at most this often (fctl_domain::schedules::MIN_GAP_MINUTES). */
export const MIN_GAP_MINUTES = 15;

const MONTHS = ["jan", "feb", "mar", "apr", "may", "jun", "jul", "aug", "sep", "oct", "nov", "dec"];
const WEEKDAYS = ["sun", "mon", "tue", "wed", "thu", "fri", "sat"];
export const WEEKDAY_NAMES = ["Sunday", "Monday", "Tuesday", "Wednesday", "Thursday", "Friday", "Saturday"];

// ───────────────────────────── cron ─────────────────────────────

/** A cron expression, read: the values each field names, and whether the day fields start with `*`. */
export interface Cron {
  minutes: number[];
  hours: number[];
  days: number[];
  months: number[];
  /** 0 is Sunday. */
  weekdays: number[];
  anyDay: boolean;
  anyWeekday: boolean;
  /** The expression, its fields one space apart. */
  text: string;
}

const SHORTHANDS: Record<string, string> = {
  "@yearly": "0 0 1 1 *",
  "@annually": "0 0 1 1 *",
  "@monthly": "0 0 1 * *",
  "@weekly": "0 0 * * 0",
  "@daily": "0 0 * * *",
  "@midnight": "0 0 * * *",
  "@hourly": "0 * * * *",
};

/** One field's values, from `lo` to `hi`, with `names` for `lo` onwards; `what` names it in errors. */
function field(text: string, lo: number, hi: number, names: string[], what: string): number[] {
  const value = (v: string): number => {
    const named = names.indexOf(v);
    if (named >= 0) return named + lo;
    if (!/^\d+$/.test(v)) throw new Error(`${v} isn't a ${what}`);
    const n = Number(v);
    if (n < lo || n > hi) throw new Error(`The ${what} must be from ${lo} to ${hi}, not ${n}`);
    return n;
  };
  const out = new Set<number>();
  for (const part of text.split(",")) {
    if (!part) throw new Error(`The ${what} has an empty item in its list`);
    const [range = "", stepText] = part.split("/", 2);
    let step = 1;
    if (stepText !== undefined) {
      step = /^\d+$/.test(stepText) ? Number(stepText) : 0;
      if (step < 1 || step > hi) throw new Error(`${stepText} isn't a step for the ${what}`);
    }
    let first: number;
    let last: number;
    if (range === "*") {
      first = lo;
      last = hi;
    } else if (range.includes("-")) {
      const [a = "", b = ""] = range.split("-", 2);
      first = value(a);
      last = value(b);
      if (first > last)
        throw new Error(`The ${what} range ${range} runs backwards: write it from the first to the last`);
    } else {
      first = value(range);
      last = stepText !== undefined ? hi : first;
    }
    for (let v = first; v <= last; v += step) out.add(v);
  }
  return [...out].sort((a, b) => a - b);
}

/** Reads a cron expression; the error says what is wrong with it, for people. */
export function parseCron(input: string): { cron: Cron } | { error: string } {
  const text = input.trim().split(/\s+/).join(" ");
  if (!text) return { error: "Write a cron expression, like 0 9 * * 1-5" };
  const lower = text.toLowerCase();
  let expanded = lower;
  if (lower.startsWith("@")) {
    const s = SHORTHANDS[lower];
    if (!s)
      return {
        error: `${text} isn't a shorthand fctl knows: use @hourly, @daily, @weekly, @monthly or @yearly`,
      };
    expanded = s;
  }
  const parts = expanded.split(" ");
  if (parts.length !== 5)
    return {
      error:
        "Use five fields: the minute, the hour, the day of the month, the month and the day of the week, like 0 9 * * 1-5",
    };
  const [minute = "", hour = "", day = "", month = "", weekday = ""] = parts;
  try {
    const weekdays = field(weekday, 0, 7, WEEKDAYS, "day of the week").map((d) => d % 7);
    return {
      cron: {
        minutes: field(minute, 0, 59, [], "minute"),
        hours: field(hour, 0, 23, [], "hour"),
        days: field(day, 1, 31, [], "day of the month"),
        months: field(month, 1, 12, MONTHS, "month"),
        weekdays: [...new Set(weekdays)].sort((a, b) => a - b),
        anyDay: day.startsWith("*"),
        anyWeekday: weekday.startsWith("*"),
        text: lower.startsWith("@") ? lower : text,
      },
    };
  } catch (e) {
    return { error: (e as Error).message };
  }
}

/** Whether it runs on a day: its day of the month, month and day of the week (Vixie cron's rule). */
function runsOn(c: Cron, day: number, month: number, weekday: number): boolean {
  if (!c.months.includes(month)) return false;
  const byDay = c.days.includes(day);
  const byWeekday = c.weekdays.includes(weekday);
  return c.anyDay || c.anyWeekday ? byDay && byWeekday : byDay || byWeekday;
}

/** The shortest time between two of its runs, in minutes, as if it ran every day. */
export function shortestGap(c: Cron): number {
  const times = c.hours.flatMap((h) => c.minutes.map((m) => h * 60 + m)).sort((a, b) => a - b);
  const first = times[0];
  const last = times[times.length - 1];
  if (first === undefined || last === undefined) return 24 * 60;
  let gap = 24 * 60 - last + first;
  for (let i = 1; i < times.length; i++) gap = Math.min(gap, (times[i] ?? 0) - (times[i - 1] ?? 0));
  return gap;
}

/** The first wall-clock time it names at or after `from` (ms since the local epoch), within five years. */
export function nextLocal(c: Cron, from: number): number | undefined {
  const start = Math.ceil(from / MINUTE);
  const first = Math.floor(start / (24 * 60));
  let minuteOfDay = start - first * 24 * 60;
  for (let day = first; day < first + HORIZON_DAYS; day++) {
    const d = new Date(day * DAY);
    if (runsOn(c, d.getUTCDate(), d.getUTCMonth() + 1, d.getUTCDay())) {
      for (const h of c.hours) {
        if (h < Math.floor(minuteOfDay / 60)) continue;
        const from = h === Math.floor(minuteOfDay / 60) ? minuteOfDay % 60 : 0;
        const m = c.minutes.find((x) => x >= from);
        if (m !== undefined) return day * DAY + h * HOUR + m * MINUTE;
      }
    }
    minuteOfDay = 0;
  }
  return undefined;
}

// ───────────────────────────── time zones ─────────────────────────────

const formats = new Map<string, Intl.DateTimeFormat>();

function zoneFormat(zone: string): Intl.DateTimeFormat {
  let f = formats.get(zone);
  if (!f) {
    f = new Intl.DateTimeFormat("en-US", {
      timeZone: zone,
      hourCycle: "h23",
      year: "numeric",
      month: "numeric",
      day: "numeric",
      hour: "numeric",
      minute: "numeric",
      second: "numeric",
    });
    formats.set(zone, f);
  }
  return f;
}

/** Whether the browser knows the time zone. */
export function knownZone(zone: string): boolean {
  try {
    zoneFormat(zone);
    return true;
  } catch {
    return false;
  }
}

/** How far ahead of UTC a zone's clocks are at `utc` (ms; negative where they're behind). */
export function offsetMs(zone: string, utc: number): number {
  const parts: Record<string, number> = {};
  for (const p of zoneFormat(zone).formatToParts(utc)) parts[p.type] = Number(p.value);
  const wall = Date.UTC(
    parts.year ?? 1970,
    (parts.month ?? 1) - 1,
    parts.day ?? 1,
    (parts.hour ?? 0) % 24,
    parts.minute ?? 0,
    parts.second ?? 0,
  );
  return wall - Math.floor(utc / 1000) * 1000;
}

/** The wall-clock time in `zone` at `utc`, as ms since a local epoch. */
export const wallMs = (zone: string, utc: number) => utc + offsetMs(zone, utc);

/**
 * The instant a wall-clock time in `zone` names: the first, when the clock goes through it twice;
 * for a time the clock skips, the instant as far past the change as it is past the change's start.
 */
export function instant(zone: string, local: number): number {
  const before = offsetMs(zone, local - DAY);
  const after = offsetMs(zone, local + DAY);
  let first: number | undefined;
  for (const o of [before, after]) {
    const utc = local - o;
    if (offsetMs(zone, utc) === o) first = first === undefined ? utc : Math.min(first, utc);
  }
  return first ?? local - before;
}

/** The first time after `after` (ms) that `c` names in `zone`, within five years. */
export function nextAfter(c: Cron, zone: string, after: number): number | undefined {
  let from = wallMs(zone, after) - MAX_SHIFT;
  let best: { utc: number; at: number } | undefined;
  for (let i = 0; i < 10_000; i++) {
    const at = nextLocal(c, from);
    if (at === undefined || (best && at > best.at + MAX_SHIFT)) break;
    const utc = instant(zone, at);
    if (utc > after && (!best || utc < best.utc)) best = { utc, at };
    from = at + MINUTE;
  }
  return best?.utc;
}

/** The next `n` runs of `c` in `zone` after `from`. */
export function nextRuns(c: Cron, zone: string, from: number, n: number): number[] {
  const out: number[] = [];
  let at = from;
  for (let i = 0; i < n; i++) {
    const next = nextAfter(c, zone, at);
    if (next === undefined) break;
    out.push(next);
    at = next;
  }
  return out;
}

/** The browser's time zone, else UTC. */
export function browserZone(): string {
  try {
    return Intl.DateTimeFormat().resolvedOptions().timeZone || "UTC";
  } catch {
    return "UTC";
  }
}

/** The zones the browser knows, UTC first. */
export function zones(): string[] {
  const known = (Intl as { supportedValuesOf?: (key: string) => string[] }).supportedValuesOf?.("timeZone");
  const list = known?.length ? known : [browserZone()];
  return ["UTC", ...list.filter((z) => z !== "UTC" && z !== "Etc/UTC")];
}

/** "New York", "Buenos Aires", "UTC": the last part of a zone's name, as people say it. */
export const zoneCity = (zone: string) => (zone.split("/").pop() ?? zone).replaceAll("_", " ");

/** "America", "Europe", or "" for UTC and the like. */
export const zoneRegion = (zone: string) => (zone.includes("/") ? (zone.split("/")[0] ?? "") : "");

/** "UTC+02:00", "UTC−04:00", "UTC" at `at`. */
export function zoneOffset(zone: string, at = Date.now()): string {
  const m = Math.round(offsetMs(zone, at) / MINUTE);
  if (m === 0) return "UTC";
  const a = Math.abs(m);
  return `UTC${m < 0 ? "−" : "+"}${String(Math.floor(a / 60)).padStart(2, "0")}:${String(a % 60).padStart(2, "0")}`;
}

/** A wall-clock date and time in `zone`: `{ date: "2026-10-30", time: "14:00" }`. */
export function wallParts(zone: string, utc: number): { date: string; time: string } {
  const iso = new Date(Math.floor(wallMs(zone, utc) / MINUTE) * MINUTE).toISOString();
  return { date: iso.slice(0, 10), time: iso.slice(11, 16) };
}

/** The instant of a wall-clock date and time in `zone` ("2026-10-30", "14:00"); NaN for neither. */
export function fromWall(zone: string, date: string, time: string): number {
  const m = /^(\d{4})-(\d{2})-(\d{2})$/.exec(date);
  const t = /^(\d{2}):(\d{2})$/.exec(time);
  if (!m || !t) return Number.NaN;
  const local = Date.UTC(Number(m[1]), Number(m[2]) - 1, Number(m[3]), Number(t[1]), Number(t[2]));
  return instant(zone, local);
}

// ───────────────────────────── in words ─────────────────────────────

/** How a time of day reads: "09:00" by default, the locale's way (`clockOf`) in the app. */
export type ClockText = (hour: number, minute: number) => string;

export const clock24: ClockText = (h, m) => `${String(h).padStart(2, "0")}:${String(m).padStart(2, "0")}`;

const localClock = new Intl.DateTimeFormat(undefined, {
  hour: "2-digit",
  minute: "2-digit",
  timeZone: "UTC",
});
/** A time of day as the browser's locale writes it, as `clock` does in the app: "09:00 AM", "09:00". */
export const clockOf: ClockText = (h, m) => localClock.format(Date.UTC(2000, 0, 1, h, m));

/** "a", "a and b", "a, b and c". */
export function list(items: string[]): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} and ${items[items.length - 1]}`;
}

/** "1st", "2nd", "3rd", "4th", "11th", "21st". */
export function ordinal(n: number): string {
  const s = n % 100 >= 11 && n % 100 <= 13 ? "th" : ({ 1: "st", 2: "nd", 3: "rd" }[n % 10] ?? "th");
  return `${n}${s}`;
}

/** The step between `values` when they start at 0 and go evenly round a `period`. */
function step(values: number[], period: number): number | undefined {
  const [first, second] = values;
  if (first !== 0 || second === undefined) return undefined;
  const n = second - first;
  const even = period % n === 0 && values.length === period / n && values.every((v, i) => v === i * n);
  return even ? n : undefined;
}

const ALL_MONTHS = [1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12].join();

/**
 * When it runs, in words, after "Runs": "every weekday at 09:00", "every hour at :15", "on the 1st of
 * every month at 08:30"; the expression itself where no shorter phrase says it, as the CLI says it
 * (fctl_domain::cron::Cron::describe).
 */
export function describeCron(c: Cron, clock: ClockText = clock24): string {
  const fallback = `on the cron schedule ${c.text}`;
  const everyDay = c.anyDay && c.anyWeekday && c.months.join() === ALL_MONTHS;
  const { minutes, hours } = c;
  if (everyDay && hours.length === 24 && minutes.length > 1) {
    const n = step(minutes, 60);
    return n ? `every ${n} minutes` : fallback;
  }
  const [minute] = minutes;
  if (minute === undefined || minutes.length !== 1) return fallback;
  if (everyDay && hours.length > 4 && hours.length < 24) {
    const n = step(hours, 24);
    if (n)
      return minute === 0 ? `every ${n} hours` : `every ${n} hours at :${String(minute).padStart(2, "0")}`;
  }
  if (hours.length === 24) return everyDay ? `every hour at :${String(minute).padStart(2, "0")}` : fallback;
  if (hours.length > 4) return fallback;
  const times = list(hours.map((h) => clock(h, minute)));
  if (everyDay) return `every day at ${times}`;
  if (c.months.join() !== ALL_MONTHS) return fallback;
  if (c.anyDay && !c.anyWeekday) {
    const w = c.weekdays.join();
    if (w === "1,2,3,4,5") return `every weekday at ${times}`;
    if (w === "0,6") return `every Saturday and Sunday at ${times}`;
    const names = [...c.weekdays]
      .sort((a, b) => ((a + 6) % 7) - ((b + 6) % 7))
      .map((d) => WEEKDAY_NAMES[d] ?? "");
    return `every ${list(names)} at ${times}`;
  }
  if (!c.anyDay && c.anyWeekday && c.days.length <= 4)
    return `on the ${list(c.days.map(ordinal))} of every month at ${times}`;
  return fallback;
}

/** "Every weekday at 9:00 AM": `describeCron` at the start of a sentence. */
export const sentence = (words: string) => words.charAt(0).toUpperCase() + words.slice(1);

// ───────────────────────────── the form's choices ─────────────────────────────

/** How often the form's "Repeats" runs a schedule. */
export type Every = "hour" | "day" | "weekday" | "week" | "month";

/** What the form's "Repeats" holds: each kind reads the parts it needs. */
export interface Repeat {
  every: Every;
  hour: number;
  minute: number;
  /** For `week`: the days, 0 for Sunday. */
  weekdays: number[];
  /** For `month`: the day of the month. */
  day: number;
}

export const DEFAULT_REPEAT: Repeat = { every: "week", hour: 9, minute: 0, weekdays: [1], day: 1 };

/** The cron expression of the form's choices. */
export function repeatCron(r: Repeat): string {
  const at = `${r.minute} ${r.hour}`;
  switch (r.every) {
    case "hour":
      return `${r.minute} * * * *`;
    case "day":
      return `${at} * * *`;
    case "weekday":
      return `${at} * * 1-5`;
    case "week":
      return `${at} * * ${[...new Set(r.weekdays.length ? r.weekdays : [1])].sort((a, b) => a - b).join(",")}`;
    case "month":
      return `${at} ${r.day} * *`;
  }
}

/** The form's choices for a cron expression, when they can say it; else it shows as cron. */
export function cronRepeat(text: string): Repeat | undefined {
  const parsed = parseCron(text);
  if ("error" in parsed) return undefined;
  const c = parsed.cron;
  const [minute] = c.minutes;
  if (c.minutes.length !== 1 || minute === undefined || c.months.join() !== ALL_MONTHS) return undefined;
  const base = { ...DEFAULT_REPEAT, minute };
  if (c.hours.length === 24)
    return c.anyDay && c.anyWeekday ? { ...base, every: "hour", hour: 9 } : undefined;
  const [hour] = c.hours;
  if (c.hours.length !== 1 || hour === undefined) return undefined;
  if (c.anyDay && c.anyWeekday) return { ...base, every: "day", hour };
  if (c.anyDay && !c.anyWeekday) {
    if (c.weekdays.join() === "1,2,3,4,5")
      return { ...base, every: "weekday", hour, weekdays: [1, 2, 3, 4, 5] };
    return { ...base, every: "week", hour, weekdays: c.weekdays };
  }
  const [day] = c.days;
  if (!c.anyDay && c.anyWeekday && c.days.length === 1 && day !== undefined)
    return { ...base, every: "month", hour, day };
  return undefined;
}

/** Why the form's cron expression won't do, or nothing: as the control plane checks it. */
export function cronProblem(text: string): string | undefined {
  const parsed = parseCron(text);
  if ("error" in parsed) return parsed.error;
  if (shortestGap(parsed.cron) < MIN_GAP_MINUTES)
    return `Schedules run at most every ${MIN_GAP_MINUTES} minutes: each run is a session of its own`;
  if (nextAfter(parsed.cron, "UTC", Date.now()) === undefined)
    return "It names no time in the next five years, so it would never run";
  return undefined;
}

// ───────────────────────────── schedules ─────────────────────────────

/** When a schedule runs, at the start of a sentence: "Every Monday at 9:00 AM", "Once, on Fri 30 Oct, 14:00". */
export function scheduleWhen(s: Pick<G.Schedule, "cron" | "at" | "timezone">, clock = clockOf): string {
  if (s.cron) {
    const parsed = parseCron(s.cron);
    return sentence("error" in parsed ? `on the cron schedule ${s.cron}` : describeCron(parsed.cron, clock));
  }
  if (s.at !== undefined) return `Once, on ${moment(s.at, s.timezone)}`;
  return "When someone runs it";
}

/** A time in a zone for lists: "Mon 26 Oct, 09:00", with the year when it isn't this year's. */
export function moment(at: number, zone: string, now = Date.now()): string {
  const year = new Date(at).getUTCFullYear() !== new Date(now).getUTCFullYear();
  return new Date(at).toLocaleString(undefined, {
    timeZone: zone,
    weekday: "short",
    day: "numeric",
    month: "short",
    year: year ? "numeric" : undefined,
    hour: "2-digit",
    minute: "2-digit",
  });
}

/** What someone may do with a schedule (fctl_domain::schedules::rights). */
export interface Rights {
  /** Run it now, on their own subscription. */
  run: boolean;
  /** Change or resume it: someone else's then runs on their subscription. */
  edit: boolean;
  /** Pause or delete it. */
  manage: boolean;
}

const RANK: Record<G.ProjectRole, number> = { viewer: 0, editor: 1, maintainer: 2, owner: 3 };

export function scheduleRights(role: G.ProjectRole | undefined, owner: boolean): Rights {
  const starts = !!role && RANK[role] >= RANK.editor;
  const maintains = !!role && RANK[role] >= RANK.maintainer;
  return { run: starts, edit: starts && (owner || maintains), manage: maintains || (owner && !!role) };
}

/** Whether people who see the project may set up schedules in it: those who start sessions. */
export const canSchedule = (role: G.ProjectRole | undefined) => scheduleRights(role, false).run;

/** A run, in a few words: "Started #41", "Skipped", "Failed". */
export function runOutcome(r: G.ScheduleRun): string {
  switch (r.outcome) {
    case "started":
      return r.session ? `Started #${r.session.number}` : "Started";
    case "skipped":
      return "Skipped";
    case "failed":
      return "Failed";
  }
}

/** Who started a run: "On time", "Ada Lovelace", "A tool, with Ada's token". */
export function runTrigger(r: G.ScheduleRun): string {
  if (r.trigger === "time") return "On time";
  const name = r.by?.name ?? "Someone";
  return r.trigger === "token" ? `A tool, with ${name}'s token` : name;
}

/**
 * A run's first message, as its task and fctl's note about the run after it
 * (fctl_domain::schedules::prompt), for the transcript to show apart; the
 * whole text as the task for any other message.
 */
export function splitRunNote(text: string): { task: string; note?: string } {
  const m = /\n\n---\n(?=[^\n]* started this session on [^\n]* the project's schedule ")/.exec(text);
  if (!m) return { task: text };
  return { task: text.slice(0, m.index), note: text.slice(m.index + m[0].length) };
}

/**
 * The name a schedule gets without one: its task's first line, cut at a word
 * to at most 72 characters (fctl_domain::naming::title_from_prompt).
 */
export function nameFromTask(task: string): string {
  const line =
    task
      .split("\n")
      .map((l) =>
        l
          .trim()
          .replace(/^[#>\-* ]+/, "")
          .trim(),
      )
      .find((l) => l) ?? "";
  const chars = [...line];
  if (chars.length <= 72) return line;
  const cut = chars.slice(0, 71).join("");
  const space = cut.search(/\s\S*$/);
  const kept = space > 36 ? cut.slice(0, space) : cut;
  return `${kept.replace(/[\s\p{P}]+$/u, "")}…`;
}

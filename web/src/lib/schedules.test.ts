import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  type Cron,
  capProblem,
  cronProblem,
  cronRepeat,
  DEFAULT_REPEAT,
  describeCaps,
  describeCron,
  describeEvent,
  eventBy,
  eventCaps,
  eventSaid,
  fromWall,
  instant,
  nameFromTask,
  nextAfter,
  nextRuns,
  offsetMs,
  orList,
  parseCron,
  type Repeat,
  readRunOn,
  repeatCron,
  runOnContext,
  runOutcome,
  runTrigger,
  scheduleRights,
  scheduleWhen,
  shortestGap,
  splitRunNote,
  wallParts,
  zoneCity,
  zoneOffset,
  zoneRegion,
} from "./schedules";

const cron = (s: string): Cron => {
  const p = parseCron(s);
  if ("error" in p) throw new Error(p.error);
  return p.cron;
};
const error = (s: string) => {
  const p = parseCron(s);
  return "error" in p ? p.error : "";
};
const utc = (y: number, mo: number, d: number, h: number, mi: number) => Date.UTC(y, mo - 1, d, h, mi);
const STOCKHOLM = "Europe/Stockholm";

describe("cron expressions", () => {
  it("reads fields, ranges, steps, lists and names", () => {
    const c = cron("*/15 9-17 * * mon-fri");
    expect(c.minutes).toEqual([0, 15, 30, 45]);
    expect(c.hours).toEqual([9, 10, 11, 12, 13, 14, 15, 16, 17]);
    expect(c.weekdays).toEqual([1, 2, 3, 4, 5]);
    expect([c.anyDay, c.anyWeekday]).toEqual([true, false]);
    const d = cron("5/20 0 1,15 JAN,jul 7");
    expect(d.minutes).toEqual([5, 25, 45]);
    expect(d.days).toEqual([1, 15]);
    expect(d.months).toEqual([1, 7]);
    expect(d.weekdays).toEqual([0]);
    expect(cron("  0   9 * *  1 ").text).toBe("0 9 * * 1");
    expect(cron("@Daily").text).toBe("@daily");
    expect(cron("@weekly").weekdays).toEqual([0]);
  });

  it("says what is wrong, as the control plane does", () => {
    for (const [bad, says] of [
      ["", "Write a cron expression"],
      ["0 9 * *", "five fields"],
      ["60 * * * *", "minute must be from 0 to 59, not 60"],
      ["0 24 * * *", "hour must be from 0 to 23"],
      ["0 0 0 * *", "day of the month must be from 1 to 31, not 0"],
      ["0 0 * 13 *", "month must be from 1 to 12"],
      ["0 0 * * 8", "day of the week must be from 0 to 7"],
      ["0 0 * * fri-mon", "runs backwards"],
      ["*/0 * * * *", "isn't a step"],
      ["0,,5 * * * *", "empty item"],
      ["x * * * *", "x isn't a minute"],
      ["@reboot", "isn't a shorthand"],
    ] as const) {
      expect(error(bad), bad).toContain(says);
    }
    expect(cronProblem("*/5 * * * *")).toContain("every 15 minutes");
    expect(cronProblem("0 0 30 2 *")).toContain("never run");
    expect(cronProblem("*/15 * * * *")).toBeUndefined();
  });

  it("says when it runs, as the CLI does", () => {
    for (const [expr, says] of [
      ["0 9 * * *", "every day at 09:00"],
      ["@daily", "every day at 00:00"],
      ["0 9,17 * * *", "every day at 09:00 and 17:00"],
      ["15 * * * *", "every hour at :15"],
      ["0 9 * * 1-5", "every weekday at 09:00"],
      ["30 8 * * 1", "every Monday at 08:30"],
      ["0 16 * * 1,4", "every Monday and Thursday at 16:00"],
      ["0 16 * * 5,3,1", "every Monday, Wednesday and Friday at 16:00"],
      ["0 10 * * 0", "every Sunday at 10:00"],
      ["0 10 * * 0,6", "every Saturday and Sunday at 10:00"],
      ["0 10 * * 6,0,1", "every Monday, Saturday and Sunday at 10:00"],
      ["0 8 1 * *", "on the 1st of every month at 08:00"],
      ["0 8 1,15 * *", "on the 1st and 15th of every month at 08:00"],
      ["0 8 22 * *", "on the 22nd of every month at 08:00"],
      ["*/15 * * * *", "every 15 minutes"],
      ["0,30 * * * *", "every 30 minutes"],
      ["5,35 * * * *", "on the cron schedule 5,35 * * * *"],
      ["0 */2 * * *", "every 2 hours"],
      ["15 */3 * * *", "every 3 hours at :15"],
      ["0 */6 * * *", "every day at 00:00, 06:00, 12:00 and 18:00"],
      ["0 */5 * * *", "on the cron schedule 0 */5 * * *"],
      ["*/15 9-17 * * 1-5", "on the cron schedule */15 9-17 * * 1-5"],
      ["0 9 1-7 * 1", "on the cron schedule 0 9 1-7 * 1"],
      ["0 9 * 1 *", "on the cron schedule 0 9 * 1 *"],
      ["15 * * * 1", "on the cron schedule 15 * * * 1"],
      // A star-led field with a step restricts the days.
      ["0 9 */2 * *", "on the cron schedule 0 9 */2 * *"],
      ["0 9 */2 * 1", "on the cron schedule 0 9 */2 * 1"],
      ["0 9 1 * */2", "on the cron schedule 0 9 1 * */2"],
      ["*/15 * */2 * *", "on the cron schedule */15 * */2 * *"],
      ["0 9 */1 * 1", "every Monday at 09:00"],
      ["0 9 1-31 * */2", "every Tuesday, Thursday, Saturday and Sunday at 09:00"],
      // Neither starts with a star: either will do, and 0-6 is every day.
      ["0 8 1 * 0-6", "every day at 08:00"],
      ["0 8 1 * 1", "on the cron schedule 0 8 1 * 1"],
    ] as const) {
      expect(describeCron(cron(expr)), expr).toBe(says);
    }
  });

  it("knows the shortest gap between runs", () => {
    expect(shortestGap(cron("*/15 * * * *"))).toBe(15);
    expect(shortestGap(cron("5,55 0,23 * * *"))).toBe(10);
    expect(shortestGap(cron("0 9 * * *"))).toBe(24 * 60);
  });
});

describe("next runs", () => {
  const now = utc(2026, 10, 23, 10, 41);

  it("finds them in UTC", () => {
    expect(nextRuns(cron("0 9 * * 1-5"), "UTC", now, 3)).toEqual([
      utc(2026, 10, 26, 9, 0),
      utc(2026, 10, 27, 9, 0),
      utc(2026, 10, 28, 9, 0),
    ]);
    expect(nextAfter(cron("0 12 29 2 *"), "UTC", now)).toBe(utc(2028, 2, 29, 12, 0));
    expect(nextAfter(cron("0 0 30 2 *"), "UTC", now)).toBeUndefined();
    // The 1st, or any Monday.
    expect(nextRuns(cron("0 8 1 * mon"), "UTC", utc(2026, 10, 23, 0, 0), 3)).toEqual([
      utc(2026, 10, 26, 8, 0),
      utc(2026, 11, 1, 8, 0),
      utc(2026, 11, 2, 8, 0),
    ]);
  });

  it("follows the zone's summer time", () => {
    expect(nextRuns(cron("0 9 * * *"), STOCKHOLM, utc(2026, 10, 23, 12, 0), 3)).toEqual([
      utc(2026, 10, 24, 7, 0),
      utc(2026, 10, 25, 8, 0),
      utc(2026, 10, 26, 8, 0),
    ]);
    // 02:30 doesn't happen on 29 March 2026: it runs at 03:30 summer time.
    expect(nextAfter(cron("30 2 * * *"), STOCKHOLM, utc(2026, 3, 28, 12, 0))).toBe(utc(2026, 3, 29, 1, 30));
    // It happens twice on 25 October 2026: it runs the first time.
    expect(nextAfter(cron("30 2 * * *"), STOCKHOLM, utc(2026, 10, 24, 12, 0))).toBe(utc(2026, 10, 25, 0, 30));
    expect(nextRuns(cron("*/30 * * * *"), STOCKHOLM, utc(2026, 3, 29, 0, 15), 4)).toEqual([
      utc(2026, 3, 29, 0, 30),
      utc(2026, 3, 29, 1, 0),
      utc(2026, 3, 29, 1, 30),
      utc(2026, 3, 29, 2, 0),
    ]);
  });

  it("reads wall-clock times in a zone", () => {
    expect(offsetMs(STOCKHOLM, utc(2026, 7, 1, 0, 0))).toBe(2 * 3_600_000);
    expect(offsetMs("America/New_York", utc(2026, 1, 1, 0, 0))).toBe(-5 * 3_600_000);
    expect(instant(STOCKHOLM, utc(2026, 3, 29, 2, 30))).toBe(utc(2026, 3, 29, 1, 30));
    expect(fromWall(STOCKHOLM, "2026-10-30", "14:00")).toBe(utc(2026, 10, 30, 13, 0));
    expect(fromWall(STOCKHOLM, "2026-10-30", "")).toBeNaN();
    expect(wallParts(STOCKHOLM, utc(2026, 10, 30, 13, 0))).toEqual({ date: "2026-10-30", time: "14:00" });
    expect(zoneOffset(STOCKHOLM, utc(2026, 7, 1, 0, 0))).toBe("UTC+02:00");
    expect(zoneOffset("America/St_Johns", utc(2026, 1, 1, 0, 0))).toBe("UTC−03:30");
    expect(zoneOffset("UTC")).toBe("UTC");
    expect([
      zoneCity("America/Argentina/Buenos_Aires"),
      zoneRegion("America/Argentina/Buenos_Aires"),
    ]).toEqual(["Buenos Aires", "America"]);
    expect([zoneCity("UTC"), zoneRegion("UTC")]).toEqual(["UTC", ""]);
  });
});

describe("the form's choices", () => {
  it("make cron expressions and read them back", () => {
    const at: Repeat = { ...DEFAULT_REPEAT, hour: 7, minute: 30 };
    const cases: [Repeat, string][] = [
      [{ ...at, every: "hour", minute: 15 }, "15 * * * *"],
      [{ ...at, every: "day" }, "30 7 * * *"],
      [{ ...at, every: "weekday" }, "30 7 * * 1-5"],
      [{ ...at, every: "week", weekdays: [5, 1] }, "30 7 * * 1,5"],
      [{ ...at, every: "month", day: 15 }, "30 7 15 * *"],
    ];
    for (const [r, expr] of cases) {
      expect(repeatCron(r)).toBe(expr);
      const back = cronRepeat(expr);
      expect(back?.every).toBe(r.every);
      expect(back && repeatCron(back)).toBe(expr);
    }
    expect(cronRepeat("0 9 * * mon-fri")?.every).toBe("weekday");
    expect(cronRepeat("@weekly")).toMatchObject({ every: "week", weekdays: [0], hour: 0, minute: 0 });
    // What the form can't say shows as cron.
    // Never these, which saving the form would rewrite.
    for (const expr of [
      "*/15 * * * *",
      "0 9,17 * * *",
      "0 9 1-7 * 1",
      "0 9 * 1 *",
      "0 9 1 * 1",
      "0 9 */2 * *",
      "0 9 */2 * 1",
      "0 9 1 * */2",
      "nonsense",
    ])
      expect(cronRepeat(expr), expr).toBeUndefined();
  });
});

describe("schedules", () => {
  const schedule = (s: Partial<G.Schedule>): Pick<G.Schedule, "cron" | "at" | "timezone" | "event"> => ({
    timezone: "UTC",
    ...s,
  });
  it("say when they run", () => {
    const clock = (h: number, m: number) => `${h}:${String(m).padStart(2, "0")}`;
    expect(scheduleWhen(schedule({ cron: "0 9 * * 1-5" }), clock)).toBe("Every weekday at 9:00");
    expect(scheduleWhen(schedule({}))).toBe("When asked");
    expect(scheduleWhen(schedule({ at: utc(2026, 10, 30, 13, 0), timezone: STOCKHOLM }))).toMatch(
      /^Once, on /,
    );
    expect(scheduleWhen(schedule({ event: { kind: "issue_opened" } }))).toBe("When an issue is opened");
  });

  it("let people do what their role allows", () => {
    expect(scheduleRights("editor", true)).toEqual({ run: true, edit: true, manage: true });
    expect(scheduleRights("editor", false)).toEqual({ run: true, edit: false, manage: false });
    expect(scheduleRights("maintainer", false)).toEqual({ run: true, edit: true, manage: true });
    expect(scheduleRights("viewer", true)).toEqual({ run: false, edit: false, manage: true });
    expect(scheduleRights("viewer", false)).toEqual({ run: false, edit: false, manage: false });
    expect(scheduleRights(undefined, true)).toEqual({ run: false, edit: false, manage: false });
  });

  it("say what came of a run", () => {
    const run = (r: Partial<G.ScheduleRun>): G.ScheduleRun => ({
      id: "srn_1" as G.ScheduleRunId,
      schedule_id: "sch_1" as G.ScheduleId,
      due_at: 0,
      at: 0,
      outcome: "started",
      trigger: "time",
      ...r,
    });
    const ada = { id: "usr_1" as G.UserId, name: "Ada" };
    expect(runOutcome(run({ session: { id: "ses_1" as G.SessionId, number: 41, title: "x" } }))).toBe(
      "Started #41",
    );
    expect(runOutcome(run({ outcome: "skipped" }))).toBe("Skipped");
    expect(runTrigger(run({}))).toBe("On time");
    expect(runTrigger(run({ trigger: "person", by: ada }))).toBe("Ada");
    expect(runTrigger(run({ trigger: "token", by: ada }))).toBe("A tool, with Ada's token");
    const autopilot = { project_id: "prj_1" as G.ProjectId, project_slug: "spaghetti" };
    expect(runTrigger(run({ trigger: "token", by: ada, autopilot }))).toBe("spaghetti's autopilot, for Ada");
    const event: G.ScheduleRunEvent = {
      kind: "issue_opened",
      repo: { owner: "acme", name: "web" },
      number: 41,
      url: "https://github.com/acme/web/issues/41",
      actor: "octocat",
    };
    expect(runTrigger(run({ trigger: "event", by: ada, event }))).toBe("@octocat opened web#41");
    expect(runTrigger(run({ trigger: "event" }))).toBe("On an event");
  });
});

describe("events", () => {
  const event = (e: Partial<G.ScheduleEvent>): G.ScheduleEvent => ({ kind: "issue_opened", ...e });

  it("say what a schedule waits for as the control plane does", () => {
    // fctl_domain::triggers::tests::describes_what_a_schedule_waits_for
    expect(describeEvent(event({}))).toBe("when an issue is opened");
    expect(describeEvent(event({ kind: "issue_labeled", labels: ["agent", "fctl"] }))).toBe(
      "when an issue gets the label agent or fctl",
    );
    expect(describeEvent(event({ kind: "pull_request_opened" }))).toBe(
      "when a pull request is opened, by people who can write to the repository",
    );
    expect(
      describeEvent(
        event({
          kind: "pull_request_merged",
          branches: ["main"],
          repos: ["acme/web", "acme/docs"],
          labels: ["docs"],
          by: "agents",
        }),
      ),
    ).toBe(
      "when a pull request merges into main in acme/web or acme/docs, with the label docs, by fctl's agents",
    );
    expect(describeEvent(event({ kind: "checks_failed" }))).toBe("when checks fail on the default branch");
    expect(describeEvent(event({ kind: "release_published" }))).toBe("when a release is published");
    expect(describeEvent(event({ kind: "other" }))).toBe("when something happens on GitHub");
  });

  it("cap the sessions they start as the control plane does", () => {
    // fctl_domain::triggers::tests::caps_the_sessions_events_start
    expect(eventCaps(event({}))).toEqual({ hour: 3, day: 10 });
    expect(eventCaps(event({ max_per_hour: 20, max_per_day: 200 }))).toEqual({ hour: 20, day: 200 });
    expect(eventCaps(event({ max_per_day: 2 }))).toEqual({ hour: 2, day: 2 });
    expect(eventCaps(event({ max_per_hour: 0, max_per_day: 5000 }))).toEqual({ hour: 1, day: 1000 });
    expect(describeCaps({ hour: 3, day: 10 })).toBe("3 sessions an hour and 10 a day");
    expect(describeCaps({ hour: 1, day: 1000 })).toBe("1 session an hour and 1,000 a day");
    expect(capProblem("3", "hour")).toBeUndefined();
    expect(capProblem(" 1000 ", "day")).toBeUndefined();
    expect(capProblem("", "hour")).toBe("Give a whole number of sessions an hour");
    expect(capProblem("2.5", "day")).toBe("Give a whole number of sessions a day");
    expect(capProblem("0", "hour")).toBe("Let it start at least 1 session an hour: pause it to start none");
    expect(capProblem("1001", "day")).toBe("It starts at most 1,000 sessions a day");
    expect(capProblem("101", "hour")).toBe("It starts at most 100 sessions an hour");
  });

  it("take pull requests from people who may write by default", () => {
    expect(eventBy(event({ kind: "pull_request_opened" }))).toBe("writers");
    expect(eventBy(event({ kind: "pull_request_merged" }))).toBe("anyone");
    expect(eventBy(event({ kind: "pull_request_opened", by: "anyone" }))).toBe("anyone");
    expect(eventBy(event({ by: "other" }))).toBe("anyone");
  });

  it("say what happened", () => {
    const repo = { owner: "acme", name: "web" };
    const url = "https://github.com/acme/web/issues/41";
    expect(eventSaid({ kind: "issue_labeled", repo, url, number: 41, actor: "ada", label: "agent" })).toBe(
      "@ada added the label agent to web#41",
    );
    expect(eventSaid({ kind: "pull_request_merged", repo, url, number: 12, branch: "main" })).toBe(
      "someone merged web#12 into main",
    );
    expect(eventSaid({ kind: "release_published", repo, url, actor: "ada", tag: "v1.2.0" })).toBe(
      "@ada published v1.2.0 of web",
    );
    expect(eventSaid({ kind: "checks_failed", repo, url, branch: "main", sha: "abc" })).toBe(
      "checks failed on main of web",
    );
    expect(orList(["a", "b", "c"])).toBe("a, b or c");
    expect(orList([])).toBe("");
  });
});

describe("a run's first message", () => {
  it("parts the task from fctl's note", () => {
    const note =
      'fctl started this session on Monday 26 October 2026 at 09:00 (Europe/Stockholm) on the project\'s schedule "Weekly", which runs every Monday at 09:00, for Ada.';
    expect(splitRunNote(`Update the dependencies.\n\n---\nKeep it small.\n\n---\n${note}`)).toEqual({
      task: "Update the dependencies.\n\n---\nKeep it small.",
      note,
    });
    const tool =
      "Another tool started this session on Monday 26 October 2026 at 09:00 (UTC) from the project's schedule \"Triage\", which runs once, with an access token of Ada's.\n\n<context>\nx\n</context>";
    expect(splitRunNote(`Triage.\n\n---\n${tool}`).note).toBe(tool);
    // fctl_domain::schedules::tests::a_run_on_an_event_says_what_happened
    const event =
      'fctl started this session on Monday 26 October 2026 at 09:00 (Europe/Stockholm) on the project\'s schedule "Triage new issues", which runs when an issue is opened, for Ada Lovelace: @octocat opened issue acme/web#41. Nobody watches it start.\n\n<context>\nIssue acme/web#41\n</context>';
    expect(splitRunNote(`Triage the issue.\n\n---\n${event}`)).toEqual({
      task: "Triage the issue.",
      note: event,
    });
    expect(splitRunNote("Just a task\n\n---\nwith a rule")).toEqual({
      task: "Just a task\n\n---\nwith a rule",
    });
  });
});

describe("a schedule's name", () => {
  it("comes from its task's first line, cut at a word", () => {
    expect(nameFromTask("## Update the dependencies\nand open a pull request")).toBe(
      "Update the dependencies",
    );
    expect(nameFromTask("  \n- Triage new issues")).toBe("Triage new issues");
    expect(
      nameFromTask(
        "Look through yesterday's failed CI runs on main and file an issue for each flaky test, with the runs that show it.",
      ),
    ).toBe("Look through yesterday's failed CI runs on main and file an issue for…");
    expect(nameFromTask("")).toBe("");
  });
});

describe("running one that waits for events by hand", () => {
  it("reads what to run it on", () => {
    const repo = "acme/web";
    expect(readRunOn("issue_opened", "#41", repo)).toEqual({ repo, number: 41 });
    expect(readRunOn("issue_opened", " 41 ", repo)).toEqual({ repo, number: 41 });
    expect(readRunOn("issue_labeled", "acme/docs#7", repo)).toEqual({ repo: "acme/docs", number: 7 });
    expect(readRunOn("pull_request_merged", "https://github.com/acme/api/pull/128/files", repo)).toEqual({
      repo: "acme/api",
      number: 128,
    });
    expect(readRunOn("pull_request_merged", "https://github.com/acme/api/pull/128", repo)).toEqual({
      repo: "acme/api",
      number: 128,
    });
    expect(readRunOn("issue_opened", "github.com/acme/web/issues/41#issuecomment-1", repo)).toEqual({
      repo,
      number: 41,
    });
    expect(readRunOn("issue_opened", "the login bug", repo)).toBeUndefined();
    expect(readRunOn("release_published", "v1.2.0", repo)).toEqual({ repo, tag: "v1.2.0" });
    expect(readRunOn("release_published", "https://github.com/acme/cli/releases/tag/v2.0.0", repo)).toEqual({
      repo: "acme/cli",
      tag: "v2.0.0",
    });
    // A link whose escape isn't one reads as it is, rather than throw as the dialog renders.
    expect(readRunOn("release_published", "github.com/acme/cli/releases/tag/v1%", repo)).toEqual({
      repo: "acme/cli",
      tag: "v1%",
    });
    expect(readRunOn("checks_failed", "main", repo)).toEqual({ repo, branch: "main" });
    expect(readRunOn("checks_failed", "no spaces", repo)).toBeUndefined();
  });

  it("tells the agent what it runs on, and where to read it", () => {
    expect(runOnContext("issue_labeled", { repo: "acme/web", number: 41 }, "fctl")).toBe(
      "Run it on issue acme/web#41 (https://github.com/acme/web/issues/41), as if it had just got the label fctl: `gh issue view 41 -R acme/web` shows it.",
    );
    expect(runOnContext("pull_request_merged", { repo: "acme/web", number: 12 })).toContain(
      "as if it had just merged: `gh pr view 12 -R acme/web` shows it.",
    );
    expect(runOnContext("checks_failed", { repo: "acme/web", branch: "main" })).toContain(
      "`gh run list -R acme/web --branch main`",
    );
  });
});

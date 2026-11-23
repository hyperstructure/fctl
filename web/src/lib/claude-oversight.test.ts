import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  adminTokenCommand,
  detail,
  facts,
  factsText,
  matches,
  nobody,
  pills,
  runsOutSoon,
  summary,
  whatAdminsSee,
  yourClaude,
} from "./claude-oversight";

const HOUR = 3_600_000;
const DAY = 24 * HOUR;
/** Monday November 23, 2026, 12:00 UTC. */
const NOW = Date.UTC(2026, 10, 23, 12);

const person = (name: string, m: Partial<G.MemberClaude> = {}): G.MemberClaude => ({
  user: { id: `usr_${name.split(" ")[0]?.toLowerCase()}`, name },
  email: `${name.split(" ")[0]?.toLowerCase()}@acme.dev`,
  state: "works",
  ...m,
});

const grace = person("Grace Hopper", {
  runs_out_at: NOW + 21 * DAY,
  runs_out_estimated: true,
  paused_until: undefined,
  connection: {
    kind: "token",
    set_by_admin_token: true,
    plan: "team",
    connected_at: NOW - 344 * DAY,
    answered_at: NOW - 2 * HOUR,
  },
  pauses: { week: 6, month: 9, last_at: NOW - DAY, last_limit: "seven_day" },
});
const ada = person("Ada Lovelace", {
  connection: { kind: "login", plan: "max", connected_at: NOW - 30 * DAY, answered_at: NOW - 5 * 60_000 },
  pauses: { week: 0, month: 0 },
});
const linus = person("Linus Lee", { state: "none" });
const margaret = person("Margaret Hamilton", {
  state: "ended",
  ended_at: NOW - 3 * DAY,
  connection: { kind: "token", connected_at: NOW - 200 * DAY },
});

describe("the facts line under each person", () => {
  it("says what their sessions run on, its plan, Claude's last answer and when it runs out", () => {
    expect(factsText(grace, "Acme", NOW)).toBe(
      "Long-lived token, set by an admin token · Team · Claude answered 2 hours ago · runs out around Dec 14 · 6 limit pauses this week",
    );
    expect(factsText(ada, "Acme", NOW)).toBe("Claude login · Max · Claude answered 5 minutes ago");
  });

  it("marks a date within a month in amber, and many pauses too", () => {
    const tones = facts(grace, "Acme", NOW).map((f) => f.tone);
    expect(tones).toEqual(["plain", "plain", "plain", "warn", "warn"]);
    const later = { ...grace, runs_out_at: NOW + 90 * DAY, pauses: { week: 1, month: 1 } };
    expect(facts(later, "Acme", NOW).every((f) => f.tone === "plain")).toBe(true);
  });

  it("says plan unknown for a token on their runners, and when it was used before Claude's answers were kept", () => {
    const linusOnRunners = person("Linus Lee", {
      connection: { kind: "on_runners", connected_at: NOW - 9 * DAY, used_at: NOW - DAY },
    });
    expect(factsText(linusOnRunners, "Acme", NOW)).toBe(
      "Token on their runners · plan unknown · used yesterday",
    );
  });

  it("says in amber who has none, and in red whose connection Claude stopped taking", () => {
    expect(facts(linus, "Acme", NOW)).toEqual([
      { text: "No Claude connection for Acme: their sessions here can't start", tone: "warn" },
    ]);
    expect(facts(margaret, "Acme", NOW)).toEqual([
      { text: "Claude refused their token on Nov 20: their sessions here can't run", tone: "danger" },
    ]);
  });

  it("says only whether sessions can run, and when that ends, where admins see no more", () => {
    const ready = { ...grace, connection: undefined, pauses: undefined };
    expect(factsText(ready, "Acme", NOW)).toBe("Claude takes their connection · runs out around Dec 14");
    const stopped = { ...margaret, connection: undefined };
    expect(factsText(stopped, "Acme", NOW)).toBe(
      "Claude stopped taking their connection on Nov 20: their sessions here can't run",
    );
  });

  it("says who waits for a limit to reset now", () => {
    const waiting = { ...ada, paused_until: NOW + 2 * HOUR, paused_limit: "five_hour" };
    expect(factsText(waiting, "Acme", NOW)).toContain("waiting for the 5-hour limit to reset at");
  });
});

describe("the summary and the pills", () => {
  const everyone = [grace, ada, linus, margaret];

  it("counts who can run sessions, whose token runs out within a month and whose stopped", () => {
    expect(summary(everyone, "Acme", NOW)).toBe(
      "2 of 4 people have Claude connected for Acme · 1 token runs out within a month · 1 stopped working",
    );
    expect(summary([ada], "Acme", NOW)).toBe("Its one person has Claude connected for Acme");
    expect(summary([linus, ada], "Acme", NOW)).toBe("1 of 2 people has Claude connected for Acme");
  });

  it("leaves out the pills nobody is under, unless chosen", () => {
    const p = pills(everyone, { limit_pauses: true }, "everyone", NOW);
    expect(p.map((x) => `${x.label} ${x.count}`)).toEqual([
      "Everyone 4",
      "No Claude 1",
      "Runs out soon 1",
      "Stopped working 1",
      "Pausing on limits 1",
    ]);
    const healthy = pills([ada], { limit_pauses: true }, "soon", NOW);
    expect(healthy.map((x) => `${x.label} ${x.count}`)).toEqual(["Everyone 1", "Runs out soon 0"]);
    expect(nobody("soon", "Acme")).toBe("Nobody's token runs out within a month.");
  });

  it("narrows to each pill's people", () => {
    expect(everyone.filter((m) => matches(m, "none", NOW)).map((m) => m.user.name)).toEqual(["Linus Lee"]);
    expect(everyone.filter((m) => matches(m, "ended", NOW)).map((m) => m.user.name)).toEqual([
      "Margaret Hamilton",
    ]);
    expect(runsOutSoon(grace, NOW)).toBe(true);
    // A token that stopped working doesn't also run out soon.
    expect(runsOutSoon({ ...margaret, runs_out_at: NOW + DAY }, NOW)).toBe(false);
  });
});

describe("a person's connection, opened", () => {
  it("says it whole: what, since when, the plan, the last answer, why the date is around, and the pauses", () => {
    expect(detail(grace, "Acme", true, NOW)).toEqual([
      "Grace Hopper's sessions in Acme run on a long-lived token an admin token set on Dec 14, 2025, on a Team plan.",
      "Claude last answered it 2 hours ago.",
      "It runs out around Dec 14, 2026, a year after it was set: fctl can't read the date from the token.",
      "Their sessions paused on usage limits 6 times this week and 9 times in 30 days, the last on the weekly limit.",
      "Their plan's limits are short for this work: a Max or Ultra seat has more.",
    ]);
  });

  it("leaves the pauses out where admins don't see them, and says a login doesn't run out", () => {
    expect(detail(ada, "Acme", false, NOW)).toEqual([
      "Ada Lovelace's sessions in Acme run on a Claude login they connected on Oct 24, 2026, on a Max plan.",
      "Claude last answered it 5 minutes ago.",
      "Claude renews the login: it runs until Claude ends it.",
    ]);
  });

  it("says what stopped and what it means", () => {
    expect(detail(margaret, "Acme", true, NOW)).toEqual([
      "On Nov 20, 2026, Claude refused the long-lived token Margaret Hamilton's sessions in Acme ran on: it was revoked, or it ran out. Their sessions here can't run until they have another.",
    ]);
    expect(detail(linus, "Acme", true, NOW)).toEqual([
      "Linus Lee has no Claude connection for Acme: their sessions here can't start until they connect one.",
    ]);
  });

  it("gives admins the command that sets a new token, for this workspace's seat", () => {
    expect(adminTokenCommand("grace@acme.dev", "Acme")).toBe(
      'fctl admin users set grace@acme.dev --claude-token - --claude-label "Acme seat"',
    );
  });
});

describe("what members read of what their admins see", () => {
  it("says it per level, and never the token or the account", () => {
    expect(whatAdminsSee("Acme", "details", true)).toBe(
      "Acme's admins see what kind of Claude connection your sessions here run on, its plan, when Claude last answered it and when it runs out, and how often your sessions paused on usage limits: never the token or the account.",
    );
    expect(whatAdminsSee("Acme", "readiness", false)).toContain(
      "whether your sessions here can run on Claude",
    );
    expect(whatAdminsSee("Acme", "off", false)).toBe("Acme's admins see nothing of your Claude connection.");
  });
});

describe("your own connections", () => {
  const token = (c: Partial<G.ClaudeCredentialInfo>): G.ClaudeCredentialInfo => ({
    id: "crd_1",
    kind: "setup_token",
    remote_control: false,
    status: "active",
    is_default: true,
    created_at: NOW - 360 * DAY,
    ...c,
  });

  it("finds a token in its last week, and says when every connection ended", () => {
    expect(yourClaude([token({ expires_at: NOW + 5 * DAY })], NOW).ending?.id).toBe("crd_1");
    expect(yourClaude([token({ expires_at: NOW + 20 * DAY })], NOW).ending).toBeUndefined();
    // A token on runners has no date fctl knows.
    expect(yourClaude([token({ expires_at: NOW + 5 * DAY, on_runners: true })], NOW).ending).toBeUndefined();
    expect(yourClaude([token({ status: "invalid", ended_at: NOW - DAY })], NOW).ended?.id).toBe("crd_1");
    expect(
      yourClaude([token({ status: "invalid" }), token({ id: "crd_2", kind: "oauth" })], NOW).ended,
    ).toBeUndefined();
  });
});

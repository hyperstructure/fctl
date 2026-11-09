import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  atWork,
  checkInWords,
  DIRECTIONS,
  defaultPowers,
  hasPlaceholders,
  phase,
  splitBriefing,
  splitCrewNote,
  wakeText,
  wokeLine,
} from "./autopilot";

const NOW = Date.UTC(2026, 10, 9, 14, 0);

function autopilot(more: Partial<G.Autopilot> = {}): G.Autopilot {
  return {
    project_id: "prj_shop",
    project_slug: "shop",
    scope: "project",
    state: "on",
    owner: { id: "usr_noor", name: "Noor Haddad" },
    direction: "Ship the checkout.",
    direction_at: NOW - 86_400_000,
    powers: ["settings", "schedules"],
    check_in_minutes: 60,
    crew_max: 3,
    wake_at: NOW + 25 * 60_000,
    wake_reason: "check_in",
    check_ins: 4,
    notes: [],
    asks: [],
    crew: [],
    crew_total: 0,
    can: { steer: true, secrets: true },
    created_at: NOW - 86_400_000,
    updated_at: NOW,
    ...more,
  };
}

const ask = (status: G.AutopilotAskStatus): G.AutopilotAsk => ({
  id: `apa_${status}`,
  kind: "question",
  text: "Which provider?",
  status,
  created_at: NOW,
});

describe("where an autopilot stands", () => {
  it("is at work while its own session works", () => {
    const a = autopilot({ session: { id: "ses_1", number: 7, status: "running", started_at: NOW } });
    expect(atWork(a)).toBe(true);
    expect(phase(a, NOW)).toMatchObject({ tone: "accent", word: "At work" });
    expect(phase(a, NOW).line).toBe("It works in its session #7 now.");
  });

  it("wakes, and starts, in its session", () => {
    const a = autopilot({ session: { id: "ses_1", number: 7, status: "provisioning", started_at: NOW } });
    expect(phase(a, NOW)).toMatchObject({ word: "Waking", line: "Its session #7 is starting." });
  });

  it("waits for people while it asks, and goes on meanwhile", () => {
    const a = autopilot({ asks: [ask("open"), ask("answered")] });
    const p = phase(a, NOW);
    expect(p).toMatchObject({ tone: "blue", word: "Waiting for you" });
    expect(p.line).toMatch(
      /^It asks you one thing, and goes on with what it can meanwhile\. It checks in at /,
    );
  });

  it("says when it checks in, or wakes as it asked", () => {
    expect(phase(autopilot(), NOW).line).toMatch(/^It checks in at .*, unless something needs it sooner\.$/);
    expect(phase(autopilot({ wake_reason: "asked" }), NOW).line).toMatch(/^It wakes at .*, as it asked/);
    expect(phase(autopilot({ wake_at: NOW + 10_000, wake_reason: "crew" }), NOW).line).toBe(
      "It wakes now for what's new.",
    );
  });

  it("says who paused it, and why", () => {
    const a = autopilot({
      state: "paused",
      paused_by: { id: "usr_noor", name: "Noor Haddad" },
      paused_at: NOW - 120_000,
      paused_reason: "Launch week",
    });
    expect(phase(a, NOW)).toMatchObject({ tone: "pink", word: "Paused" });
    expect(phase(a, NOW).line).toBe(
      "Noor Haddad paused it 2 minutes ago: Launch week It wakes again once someone resumes it.",
    );
  });
});

describe("what woke it", () => {
  const wake = (more: Partial<G.AutopilotWake>): G.AutopilotWake => ({
    reasons: [],
    notes: 0,
    answers: 0,
    direction: false,
    ...more,
  });

  it("names notes, answers and sessions", () => {
    expect(wakeText(wake({ reasons: ["check_in"] }))).toBe("its check-in");
    expect(wakeText(wake({ reasons: ["resumed", "steered"], notes: 1 }))).toBe(
      "being resumed and a new note",
    );
    expect(wakeText(wake({ reasons: ["steered"], notes: 2 }))).toBe("2 new notes");
    expect(wakeText(wake({ reasons: ["steered"], direction: true }))).toBe("a new direction");
    expect(
      wakeText(
        wake({
          reasons: ["answered", "crew"],
          answers: 1,
          crew: [{ id: "ses_9", number: 41, title: "Board" }],
        }),
      ),
    ).toBe("an answer and session #41");
    expect(wakeText(wake({}))).toBe("its check-in");
  });

  it("says a start or a resume alone as its log does", () => {
    expect(wokeLine(wake({ reasons: ["resumed"] }))).toBe("Woke as it was resumed");
    expect(wokeLine(wake({ reasons: ["started"] }))).toBe("Woke as it was put on");
    expect(wokeLine(wake({ reasons: ["resumed", "steered"], notes: 2 }))).toBe(
      "Woke for being resumed and 2 new notes",
    );
    expect(wokeLine(wake({ reasons: ["check_in"] }))).toBe("Woke for its check-in");
    expect(wokeLine(wake({ reasons: ["woken"] }))).toBe("Woke as someone woke it");
  });
});

describe("an autopilot's messages", () => {
  it("sets its briefing's direction apart from what woke it", () => {
    const text =
      "You are the autopilot of the project Shop (shop): an agent…\n\n## Where to take it\n\nShip the checkout.\n\n## How you work\n\n- …\n---\n\nfctl woke you, the autopilot: you were put on.";
    const b = splitBriefing(text);
    expect(b.direction).toBe("Ship the checkout.");
    expect(b.news).toBe("fctl woke you, the autopilot: you were put on.");
    expect(
      splitBriefing("fctl woke you, the autopilot: it's time for your check-in.").briefing,
    ).toBeUndefined();
  });

  it("keeps fctl's note apart from a task it gave a session", () => {
    const task = "Build the board.\n\n---\nThe autopilot of Shop started this session: an agent that…";
    expect(splitCrewNote(task)).toEqual({
      task: "Build the board.",
      note: "The autopilot of Shop started this session: an agent that…",
    });
    expect(splitCrewNote("Build it.\n\n---\nSomething else")).toEqual({
      task: "Build it.\n\n---\nSomething else",
    });
  });
});

describe("how people set it up", () => {
  it("offers a workspace's autopilot projects too", () => {
    expect(defaultPowers("project")).toEqual(["settings", "secrets", "repos", "schedules"]);
    expect(defaultPowers("workspace")).toContain("projects");
  });

  it("says check-ins in words", () => {
    expect(checkInWords(60)).toBe("every hour");
    expect(checkInWords(1440)).toBe("once a day");
    expect(checkInWords(90)).toBe("every 90 minutes");
  });

  it("knows a direction still to fill in", () => {
    const build = DIRECTIONS.find((d) => d.key === "build");
    expect(build && hasPlaceholders(build.direction)).toBe(true);
    expect(hasPlaceholders("Ship [x]")).toBe(false);
    expect(hasPlaceholders("Ship the checkout.")).toBe(false);
  });
});

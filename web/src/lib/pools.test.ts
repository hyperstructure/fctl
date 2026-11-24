import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  defaultPlacement,
  fallbackOf,
  labelProblem,
  labelsWords,
  parseLabels,
  placementWords,
  poolChoices,
  poolFacts,
  poolFor,
  poolLine,
  poolShapeWords,
  poolState,
  runnersGist,
  sameLabels,
  waited,
} from "./pools";

const AT = 1_795_000_000_000;

const demand = (d: {
  runners?: Partial<G.PoolRunners>;
  slots?: Partial<G.PoolSlots>;
  waiting?: Partial<G.PoolWaiting>;
  pool?: Partial<G.PoolInfo>;
  wanted?: number;
  shape?: Partial<G.PoolShape>;
}): G.PoolDemand => ({
  pool: { id: "enr_1", name: "AWS large", labels: ["aws", "large"], placement: "packed", ...d.pool },
  runners: { online: 0, busy: 0, draining: 0, pending: 0, starting: 0, offline: 0, ...d.runners },
  slots: { capacity: 0, busy: 0, free: 0, ...d.slots },
  waiting: { sessions: 0, ...d.waiting },
  wanted: d.wanted ?? 0,
  shape: d.shape
    ? {
        capacity: 4,
        cpus: 16,
        memory_mb: 65536,
        driver: "docker",
        version: "0.38.0",
        seen_at: AT,
        ...d.shape,
      }
    : undefined,
  at: AT,
});

describe("labels", () => {
  it("are read as typed, and checked as the server does", () => {
    expect(parseLabels(" AWS, large  gpu,aws ")).toEqual(["aws", "large", "gpu"]);
    expect(parseLabels("")).toEqual([]);
    expect(labelProblem(["aws", "eu-north-1", "zone:a", "v1.2"])).toBeUndefined();
    expect(labelProblem(["-aws"])).toContain('"-aws" isn\'t a label');
    expect(labelProblem(["a/b"])).toContain("isn't a label");
    expect(labelProblem(Array.from({ length: 17 }, (_, i) => `l${i}`))).toBe("Name at most 16 labels");
    expect(labelsWords([])).toBe("no labels");
    expect(labelsWords(["gpu"])).toBe("the label gpu");
    expect(labelsWords(["aws", "gpu", "large"])).toBe("the labels aws, gpu and large");
  });
});

describe("a pool's line", () => {
  it("says its labels and its placement, as the CLI does", () => {
    expect(poolLine(["aws", "large"], "packed")).toBe(
      "Its runners carry the labels aws and large. Packed: fills one runner before the next, so that idle ones can go.",
    );
    expect(poolLine([], undefined)).toBe(
      "No labels: projects that ask for none run on its runners. Spread: each session goes to the least busy runner.",
    );
  });
});

describe("placement", () => {
  it("says what it does, and packs a pool that removes offline runners", () => {
    expect(placementWords("packed")).toBe(
      "Packed: fills one runner before the next, so that idle ones can go",
    );
    expect(placementWords(undefined)).toBe("Spread: each session goes to the least busy runner");
    expect(defaultPlacement(5)).toBe("packed");
    expect(defaultPlacement(undefined)).toBe("spread");
  });
});

describe("a pool's card", () => {
  it("counts its runners, its sessions and what waits", () => {
    const facts = poolFacts(
      demand({
        runners: { online: 8, busy: 6, starting: 1, pending: 1 },
        slots: { capacity: 32, busy: 11, free: 21 },
        waiting: { sessions: 3, since: AT - 125_000 },
      }),
    ).map((f) => f.text);
    expect(facts).toEqual([
      "6 of 8 runners busy",
      "1 starting",
      "1 waiting for approval",
      "11 of 32 sessions",
      "3 sessions waiting",
      "longest 2m",
    ]);
    expect(poolFacts(demand({ pool: { max_runners: 20 }, runners: { offline: 2 } }))[0]?.text).toBe(
      "2 of 20 runners",
    );
  });

  it("says what the pool is doing, amber where someone should look", () => {
    const waiting = { sessions: 3, since: AT - 120_000 };
    expect(
      poolState(demand({ runners: { online: 20, busy: 20 }, pool: { max_runners: 20 }, waiting })),
    ).toEqual({
      text: "At its limit of 20 runners: 3 sessions wait for room.",
      tone: "amber",
    });
    expect(poolState(demand({ runners: { online: 2, busy: 2, starting: 1 }, waiting }))).toEqual({
      text: "Growing: 1 runner is starting for 3 sessions that wait.",
      tone: "accent",
    });
    expect(poolState(demand({ runners: { online: 2, busy: 2 }, waiting, wanted: 3 }))).toEqual({
      text: "3 sessions wait, the longest for 2m: it wants 3 runners, and has 2 online.",
      tone: "amber",
    });
    expect(poolState(demand({})).text).toBe("No runners yet: machines that enroll with its token show here.");
    expect(poolState(demand({ runners: { offline: 2 } })).text).toBe(
      "No runner online, and no session waits for it.",
    );
    expect(poolState(demand({ runners: { online: 2 } })).text).toBe("Idle: its 2 runners run nothing.");
    expect(poolState(demand({ runners: { online: 4, busy: 1 }, wanted: 1 })).text).toBe(
      "Could shrink: it wants 1 runner of its 4.",
    );
    expect(poolState(demand({ runners: { online: 4, busy: 4 }, wanted: 4 })).text).toBe(
      "Busy: 4 of 4 runners run sessions.",
    );
    expect(poolState(demand({ runners: { online: 2, busy: 1 }, wanted: 2 })).text).toBe(
      "Busy: 1 of 2 runners runs sessions.",
    );
    expect(poolState(demand({ pool: { closed: true } })).text).toBe("Its token enrolls no more runners.");
  });

  it("says how long in the CLI's words", () => {
    expect([
      waited(40_000),
      waited(125_000),
      waited(3_900_000),
      waited(2 * 86_400_000 + 3 * 3_600_000),
    ]).toEqual(["40s", "2m", "1h 5m", "2d 3h"]);
  });
});

describe("choosing a pool", () => {
  it("describes one by what its runners give", () => {
    expect(poolShapeWords(demand({ runners: { online: 4 }, shape: {} }))).toBe(
      "4 runners with 16 CPUs and 64 GiB",
    );
    expect(poolShapeWords(demand({}))).toBe("no runners yet");
    expect(poolShapeWords(demand({ shape: {} }))).toBe("no runners now, 16 CPUs and 64 GiB each");
  });

  it("finds the pool a project's labels wait for: the one with the fewest others", () => {
    const large = demand({ pool: { id: "enr_l", labels: ["aws", "large", "gpu"] } });
    const small = demand({ pool: { id: "enr_s", labels: ["aws", "large"] } });
    const tagged = demand({ pool: { id: "enr_t", labels: ["aws"] }, shape: { labels: ["large"] } });
    expect(poolFor(["aws", "large"], [large, small])?.pool.id).toBe("enr_s");
    expect(poolFor(["large"], [tagged])?.pool.id).toBe("enr_t");
    expect(poolFor(["arm"], [large, small])).toBeUndefined();
    expect(poolFor([], [large])).toBeUndefined();
    // A member's own pool runs only their sessions: no project's waits for it.
    const own = demand({
      pool: { id: "enr_o", labels: ["aws", "large"], owner: { id: "usr_grace", name: "Grace" } },
    });
    expect(poolFor(["aws", "large"], [own])).toBeUndefined();
    expect(poolFor(["aws", "large"], [own, large])?.pool.id).toBe("enr_l");
    // Nor does one whose token ended with no runners left; one with runners still counts.
    const ended = demand({ pool: { id: "enr_e", labels: ["aws", "large"], closed: true } });
    expect(poolFor(["aws", "large"], [ended])).toBeUndefined();
    expect(poolFor(["aws", "large"], [demand({ ...ended, runners: { online: 1 } })])?.pool.id).toBe("enr_e");
  });
});

describe("a project's choice of runners", () => {
  const runner = (id: string, labels: string[], pool?: G.RunnerPool, personal: string[] = []) =>
    ({ id, name: id, labels, pool, personal, status: "online" }) as G.Runner;

  it("lists the pools the viewer manages, and the others through their runners", () => {
    const managed = demand({ pool: { id: "enr_l", name: "AWS large" }, runners: { online: 4 }, shape: {} });
    // A member's own pool, and a runner of one in this workspace, run only their sessions.
    const own = demand({ pool: { id: "enr_o", name: "Grace's", owner: { id: "usr_grace", name: "Grace" } } });
    const choices = poolChoices(
      [managed, own],
      [
        runner("a", ["aws", "large"], { id: "enr_l", name: "AWS large" }),
        runner("g1", ["gpu", "eu"], { id: "enr_g", name: "GPUs" }),
        runner("g2", ["gpu"], { id: "enr_g", name: "GPUs" }),
        runner("mine", ["arm"], { id: "enr_m", name: "Ada's" }, ["wsp_acme"]),
        runner("mac", []),
      ],
      "wsp_acme",
    );
    expect(choices).toEqual([
      {
        id: "enr_l",
        name: "AWS large",
        labels: ["aws", "large"],
        facts: "4 runners with 16 CPUs and 64 GiB",
      },
      { id: "enr_g", name: "GPUs", labels: ["gpu"], facts: "2 runners" },
    ]);
  });

  it("says the wait it chose, the project's own too, and the runners in a few words", () => {
    expect(fallbackOf(null).label).toBe("Wait for one");
    expect(fallbackOf(15).label).toBe("After 15 minutes, any runner");
    expect(fallbackOf(30)).toMatchObject({ value: "30", label: "After 30 minutes, any runner" });
    expect(runnersGist({})).toBe("Any runner");
    expect(runnersGist({ runner_labels: ["aws", "large"] })).toBe("Runners with aws, large");
    expect(sameLabels(["a", "b"], ["b", "a"])).toBe(true);
    expect(sameLabels(["a"], ["a", "b"])).toBe(false);
  });
});

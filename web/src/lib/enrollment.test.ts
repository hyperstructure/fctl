import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  describeEnrollment,
  enrollCommand,
  enrolledFrom,
  enrollmentReady,
  enrollmentRequest,
  limitOf,
  machineFacts,
  minutesText,
  NEW_ENROLLMENT,
  tokenState,
  waitingByWorkspace,
  whoseToken,
} from "./enrollment";

const token = (t: Partial<G.EnrollmentToken> = {}): G.EnrollmentToken => ({
  id: "enr_1",
  name: "Build fleet",
  personal: false,
  approval: false,
  created_at: 0,
  runners: 0,
  pending: 0,
  ...t,
});

describe("enrollmentRequest", () => {
  it("asks for what the form says, and leaves out what it doesn't limit", () => {
    expect(enrollmentRequest({ ...NEW_ENROLLMENT, name: "  Build fleet " }, true)).toEqual({
      name: "Build fleet",
      personal: false,
      approval: false,
      expires_in_hours: 90 * 24,
    });
    expect(
      enrollmentRequest(
        { name: "CI", personal: true, expires: "never", limit: "20", approval: true, offline: "1h" },
        true,
      ),
    ).toEqual({
      name: "CI",
      personal: true,
      approval: true,
      max_runners: 20,
      remove_offline_after_minutes: 60,
    });
  });

  it("makes a member's for their own sessions, whatever the form held", () => {
    expect(enrollmentRequest({ ...NEW_ENROLLMENT, name: "My cluster" }, false).personal).toBe(true);
  });

  it("takes a limit of whole runners within bounds, or none", () => {
    expect(limitOf("")).toBeUndefined();
    expect(limitOf(" 3 ")).toBe(3);
    expect(limitOf("0")).toBeNaN();
    expect(limitOf("2.5")).toBeNaN();
    expect(limitOf("1001")).toBeNaN();
    expect(enrollmentReady({ ...NEW_ENROLLMENT, name: "CI", limit: "many" })).toBe(false);
    expect(enrollmentReady({ ...NEW_ENROLLMENT, name: " " })).toBe(false);
    expect(enrollmentReady({ ...NEW_ENROLLMENT, name: "CI" })).toBe(true);
  });
});

describe("describeEnrollment", () => {
  // As `fctl_domain::enrollment::describe` says it.
  it("says what happens to the runners a token enrolls", () => {
    const everyone = { everyone: true } as const;
    expect(describeEnrollment(everyone, false)).toBe(
      "Enrolls any number of runners, which run everyone's sessions at once.",
    );
    expect(describeEnrollment(everyone, true, 20, 5)).toBe(
      "Enrolls up to 20 runners at a time, which wait for an admin's approval and are removed once they have been offline for 5 minutes.",
    );
    expect(describeEnrollment(everyone, false, 1, 1440)).toBe(
      "Enrolls one runner at a time, which runs everyone's sessions at once and is removed once it has been offline for a day.",
    );
    expect(describeEnrollment(everyone, true, 1)).toBe(
      "Enrolls one runner at a time, which waits for an admin's approval before it runs anything.",
    );
  });

  it("says whose sessions a member's token runs, as you or others read it", () => {
    const ada = { id: "usr_ada", name: "Ada" };
    const mine = whoseToken({ personal: true, created_by: ada }, "usr_ada");
    expect(describeEnrollment(mine, false, 4, 5)).toBe(
      "Enrolls up to 4 runners at a time, which run your sessions at once and are removed once they have been offline for 5 minutes.",
    );
    expect(describeEnrollment(whoseToken({ personal: true, created_by: ada }, "usr_bo"), true)).toBe(
      "Enrolls any number of runners, which wait for Ada's approval before they run anything.",
    );
    // An admin's for everyone's sessions is the workspace's, whoever reads it.
    expect(whoseToken({ personal: false, created_by: ada }, "usr_ada")).toEqual({ everyone: true });
  });

  it("names durations as people say them", () => {
    expect([1, 15, 60, 360, 1440, 4320, 10080].map(minutesText)).toEqual([
      "a minute",
      "15 minutes",
      "an hour",
      "6 hours",
      "a day",
      "3 days",
      "a week",
    ]);
  });
});

describe("tokenState", () => {
  it("says why a token enrolls nothing now", () => {
    expect(tokenState(token(), 10)).toBe("active");
    expect(tokenState(token({ revoked_at: 5, expires_at: 1 }), 10)).toBe("revoked");
    expect(tokenState(token({ expires_at: 10 }), 10)).toBe("expired");
    expect(tokenState(token({ max_runners: 2, runners: 2 }), 10)).toBe("full");
    expect(tokenState(token({ max_runners: 2, runners: 1 }), 10)).toBe("active");
  });
});

describe("enrollCommand", () => {
  it("enrolls without a sign-in", () => {
    expect(enrollCommand("https://app.fctl.sh", "fctl_et_x")).toBe(
      "FCTL_SERVER=https://app.fctl.sh \\\nFCTL_ENROLL_TOKEN=fctl_et_x \\\nfctl host install",
    );
  });
});

const runner = (r: Partial<G.Runner> & { id: string }): G.Runner =>
  ({
    name: r.id,
    status: "online",
    drivers: [],
    capacity: 4,
    active_sessions: 0,
    labels: [],
    created_at: 0,
    ...r,
  }) as G.Runner;

describe("what a waiting runner shows", () => {
  it("says what it is and where it enrolled from", () => {
    expect(
      machineFacts(
        runner({
          id: "rnr_1",
          name: "build-7",
          hostname: "ip-10-0-3-17",
          os: "linux",
          arch: "x86_64",
          driver: "docker",
          cpus: 8,
          memory_mb: 16384,
          version: "0.28.0",
        }),
      ),
    ).toEqual(["ip-10-0-3-17", "linux/x86_64", "docker", "8 CPUs, 16 GiB", "fctl 0.28.0"]);
    // Its hostname is its name: once is enough.
    expect(machineFacts(runner({ id: "rnr_2", name: "mini", hostname: "mini" }))).toEqual([]);
    expect(enrolledFrom({ address: "203.0.113.7", location: "Stockholm, SE" })).toBe(
      "203.0.113.7 (Stockholm, SE)",
    );
    expect(enrolledFrom({ address: "203.0.113.7" })).toBe("203.0.113.7");
    expect(enrolledFrom({})).toBeUndefined();
  });

  it("goes to the admins of its workspace that it is open to", () => {
    const ws = (id: string, role: G.WorkspaceRole, blocked = false) => ({ id, role, blocked }) as G.Workspace;
    const acme = { id: "wsp_acme" } as G.WorkspaceRef;
    const side = { id: "wsp_side" } as G.WorkspaceRef;
    const runners = [
      runner({ id: "rnr_1", workspace: acme, pending_until: 9 }),
      runner({ id: "rnr_2", workspace: acme, pending_until: 9 }),
      runner({ id: "rnr_3", workspace: acme }),
      runner({ id: "rnr_4", workspace: side, pending_until: 9 }),
      runner({ id: "rnr_5", pending_until: 9 }),
    ];
    const waiting = waitingByWorkspace(runners, [ws("wsp_acme", "admin"), ws("wsp_side", "member")]);
    expect(waiting.map((w) => [w.workspace.id, w.runners.map((r) => r.id)])).toEqual([
      ["wsp_acme", ["rnr_1", "rnr_2"]],
    ]);
    // Instance admins see every runner, but approve only in their workspaces, as their admins.
    expect(waitingByWorkspace(runners, [ws("wsp_acme", "owner", true)])).toEqual([]);
  });

  it("goes to its owner when it enrolled for their own sessions", () => {
    const ws = (id: string, role: G.WorkspaceRole) => ({ id, role }) as G.Workspace;
    const side = { id: "wsp_side" } as G.WorkspaceRef;
    const ada = { id: "usr_ada", name: "Ada" };
    const runners = [
      runner({ id: "rnr_mine", workspace: side, created_by: ada, yours: true, pending_until: 9 }),
      // Someone else's, which an instance admin sees: theirs to approve, not the workspace's admins'.
      runner({ id: "rnr_theirs", workspace: side, created_by: ada, yours: false, pending_until: 9 }),
    ];
    expect(
      waitingByWorkspace(runners, [ws("wsp_side", "member")]).map((w) => w.runners.map((r) => r.id)),
    ).toEqual([["rnr_mine"]]);
    expect(
      waitingByWorkspace(runners, [ws("wsp_side", "admin")]).map((w) => w.runners.map((r) => r.id)),
    ).toEqual([["rnr_mine"]]);
  });
});

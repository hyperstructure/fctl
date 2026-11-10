import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { takeOverOf, workspaceConnection } from "./subscription";

const ada: G.UserRef = { id: "usr_ada", name: "Ada" };
const bo: G.UserRef = { id: "usr_bo", name: "Bo" };

const connection = (id: string, status: G.CredentialStatus = "active", created_at = 0, is_default = false) =>
  ({ id, kind: "oauth", status, created_at, is_default }) as unknown as G.ClaudeCredentialInfo;

const me = (user: G.UserRef, claude = [connection("cred_1")]) =>
  ({ user: { ...user }, claude }) as unknown as Pick<G.Me, "user" | "claude">;

const session = (more: Partial<G.Session> = {}) =>
  ({ id: "ses_1", status: "paused", created_by: ada, ...more }) as G.Session;

describe("takeOverOf", () => {
  it("offers someone else's session that waits for its limit", () => {
    expect(takeOverOf(session(), "editor", me(bo))).toEqual({ whose: "Ada", waits: true });
  });

  it("follows whose subscription it runs on now, not who started it", () => {
    expect(takeOverOf(session({ claude: bo }), "owner", me(bo))).toBeUndefined();
    expect(takeOverOf(session({ claude: bo }), "owner", me(ada))).toEqual({ whose: "Bo", waits: true });
  });

  it("offers your own session only to move it to the connection its workspace uses now", () => {
    const two = [connection("cred_1", "active", 1, true), connection("cred_2", "active", 2)];
    // Its workspace uses cred_2, and it runs on cred_1.
    expect(takeOverOf(session(), "owner", me(ada, two), { mine: "cred_1", chosen: "cred_2" })).toEqual({
      waits: true,
    });
    // It runs on the one its workspace uses already: your default, as none was chosen for it.
    expect(takeOverOf(session(), "owner", me(ada, two), { mine: "cred_1" })).toBeUndefined();
    expect(
      takeOverOf(session(), "owner", me(ada, two), { mine: "cred_2", chosen: "cred_2" }),
    ).toBeUndefined();
    // Which one it runs on is unknown: nothing to offer.
    expect(takeOverOf(session(), "owner", me(ada, two), { chosen: "cred_2" })).toBeUndefined();
    // A single connection, or another that no longer works.
    expect(takeOverOf(session(), "owner", me(ada), { mine: "cred_0", chosen: "cred_1" })).toBeUndefined();
    expect(
      takeOverOf(session(), "owner", me(ada, [connection("cred_1"), connection("cred_2", "invalid")]), {
        mine: "cred_1",
        chosen: "cred_2",
      }),
    ).toBeUndefined();
  });

  it("picks the workspace's connection as the server does", () => {
    const a = connection("cred_a", "active", 1);
    const b = connection("cred_b", "active", 3);
    const c = connection("cred_c", "active", 2, true);
    expect(workspaceConnection([a, b, c], "cred_a")).toBe("cred_a");
    expect(workspaceConnection([a, b, c], "cred_gone")).toBe("cred_c");
    expect(workspaceConnection([a, b], undefined)).toBe("cred_b");
    expect(workspaceConnection([], undefined)).toBeUndefined();
  });

  it("offers sessions that rest, but not ones at work or failed", () => {
    expect(takeOverOf(session({ status: "idle" }), "editor", me(bo))).toEqual({ whose: "Ada", waits: false });
    expect(takeOverOf(session({ status: "stopped" }), "editor", me(bo))?.waits).toBe(false);
    for (const status of ["queued", "provisioning", "running", "failed"] as const) {
      expect(takeOverOf(session({ status }), "editor", me(bo))).toBeUndefined();
    }
  });

  it("is only for those who may message it, with a Claude connection", () => {
    expect(takeOverOf(session(), "viewer", me(bo))).toBeUndefined();
    expect(takeOverOf(session(), undefined, me(bo))).toBeUndefined();
    expect(takeOverOf(session(), "editor", me(bo, []))).toBeUndefined();
    expect(takeOverOf(session(), "editor", null)).toBeUndefined();
  });

  it("leaves review agents and autopilots' own sessions alone", () => {
    const review = { repo: { owner: "acme", name: "shop" }, number: 4, url: "u", head_sha: "abc" };
    expect(takeOverOf(session({ review }), "editor", me(bo))).toBeUndefined();
    const pilot = { role: "pilot", project_id: "prj_1", project_slug: "shop" } as G.SessionAutopilot;
    expect(takeOverOf(session({ autopilot: pilot }), "editor", me(bo))).toBeUndefined();
    expect(takeOverOf(session({ autopilot: { ...pilot, role: "crew" } }), "editor", me(bo))).toEqual({
      whose: "Ada",
      waits: true,
    });
  });
});

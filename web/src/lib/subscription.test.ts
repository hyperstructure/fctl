import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { takeOverOf } from "./subscription";

const ada: G.UserRef = { id: "usr_ada", name: "Ada" };
const bo: G.UserRef = { id: "usr_bo", name: "Bo" };

const connection = (id: string, status: G.CredentialStatus = "active") =>
  ({ id, kind: "oauth", status }) as unknown as G.ClaudeCredentialInfo;

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

  it("offers your own session only when you have another connection to move it to", () => {
    expect(takeOverOf(session(), "owner", me(ada))).toBeUndefined();
    expect(takeOverOf(session(), "owner", me(ada, [connection("cred_1"), connection("cred_2")]))).toEqual({
      whose: undefined,
      waits: true,
    });
    expect(
      takeOverOf(session(), "owner", me(ada, [connection("cred_1"), connection("cred_2", "invalid")])),
    ).toBeUndefined();
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

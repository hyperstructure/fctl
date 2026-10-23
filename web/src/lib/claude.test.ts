import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { connectionIn, tokenOnRunnersIn, tokenRunnersIn, tokenState, yourRunnerTokens } from "./claude";

const cred = (c: Partial<G.ClaudeCredentialInfo> & { id: string }): G.ClaudeCredentialInfo => ({
  kind: "setup_token",
  remote_control: false,
  status: "active",
  is_default: false,
  created_at: 0,
  ...c,
});

const me = (claude: G.ClaudeCredentialInfo[]) => ({ claude });

const ref = (id: string): G.WorkspaceRef => ({ id, slug: id, name: id, color: "blue" });

const runner = (r: Partial<G.Runner> & { id: string }): G.Runner => ({
  name: r.id,
  status: "online",
  drivers: [],
  capacity: 4,
  active_sessions: 0,
  labels: [],
  created_at: 0,
  workspaces: [ref("ws_side")],
  yours: true,
  ...r,
});

describe("connectionIn", () => {
  const kept = cred({ id: "crd_kept", on_runners: true, created_at: 1 });
  const seat = cred({ id: "crd_seat", kind: "oauth", is_default: true, created_at: 2 });
  const broken = cred({ id: "crd_old", status: "invalid", created_at: 3 });

  it("takes the one chosen in the workspace, else the default, as the control plane does", () => {
    expect(connectionIn(me([kept, seat]), { claude: "crd_kept" })?.id).toBe("crd_kept");
    expect(connectionIn(me([kept, seat]), {})?.id).toBe("crd_seat");
    expect(tokenOnRunnersIn(me([kept, seat]), { claude: "crd_kept" })).toBe(true);
    expect(tokenOnRunnersIn(me([kept, seat]), {})).toBe(false);
  });

  it("passes over a chosen one that stopped working, and falls back to the newest working one", () => {
    expect(connectionIn(me([kept, seat, broken]), { claude: "crd_old" })?.id).toBe("crd_seat");
    const noDefault = [kept, cred({ id: "crd_newer", created_at: 5 }), broken];
    expect(connectionIn(me(noDefault), {})?.id).toBe("crd_newer");
    expect(connectionIn(me([]), {})).toBeUndefined();
    expect(connectionIn(undefined, {})).toBeUndefined();
  });
});

describe("where your runners stand with your token", () => {
  it("tells runners that keep it from those that could and those too old to", () => {
    expect(tokenState(runner({ id: "a", claude_token: { saved_at: 5 } }))).toBe("kept");
    expect(tokenState(runner({ id: "b", claude_token: {} }))).toBe("none");
    expect(tokenState(runner({ id: "c" }))).toBe("old");
  });

  it("lists yours, those that keep it first and online first", () => {
    const list = yourRunnerTokens([
      runner({ id: "old-box" }),
      runner({ id: "theirs", yours: false, claude_token: { saved_at: 1 } }),
      runner({ id: "build", status: "offline", claude_token: { saved_at: 2 } }),
      runner({ id: "spare", claude_token: {} }),
      runner({ id: "studio", claude_token: { saved_at: 3 } }),
    ]);
    expect(list.map((r) => `${r.runner.id}:${r.state}`)).toEqual([
      "studio:kept",
      "build:kept",
      "spare:none",
      "old-box:old",
    ]);
  });

  it("counts yours in a workspace, those that keep it, and those of them online", () => {
    const runners = [
      runner({ id: "studio", claude_token: { saved_at: 3 } }),
      runner({ id: "build", status: "offline", claude_token: { saved_at: 2 } }),
      runner({ id: "spare", claude_token: {} }),
      runner({ id: "work-only", workspaces: [ref("ws_work")], claude_token: { saved_at: 1 } }),
      runner({ id: "office", yours: false, claude_token: { saved_at: 1 } }),
    ];
    expect(tokenRunnersIn(runners, { id: "ws_side" })).toEqual({ here: 3, kept: 2, online: 1 });
    expect(tokenRunnersIn(runners, { id: "ws_work" })).toEqual({ here: 1, kept: 1, online: 1 });
    expect(tokenRunnersIn(runners, { id: "ws_none" })).toEqual({ here: 0, kept: 0, online: 0 });
  });
});

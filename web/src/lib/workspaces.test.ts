import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { claudeOptions, createsProjects, runnersOf } from "./workspaces";

const cred = (c: Partial<G.ClaudeCredentialInfo> & { id: string }): G.ClaudeCredentialInfo => ({
  kind: "oauth",
  remote_control: true,
  status: "active",
  is_default: false,
  created_at: 0,
  ...c,
});

const me = (claude: G.ClaudeCredentialInfo[]) => ({ claude }) as G.Me;

describe("claudeOptions", () => {
  const work = cred({ id: "crd_work", account_email: "ada@company.example", subscription_type: "team" });
  const own = cred({ id: "crd_own", account_email: "ada@private.example", is_default: true });
  const broken = cred({ id: "crd_old", account_email: "old@company.example", status: "invalid" });

  it("offers the default first, then each working connection", () => {
    const options = claudeOptions(me([work, own, broken]), undefined);
    expect(options.map((o) => o.value)).toEqual(["default", "crd_work", "crd_own"]);
    expect(options[0]?.label).toBe("Your default (ada@private.example)");
  });

  it("keeps the chosen one when it stopped working, saying so", () => {
    const options = claudeOptions(me([work, own, broken]), "crd_old");
    expect(options.map((o) => o.value)).toEqual(["default", "crd_work", "crd_own", "crd_old"]);
    expect(options[3]?.label).toBe("old@company.example (reconnect it)");
  });

  it("names one that was removed instead of showing its id", () => {
    const options = claudeOptions(me([own]), "crd_gone", "Claude: ");
    expect(options.at(-1)).toMatchObject({ value: "crd_gone", label: "Claude: A connection you removed" });
  });
});

describe("runnersOf", () => {
  const ref = (id: string): G.WorkspaceRef => ({ id, slug: id, name: id, color: "blue" });
  const runner = (r: Partial<G.Runner> & { id: string }): G.Runner => ({
    name: r.id,
    status: "online",
    drivers: [],
    capacity: 4,
    active_sessions: 0,
    labels: [],
    created_at: 0,
    workspaces: [],
    yours: false,
    ...r,
  });

  const ws = (id: string, own_runners = false) => ({ id, own_runners });

  it("counts the runners that serve the workspace, and those online", () => {
    const runners = [
      runner({ id: "studio", workspaces: [ref("ws_side"), ref("ws_work")] }),
      runner({ id: "mini", status: "offline", workspaces: [ref("ws_side")] }),
      runner({ id: "old", status: "disabled", workspaces: [ref("ws_side")] }),
      runner({ id: "office", workspaces: [ref("ws_work")] }),
    ];
    expect(runnersOf(runners, ws("ws_side"))).toEqual({ total: 3, online: 1, others: 0 });
    expect(runnersOf(runners, ws("ws_work"))).toEqual({ total: 2, online: 2, others: 0 });
  });

  it("leaves out other members' runners for their own sessions, and with own runners everyone else's", () => {
    const runners = [
      runner({ id: "office", workspaces: [ref("ws_work")] }),
      runner({ id: "bos-laptop", workspaces: [ref("ws_work")], personal: ["ws_work"] }),
      runner({
        id: "mine",
        status: "offline",
        workspaces: [ref("ws_work")],
        personal: ["ws_work"],
        yours: true,
      }),
    ];
    expect(runnersOf(runners, ws("ws_work"))).toEqual({ total: 2, online: 1, others: 1 });
    expect(runnersOf(runners, ws("ws_work", true))).toEqual({ total: 1, online: 0, others: 2 });
  });

  it("finds none in a new workspace, even beside runners of no workspace", () => {
    expect(runnersOf([], ws("ws_new"))).toEqual({ total: 0, online: 0, others: 0 });
    expect(runnersOf([runner({ id: "spare", yours: true })], ws("ws_new"))).toEqual({
      total: 0,
      online: 0,
      others: 0,
    });
  });

  it("can't tell from a control plane before workspaces", () => {
    const before = runner({ id: "studio" });
    delete before.workspaces;
    expect(runnersOf([before], ws("ws_side"))).toBeUndefined();
  });
});

describe("createsProjects", () => {
  it("is for a workspace's admins, while it is open to them", () => {
    expect(createsProjects({ role: "owner", blocked: false })).toBe(true);
    expect(createsProjects({ role: "admin", blocked: false })).toBe(true);
    expect(createsProjects({ role: "member", blocked: false })).toBe(false);
    expect(createsProjects({ role: "admin", blocked: true })).toBe(false);
  });
});

import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  accountState,
  answerWords,
  asksOf,
  startedThrough,
  tallyWords,
  usageSnippet,
} from "./service-accounts";

const workspaceRef = { id: "wsp_acme", slug: "acme", name: "Acme", color: "yellow" } as G.WorkspaceRef;

function account(over: Partial<G.ServiceAccount> = {}): G.ServiceAccount {
  return {
    id: "sva_portal",
    workspace: workspaceRef,
    name: "Acme portal",
    created_at: 1,
    acts_for_you: false,
    consent: true,
    can_manage: false,
    ...over,
  };
}

function workspace(over: Partial<G.Workspace> = {}): G.Workspace {
  return {
    ...workspaceRef,
    role: "member",
    email: "ada@acme.dev",
    base_role: "editor",
    require_two_factor: false,
    instance_requires_two_factor: false,
    blocked: false,
    members: 3,
    projects: 1,
    runners: 1,
    runners_online: 1,
    github: [],
    created_at: 1,
    ...over,
  } as G.Workspace;
}

describe("asksOf", () => {
  it("lists what hasn't heard your answer, where people choose", () => {
    const asking = account();
    const allowed = account({ id: "sva_bot", name: "Slack bot", answer: "allowed", acts_for_you: true });
    const declined = account({ id: "sva_cli", name: "Script", answer: "declined" });
    const asks = asksOf([workspace({ service_accounts: [asking, allowed, declined] })]);
    expect(asks.map((a) => a.account.name)).toEqual(["Acme portal"]);
    expect(asks[0]?.workspace.slug).toBe("acme");
  });

  it("asks nothing where the instance doesn't ask people, or the workspace is closed to you", () => {
    expect(
      asksOf([workspace({ service_accounts: [account({ consent: false, acts_for_you: true })] })]),
    ).toEqual([]);
    expect(asksOf([workspace({ blocked: true, service_accounts: [account()] })])).toEqual([]);
    expect(asksOf([workspace()])).toEqual([]);
    expect(asksOf(undefined)).toEqual([]);
  });
});

describe("answerWords", () => {
  it("says how it stands for you", () => {
    expect(answerWords(account())).toBe("Asks to start agents for you");
    expect(answerWords(account({ answer: "allowed" }))).toBe("Starts agents for you");
    expect(answerWords(account({ answer: "declined" }))).toBe("You didn't allow it");
    expect(answerWords(account({ consent: false, answer: "declined" }))).toBe(
      "Starts agents for anyone here",
    );
  });
});

describe("tallyWords", () => {
  it("counts how its people answered", () => {
    expect(tallyWords({ allowed: 4, declined: 1, unanswered: 7 }, true)).toBe(
      "4 of 12 allowed it · 1 didn't · 7 haven't answered",
    );
    expect(tallyWords({ allowed: 1, declined: 0, unanswered: 1 }, true)).toBe(
      "1 of 2 allowed it · 1 hasn't answered",
    );
    expect(tallyWords({ allowed: 3, declined: 0, unanswered: 0 }, true)).toBe("3 of 3 allowed it");
    expect(tallyWords({ allowed: 0, declined: 0, unanswered: 1 }, false)).toBe(
      "It acts for all 1 person here",
    );
    expect(tallyWords({ allowed: 2, declined: 1, unanswered: 9 }, false)).toBe(
      "It acts for all 12 people here",
    );
  });
});

describe("accountState", () => {
  it("is stopped with its keeper, then used or new", () => {
    expect(accountState(account({ stopped: "Ada, who made its secret, is no longer an admin" }))).toBe(
      "stopped",
    );
    expect(accountState(account({ last_used_at: 5 }))).toBe("used");
    expect(accountState(account())).toBe("new");
  });
});

describe("usageSnippet", () => {
  const where = { origin: "https://fctl.acme.dev", project: "web", person: "ada@acme.dev" };

  it("names the person in the header, and the secret only where it's known", () => {
    const curl = usageSnippet("curl", where);
    expect(curl).toContain('-H "Fctl-On-Behalf-Of: ada@acme.dev"');
    expect(curl).toContain('-H "Authorization: Bearer $FCTL_TOKEN"');
    expect(curl).toContain("https://fctl.acme.dev/api/v1/projects/web/sessions");
    const made = usageSnippet("curl", { ...where, secret: "fctl_svc_abc" });
    expect(made).toContain("Bearer fctl_svc_abc");
  });

  it("starts the CLI's session detached, for the person", () => {
    const cli = usageSnippet("cli", { ...where, secret: "fctl_svc_abc" });
    expect(cli).toContain("export FCTL_TOKEN=fctl_svc_abc");
    expect(cli).toContain('fctl run -p web --on-behalf-of ada@acme.dev -d "Fix the login redirect"');
    expect(usageSnippet("cli", where)).toContain("export FCTL_TOKEN=fctl_svc_…");
  });

  it("tells a program's refusals apart by their field", () => {
    const node = usageSnippet("node", where);
    expect(node).toContain('"Fctl-On-Behalf-Of": user.email');
    expect(node).toContain('body.field === "on_behalf_of"');
    expect(node).not.toContain("fctl_svc_");
  });
});

describe("startedThrough", () => {
  it("names the service account, or says it's gone", () => {
    expect(startedThrough({})).toBeUndefined();
    expect(startedThrough({ service_account: { id: "sva_1", name: "Acme portal" } })).toBe("Acme portal");
    expect(startedThrough({ service_account: { id: "sva_1" } })).toBe("a service account deleted since");
  });
});

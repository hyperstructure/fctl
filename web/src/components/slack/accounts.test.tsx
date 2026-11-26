import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { SlackAccounts, type SlackReturn } from "./accounts";

const goConfirmed = vi.hoisted(() => vi.fn(() => Promise.resolve()));
const unlinkSlack = vi.hoisted(() => vi.fn());

// No request is answered but unlinking: the page shows what the cache holds.
vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  const answered: Record<string | symbol, unknown> = { unlinkSlack };
  const api = new Proxy({}, { get: (_, k) => answered[k] ?? (() => new Promise(() => {})) });
  return {
    api,
    ApiFailure: actual.ApiFailure,
    slackLinkUrl: actual.slackLinkUrl,
    goConfirmed,
    clickConfirmed: () => undefined,
  };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  goConfirmed.mockClear();
  unlinkSlack.mockReset();
});

const acme: G.SlackTeamRef = {
  team: "T1",
  name: "Acme",
  domain: "acme",
  workspaces: [{ id: "wsp_1", slug: "acme", name: "Acme Corp" }],
};
const globex: G.SlackTeamRef = {
  team: "T2",
  name: "Globex",
  workspaces: [{ id: "wsp_2", slug: "globex", name: "Globex Labs" }],
};
const account: G.SlackAccount = {
  team: "T1",
  team_name: "Acme",
  team_domain: "acme",
  slack_user: "U1",
  name: "Ada Lovelace",
  email: "ada@acme.dev",
  via: "linked",
  dm_project: { id: "prj_web", slug: "web", name: "Web" },
  linked_at: Date.now() - 2 * 86_400_000,
};

function page(mine: G.MySlack, returned: SlackReturn = {}) {
  queryClient.setQueryData(keys.instance, { name: "fctl", features: { slack: true, email: true } });
  queryClient.setQueryData(keys.mySlack, mine);
  queryClient.setQueryData(keys.projects, {
    projects: [
      {
        id: "prj_web",
        slug: "web",
        name: "Web",
        workspace: { id: "wsp_1", name: "Acme Corp" },
        role: "editor",
      },
      {
        id: "prj_api",
        slug: "api",
        name: "API",
        workspace: { id: "wsp_1", name: "Acme Corp" },
        role: "viewer",
      },
      {
        id: "prj_ops",
        slug: "ops",
        name: "Ops",
        workspace: { id: "wsp_2", name: "Globex Labs" },
        role: "owner",
      },
    ],
  });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <SlackAccounts returned={returned} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

describe("Settings → Slack", () => {
  it("says whom fctl knows you as in each Slack workspace, and how", () => {
    const c = page({ configured: true, accounts: [account], teams: [acme] });
    expect(c.textContent).toContain("Ada Lovelace in Acme");
    expect(c.textContent).toContain("ada@acme.dev · acme.slack.com · linked 2 days ago");
    expect(c.textContent).toContain("Your direct messages with fctl start sessions in");
    expect(c.textContent).toContain("and emails you when one is linked");
  });

  it("offers the projects you start sessions in there, or to be asked each time", () => {
    const c = page({ configured: true, accounts: [account], teams: [acme] });
    const choice = c.querySelector(
      '[aria-label="The project your direct messages with fctl in Acme start sessions in"]',
    );
    expect(choice?.textContent).toContain("Web");
    const text = c.textContent ?? "";
    expect(text).toContain("Ask each time");
    // Not a project you only view, nor one of a workspace that doesn't work in that Slack workspace.
    expect(text).not.toContain("API");
    expect(text).not.toContain("Ops");
  });

  it("links an account in the Slack workspaces your workspaces work in, where fctl doesn't know you yet", () => {
    const c = page({ configured: true, accounts: [account], teams: [acme, globex] });
    expect(c.textContent).toContain("Globex Labs works there: until you link your account");
    const link = [...c.querySelectorAll("a")].find((a) => a.textContent?.includes("Link your account"));
    expect(link?.getAttribute("href")).toBe("/api/v1/slack/link?team=T2");
  });

  it("says when none of your workspaces works in Slack yet", () => {
    const c = page({ configured: true, accounts: [], teams: [] });
    expect(c.textContent).toContain("None of your workspaces works in a Slack workspace yet");
  });

  it("says that fctl knows you by your address, where it does, and what unlinking that does", async () => {
    const c = page({ configured: true, accounts: [{ ...account, via: "email" }], teams: [acme] });
    expect(c.textContent).toContain("matched by your address");
    expect(c.textContent).toContain(
      "fctl knows you there by your verified address, as Acme Corp's admins let it.",
    );
    unlinkSlack.mockResolvedValue({ configured: true, accounts: [], teams: [acme] });
    const unlink = [...c.querySelectorAll("button")].find((b) => b.textContent === "Unlink");
    await act(async () => unlink?.click());
    const dialog = document.body.querySelector("dialog");
    expect(dialog?.textContent).toContain(
      "It knows you there again by your address the next time you ask fctl",
    );
    const confirm = [...(dialog?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "Unlink");
    await act(async () => confirm?.click());
    expect(unlinkSlack).toHaveBeenCalledWith("T1");
  });

  it("asks you to confirm it's you on the way to link an account, then goes on to Slack", () => {
    const c = page(
      { configured: true, accounts: [], teams: [globex] },
      { slack: "confirm", team: "T2", thread: "T2:C1:1.2" },
    );
    expect(c.textContent).toContain("Confirm it's you to link your Slack account in Globex");
    expect(c.textContent).toContain("and your request there starts");
    expect(goConfirmed).toHaveBeenCalledTimes(1);
    expect(goConfirmed).toHaveBeenCalledWith("/api/v1/slack/link?team=T2&thread=T2%3AC1%3A1.2");
  });

  it("says how linking went, back from Slack", () => {
    const c = page({ configured: true, accounts: [account], teams: [acme] }, { slack: "linked" });
    expect(c.textContent).toContain("Your Slack account is linked");
    expect(goConfirmed).not.toHaveBeenCalled();
  });
});

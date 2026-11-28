import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import type { SlackOutcome } from "@/lib/slack";
import { SlackTab } from "./workspace";

const updateSlack = vi.hoisted(() => vi.fn());
const connectSlack = vi.hoisted(() => vi.fn());

// No request is answered but changing a choice and connecting: the tab shows what the cache holds.
vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  const never = new Proxy({}, { get: () => () => new Promise(() => {}) });
  const answered: Record<string | symbol, unknown> = { updateSlack, connectSlack };
  const workspaces = new Proxy({}, { get: (_, k) => answered[k] ?? (() => new Promise(() => {})) });
  return {
    api: never,
    workspaces,
    ApiFailure: actual.ApiFailure,
    slackLinkUrl: actual.slackLinkUrl,
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
  updateSlack.mockReset();
  connectSlack.mockReset();
  vi.restoreAllMocks();
});

const workspace = (role: G.WorkspaceRole) =>
  ({
    id: "wsp_1",
    slug: "acme",
    name: "Acme",
    color: "yellow",
    role,
    members: 12,
    primary: { id: "usr_ada", name: "Ada Lovelace" },
  }) as unknown as G.Workspace;

const settings: G.SlackSettings = {
  channels: true,
  direct_messages: true,
  search: true,
  agent_replies: true,
  notifications: true,
  project_events: true,
  match_email: false,
  transcripts: "keep",
};

const connection: G.SlackConnection = {
  team: "T1",
  name: "Acme Corp",
  domain: "acme",
  bot_user: "U0FCTL",
  app_name: "fctl",
  status: "active",
  connected_by: { id: "usr_ada", name: "Ada Lovelace" },
  slack_user: "U0ADA",
  created_at: Date.now() - 3 * 86_400_000,
  settings,
  channels: [
    {
      id: "C1",
      name: "web-team",
      project: { id: "prj_web", slug: "web", name: "Web" },
      updated_at: 0,
    },
    { id: "C2", name: "incidents", readable: true, updated_at: 0 },
  ],
  projects: [
    {
      id: "prj_web",
      slug: "web",
      name: "Web",
      read_channels: true,
      mapped: [{ id: "C1", name: "web-team" }],
    },
    {
      id: "prj_api",
      slug: "api",
      name: "API",
      read_channels: false,
      events: { team: "T1", channel: "C3", channel_name: "api-alerts" },
    },
  ],
  reach: "Agents read the thread they started from, and #incidents, which they can search.",
  linked: 3,
};

const slack = (s: Partial<G.WorkspaceSlack> = {}): G.WorkspaceSlack => ({
  configured: true,
  events: true,
  socket_mode: false,
  match_email: false,
  instance_admin: false,
  connections: [],
  ...s,
});

function tab(s: G.WorkspaceSlack, role: G.WorkspaceRole = "admin", returned?: SlackOutcome, team?: string) {
  queryClient.setQueryData(keys.workspaceSlack("wsp_1"), s);
  queryClient.setQueryData(keys.instance, { name: "fctl", features: { slack: true } });
  queryClient.setQueryData(keys.projects, {
    projects: [{ id: "prj_web", slug: "web", name: "Web", workspace: { id: "wsp_1" }, role: "owner" }],
  });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <SlackTab w={workspace(role)} returned={returned} team={team} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

const buttons = (c: HTMLElement) => [...c.querySelectorAll("button")];
const button = (c: HTMLElement, text: string) => buttons(c).find((b) => b.textContent?.trim() === text);
const switches = (c: HTMLElement) => [...c.querySelectorAll<HTMLButtonElement>('[role="switch"]')];
/** Clicks, then lets what it started settle: queries tell their observers on a timer. */
const click = async (el: Element | null | undefined) => {
  await act(async () => {
    el?.dispatchEvent(new MouseEvent("click", { bubbles: true }));
  });
  await act(() => new Promise((done) => setTimeout(done, 10)));
};

describe("a workspace's Slack, before it's connected", () => {
  it("says what connecting leads to here, what agents read and never reach, with a thread as Slack shows it", () => {
    const c = tab(slack());
    const text = c.textContent ?? "";
    expect(text).toContain("Start agents from Slack");
    // What connecting leads to, in this workspace's words: its project, fctl's app as people mention it.
    expect(text).toContain(
      "Connect a Slack workspace, and people there start agents in Web by mentioning @fctl",
    );
    expect(text).toContain("Each session runs as the person who asked, on their Claude subscription.");
    expect(text).toContain("Agents read the thread they started from");
    expect(text).toContain(
      "They never reach other messages of private channels, anyone's direct messages with fctl, or channels shared with other organizations.",
    );
    // The preview: the card, its plan and pull request, a question with its answers, and a reply.
    const preview = c.querySelector('[aria-hidden="true"].select-none');
    expect(preview?.textContent).toContain("Running the tests");
    expect(preview?.textContent).toContain("3 of 4 done");
    expect(preview?.textContent).toContain("checks passed");
    expect(preview?.textContent).toContain("Open in fctl");
    expect(preview?.textContent).toContain("The agent asks");
    expect(preview?.textContent).toContain("Keep the redirect");
  });

  it("has admins add it to Slack, one primary action", async () => {
    connectSlack.mockResolvedValue({ url: "https://slack.com/oauth/v2/authorize?x", expires_at: 1 });
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => {});
    const c = tab(slack());
    const add = button(c, "Add to Slack");
    expect(add?.className).toContain("bg-highlight");
    await click(add);
    expect(connectSlack).toHaveBeenCalledWith("wsp_1", undefined);
    expect(assign).toHaveBeenCalledWith("https://slack.com/oauth/v2/authorize?x");
  });

  it("tells everyone else who adds it", () => {
    const c = tab(slack(), "member");
    expect(button(c, "Add to Slack")).toBeUndefined();
    expect(c.textContent).toContain("Acme's admins add it to Slack (Ada Lovelace is its primary admin)");
  });

  it("says when nothing in Slack reaches fctl, and which secrets the instance's admins set", () => {
    const c = tab(slack({ events: false }));
    expect(c.textContent).toContain("Nothing people do in Slack reaches fctl yet");
    expect(c.textContent).toContain("SLACK_SIGNING_SECRET, or SLACK_APP_TOKEN for Socket Mode");
  });
});

describe("a workspace's Slack on an instance without a Slack app", () => {
  const app: G.AdminSlack = {
    configured: false,
    signing_secret: false,
    socket_mode: false,
    reachable: true,
    manifest: { display_information: { name: "fctl" } },
    manifest_yaml: "display_information:\n  name: fctl\n",
    create_url: "https://api.slack.com/apps?new_app=1&manifest_json=x",
    events_url: "https://fctl.acme.dev/api/v1/slack/events",
    interactivity_url: "https://fctl.acme.dev/api/v1/slack/interactivity",
    commands_url: "https://fctl.acme.dev/api/v1/slack/commands",
    redirect_urls: ["https://fctl.acme.dev/api/v1/slack/installed"],
    match_email: false,
  };

  it("shows the instance's admins how to create it, from its manifest", () => {
    queryClient.setQueryData(keys.adminSlack, app);
    const c = tab(slack({ configured: false, instance_admin: true }));
    expect(c.textContent).toContain("This instance has no Slack app yet.");
    const create = [...c.querySelectorAll("a")].find((a) => a.textContent?.includes("Create the Slack app"));
    expect(create?.getAttribute("href")).toBe(app.create_url);
    expect(create?.getAttribute("target")).toBe("_blank");
    for (const name of ["SLACK_CLIENT_ID", "SLACK_CLIENT_SECRET", "SLACK_SIGNING_SECRET", "SLACK_APP_TOKEN"])
      expect(c.textContent).toContain(name);
  });

  it("tells everyone else that the instance's admins create it", () => {
    const c = tab(slack({ configured: false }));
    expect(c.textContent).toContain("Its admins create one");
    expect(c.textContent).not.toContain("SLACK_CLIENT_ID");
  });
});

describe("a workspace's Slack, connected", () => {
  it("says what agents can see there, prominently, and who connected it", () => {
    const c = tab(slack({ connections: [connection] }));
    const text = c.textContent ?? "";
    expect(text).toContain("People in Acme Corp start Acme's agents by mentioning @fctl");
    expect(text).toContain(
      "Agents read the thread they started from, and #incidents, which they can search.",
    );
    expect(text).toContain("acme.slack.com · as @fctl · connected by Ada Lovelace 3 days ago");
    expect(text).toContain("3 people here linked their accounts");
    expect(text).toContain("Connected");
    expect(text).not.toContain("Start agents from Slack, before");
  });

  it("puts each channel's project in a sentence, and the readable channels as chips", () => {
    const c = tab(slack({ connections: [connection] }));
    expect(c.textContent).toContain("Mentions in web-team start sessions in");
    expect(
      c.querySelector('[aria-label="The project mentions in #web-team start sessions in"]'),
    ).not.toBeNull();
    const chips = c.querySelector('[aria-label="Readable channels"]');
    expect(chips?.textContent).toContain("incidents");
    expect(c.querySelector('[aria-label="Stop agents reading #incidents"]')).not.toBeNull();
    expect(button(c, "Add a channel")).toBeDefined();
  });

  it("lists each project's choice of what its agents read, and the events projects post", () => {
    const text = tab(slack({ connections: [connection] })).textContent ?? "";
    expect(text).toContain("WebReads them");
    expect(text).toContain("APIReads its threads alone");
    expect(text).toContain(
      "API posts to #api-alerts when pull requests are ready for review or a session fails.",
    );
  });

  it("saves a choice at once, and says what follows", async () => {
    updateSlack.mockResolvedValue(
      slack({ connections: [{ ...connection, settings: { ...settings, channels: false } }] }),
    );
    const c = tab(slack({ connections: [connection] }));
    const mentions = switches(c).find((s) => s.getAttribute("aria-label") === "Mentions in channels");
    expect(mentions?.getAttribute("aria-checked")).toBe("true");
    await click(mentions);
    expect(updateSlack).toHaveBeenCalledWith("wsp_1", "T1", { channels: false });
    expect(mentions?.getAttribute("aria-checked")).toBe("false");
  });

  it("lets admins know people by their address only where the instance allows it", () => {
    const off = tab(slack({ connections: [connection] }));
    expect(
      switches(off).some((s) => s.getAttribute("aria-label") === "Know people by their verified address"),
    ).toBe(false);
    expect(off.textContent).toContain(
      "this instance's admins don't let workspaces know people by their address",
    );
    const on = tab(slack({ connections: [connection], match_email: true }));
    expect(
      switches(on).some((s) => s.getAttribute("aria-label") === "Know people by their verified address"),
    ).toBe(true);
  });

  it("shows members what is chosen, and who chooses, with nothing to change", () => {
    const c = tab(slack({ connections: [connection] }), "member");
    expect(switches(c)).toEqual([]);
    expect(c.textContent).toContain("Acme's admins change these (Ada Lovelace is its primary admin).");
    expect(c.textContent).toContain("On");
    expect(c.querySelector('[aria-label="Stop agents reading #incidents"]')).toBeNull();
    expect(button(c, "Add a channel")).toBeUndefined();
    expect(c.textContent).not.toContain("Danger zone");
  });

  it("asks you to link your account until fctl knows you there", () => {
    const unlinked = tab(slack({ connections: [connection] }));
    const link = [...unlinked.querySelectorAll("a")].find((a) =>
      a.textContent?.includes("Link your Slack account"),
    );
    expect(link?.getAttribute("href")).toBe("/api/v1/slack/link?team=T1");
    const linked = tab(slack({ connections: [connection], linked_teams: ["T1"] }));
    expect(linked.textContent).toContain("fctl knows your Slack account in Acme Corp");
  });

  it("says what to do when Slack took fctl's access back", () => {
    const revoked = { ...connection, status: "revoked" as const };
    const admin = tab(slack({ connections: [revoked] }));
    expect(button(admin, "Connect again")).toBeDefined();
    expect(admin.textContent).toContain("Slack took fctl's access back");
    expect(admin.textContent).not.toContain(connection.reach);
    const member = tab(slack({ connections: [revoked] }), "member");
    expect(member.textContent).toContain("Access revoked");
    expect(button(member, "Connect again")).toBeUndefined();
  });

  it("connects the same Slack workspace again", async () => {
    connectSlack.mockResolvedValue({ url: "https://slack.com/oauth/v2/authorize?team=T1", expires_at: 1 });
    const assign = vi.spyOn(window.location, "assign").mockImplementation(() => {});
    const c = tab(slack({ connections: [{ ...connection, status: "revoked" as const }] }));
    await click(button(c, "Connect again"));
    expect(connectSlack).toHaveBeenCalledWith("wsp_1", "T1");
    expect(assign).toHaveBeenCalledWith("https://slack.com/oauth/v2/authorize?team=T1");
  });

  it("says when Socket Mode isn't connected", () => {
    const c = tab(slack({ connections: [connection], socket_mode: true, socket_connected: false }));
    expect(c.textContent).toContain("fctl isn't connected to Slack's Socket Mode right now");
  });

  it("disconnects behind a question that says what goes and what stays", () => {
    const c = tab(slack({ connections: [connection] }));
    expect(c.textContent).toContain("Danger zone");
    expect(c.textContent).toContain("Sessions it started carry on, without posting there.");
  });

  it("confirms the connection on the way back from Slack", () => {
    const c = tab(slack({ connections: [connection] }), "admin", "connected");
    expect(c.textContent).toContain("fctl's app is in Acme Corp now");
    // The Slack workspace the way back names, though another was connected later.
    const older = { ...connection, team: "T0", name: "Globex", created_at: connection.created_at - 1 };
    const later = { ...connection, created_at: connection.created_at + 1 };
    const named = tab(slack({ connections: [older, later] }), "admin", "connected", "T0");
    expect(named.textContent).toContain("fctl's app is in Globex now");
  });
});

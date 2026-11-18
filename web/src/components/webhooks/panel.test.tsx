import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { WebhooksPanel } from "./panel";

const { createWorkspaceWebhook } = vi.hoisted(() => ({ createWorkspaceWebhook: vi.fn() }));
// Only adding a webhook is answered; other requests wait, and the panel shows what the cache holds.
vi.mock("@/lib/api", () => ({
  api: new Proxy({} as Record<string, unknown>, {
    get: (_, k) => (k === "createWorkspaceWebhook" ? createWorkspaceWebhook : () => new Promise(() => {})),
  }),
  ApiFailure: class extends Error {},
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  vi.resetAllMocks();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={queryClient}>{node}</QueryClientProvider>));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const settle = () => act(() => new Promise((r) => setTimeout(r, 10)));

const CATALOG: G.WebhookEvents = {
  groups: [
    { id: "sessions", label: "Sessions" },
    { id: "reviews", label: "Reviews" },
  ],
  events: [
    {
      type: "session.started",
      group: "sessions",
      label: "A session starts",
      description: "",
      sessions: true,
    },
    {
      type: "session.ready_for_review",
      group: "sessions",
      label: "Pull requests are ready for review",
      description: "",
      sessions: true,
    },
    {
      type: "session.question",
      group: "sessions",
      label: "An agent asks something",
      description: "",
      sessions: true,
    },
    { type: "session.failed", group: "sessions", label: "A session fails", description: "", sessions: true },
    {
      type: "review.finished",
      group: "reviews",
      label: "A review is posted",
      description: "",
      sessions: false,
    },
  ],
};

const workspace = {
  id: "wsp_1",
  slug: "acme",
  name: "Acme",
  role: "owner",
} as G.Workspace;

const project = {
  id: "prj_web",
  slug: "web",
  name: "Web",
  role: "maintainer",
  workspace: { id: "wsp_1", slug: "acme", name: "Acme" },
} as G.Project;

const hook = (over: Partial<G.Webhook> = {}): G.Webhook => ({
  id: "whk_1",
  workspace_id: "wsp_1",
  url: "https://hooks.slack.com/services/x",
  events: [],
  projects: [],
  started_by: [],
  enabled: true,
  created_at: 0,
  updated_at: 0,
  can_manage: true,
  ...over,
});

function seed(scope: "workspace" | "project", webhooks: G.Webhook[]) {
  queryClient.setQueryData(keys.webhookEvents, CATALOG);
  queryClient.setQueryData(keys.projects, { projects: [project] });
  queryClient.setQueryData(keys.webhooks(scope, scope === "workspace" ? "wsp_1" : "prj_web"), {
    webhooks,
    count: webhooks.length,
    limit: 20,
  } satisfies G.WebhookList);
}

const text = (c: HTMLElement) => c.textContent ?? "";
const button = (c: HTMLElement, name: string) =>
  [...c.querySelectorAll("button")].find((b) => b.textContent?.trim() === name) as HTMLButtonElement;

describe("a workspace's webhooks", () => {
  it("says what they do here before there are any, and who adds them", async () => {
    seed("workspace", []);
    const c = render(<WebhooksPanel scope={{ kind: "workspace", workspace }} onOpen={() => {}} />);
    await settle();
    expect(c.querySelector("h2")?.textContent).toBe("What agents do, posted to your tools");
    expect(text(c)).toContain("When a session in Acme starts, finishes a turn, asks something");
    expect(text(c)).toContain("Acme's admins add webhooks for all its projects, or some");
    expect(button(c, "Add a webhook")).toBeTruthy();
  });

  it("says what each posts in a sentence, and what's wrong with one that fails or was turned off", async () => {
    seed("workspace", [
      hook({
        name: "Slack",
        events: ["session.question", "session.ready_for_review"],
        projects: ["prj_web"],
        started_by: ["person"],
        health: { delivered: 3, failed: 1, pending: 2, dropped: 0, failing_since: Date.now() - 60_000 },
      }),
      hook({
        id: "whk_2",
        url: "https://deploy.example.com/hook",
        enabled: false,
        disabled_reason: "Every delivery failed for 3 days, the last with HTTP 502.",
      }),
    ]);
    const c = render(<WebhooksPanel scope={{ kind: "workspace", workspace }} onOpen={() => {}} />);
    await settle();
    expect(text(c)).toContain("2 webhooks · 1 failing · 1 off");
    expect(text(c)).toContain(
      "Posts to hooks.slack.com when pull requests are ready for review or an agent asks something, in Web, for sessions started by people.",
    );
    expect(text(c)).toContain("Every attempt has failed since");
    expect(text(c)).toContain(
      "fctl turned it off. Every delivery failed for 3 days, the last with HTTP 502.",
    );
    expect(text(c)).toContain("deploy.example.com");
  });

  it("adds one, says what it will post as it's chosen, and shows its secret once", async () => {
    seed("workspace", []);
    createWorkspaceWebhook.mockResolvedValue({
      webhook: hook({ id: "whk_new", name: "Deploy bot", events: ["session.failed"] }),
      secret: "whsec_c2VjcmV0",
    } satisfies G.CreatedWebhook);
    const c = render(<WebhooksPanel scope={{ kind: "workspace", workspace }} onOpen={() => {}} />);
    await settle();
    act(() => button(c, "Add a webhook").click());
    const url = c.querySelector<HTMLInputElement>("input[type=url]");
    expect(url).toBeTruthy();
    if (!url) return;
    act(() => {
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(url, "https://deploy.example.com/hook");
      url.dispatchEvent(new Event("input", { bubbles: true }));
    });
    // It starts from the events that need people, and says so in its sentence.
    expect(text(c)).toContain(
      "Posts to deploy.example.com when pull requests are ready for review, an agent asks something or a session fails.",
    );
    await act(async () => button(c, "Add webhook").click());
    await settle();
    expect(createWorkspaceWebhook).toHaveBeenCalledWith(
      "wsp_1",
      expect.objectContaining({
        url: "https://deploy.example.com/hook",
        events: ["session.ready_for_review", "session.question", "session.failed"],
        projects: [],
      }),
    );
    expect(text(c)).toContain("Added Deploy bot");
    expect(text(c)).toContain("whsec_c2VjcmV0");
    expect(text(c)).toContain("it shows once here");
  });
});

describe("a project's webhooks", () => {
  it("lists the workspace's that send its events apart, for its admins to manage", async () => {
    seed("project", [
      hook({ id: "whk_own", name: "Web's own", project: { id: "prj_web", slug: "web", name: "Web" } }),
      hook({ id: "whk_ws", name: "Acme-wide", can_manage: false }),
    ]);
    const open = vi.fn();
    const c = render(<WebhooksPanel scope={{ kind: "project", project }} onOpen={open} titled />);
    await settle();
    expect(text(c)).toContain("Acme's, which send Web's events too");
    expect(text(c)).toContain("The workspace's admins manage them.");
    // Its own opens its page; the workspace's is no button for those who can't manage it.
    act(() => [...c.querySelectorAll("button")].find((b) => b.textContent?.includes("Web's own"))?.click());
    expect(open).toHaveBeenCalledWith("whk_own");
    expect([...c.querySelectorAll("button")].some((b) => b.textContent?.includes("Acme-wide"))).toBe(false);
  });
});

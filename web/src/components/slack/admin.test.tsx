import { QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { AdminSlackTab } from "./admin";

const updateInstanceSettings = vi.hoisted(() => vi.fn());

vi.mock("@/lib/api", () => {
  const answered: Record<string | symbol, unknown> = { updateInstanceSettings };
  const api = new Proxy({}, { get: (_, k) => answered[k] ?? (() => new Promise(() => {})) });
  return { api };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  updateInstanceSettings.mockReset();
});

const app = (a: Partial<G.AdminSlack> = {}): G.AdminSlack => ({
  configured: true,
  signing_secret: true,
  socket_mode: false,
  reachable: true,
  manifest: { display_information: { name: "fctl" } },
  manifest_yaml: "display_information:\n  name: fctl\n",
  create_url: "https://api.slack.com/apps?new_app=1&manifest_json=x",
  events_url: "https://fctl.acme.dev/api/v1/slack/events",
  interactivity_url: "https://fctl.acme.dev/api/v1/slack/interactivity",
  commands_url: "https://fctl.acme.dev/api/v1/slack/commands",
  redirect_urls: [
    "https://fctl.acme.dev/api/v1/slack/installed",
    "https://fctl.acme.dev/api/v1/slack/linked",
  ],
  teams: [
    { team: "T1", name: "Acme", domain: "acme", status: "active", workspaces: 2, installed_at: Date.now() },
  ],
  match_email: false,
  ...a,
});

function tab(a: G.AdminSlack) {
  queryClient.setQueryData(keys.adminSlack, a);
  queryClient.setQueryData(keys.instance, { name: "fctl", features: {} });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <AdminSlackTab />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

describe("Admin → Slack", () => {
  it("walks admins through creating the app until it's whole", () => {
    const c = tab(app({ configured: false, signing_secret: false, teams: [] }));
    expect(c.textContent).toContain("Not set up yet");
    expect(c.textContent).toContain("Create the Slack app");
    expect(c.textContent).toContain("Nothing people do in Slack reaches fctl yet");
    const create = [...c.querySelectorAll("a")].find((a) => a.textContent?.includes("Create the Slack app"));
    expect(create?.getAttribute("href")).toBe("https://api.slack.com/apps?new_app=1&manifest_json=x");
    expect(c.textContent).toContain("None yet: a workspace's admins add the app to Slack");
  });

  it("shows a whole app's state, the addresses Slack calls and where it's installed", () => {
    const c = tab(app());
    const text = c.textContent ?? "";
    expect(text).toContain("Set up");
    expect(text).toContain("signed with SLACK_SIGNING_SECRET");
    expect(text).not.toContain("Give this instance its credentials");
    expect(text).toContain("https://fctl.acme.dev/api/v1/slack/events");
    expect(text).toContain("https://fctl.acme.dev/api/v1/slack/linked");
    expect(text).toContain("acme.slack.com · 2 workspaces work in it");
    expect(text).toContain("Installed");
  });

  it("says when Socket Mode isn't connected, and why", () => {
    const c = tab(
      app({
        signing_secret: false,
        socket_mode: true,
        socket_connected: false,
        socket_error: "invalid_auth",
      }),
    );
    expect(c.textContent).toContain("Socket Mode isn't connected: invalid_auth.");
  });

  it("says when Slack can't reach the instance over HTTP", () => {
    const c = tab(app({ reachable: false }));
    expect(c.textContent).toContain("Slack can't reach this instance's address");
  });

  it("lets workspaces know people by their address only once an admin chooses, with the risk beside it", async () => {
    updateInstanceSettings.mockResolvedValue({ slack_match_email: true });
    const c = tab(app());
    expect(c.textContent).toContain("Off: fctl knows only the people who link their Slack account");
    expect(c.textContent).toContain("Only where people sign in to Slack and fctl through the same provider");
    const toggle = c.querySelector<HTMLButtonElement>(
      '[role="switch"][aria-label="Workspaces may know people by their verified address"]',
    );
    await act(async () => toggle?.click());
    expect(updateInstanceSettings).toHaveBeenCalledWith({ slack_match_email: true });
  });
});

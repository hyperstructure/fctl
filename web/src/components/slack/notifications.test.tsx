import { QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { SlackNotifications, slackReaches } from "./notifications";

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  const never = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: never, slackLinkUrl: actual.slackLinkUrl, clickConfirmed: () => undefined };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

const place = (p: Partial<G.SlackNotifyPlace>): G.SlackNotifyPlace => ({
  team: "T1",
  team_name: "Acme",
  workspaces: [{ id: "wsp_1", slug: "acme", name: "Acme Corp" }],
  allowed: true,
  ...p,
});

function part(slack: G.SlackNotifyPlace[] | undefined, on = true, onChange = vi.fn(), app = true) {
  queryClient.setQueryData(keys.instance, { name: "fctl", features: { slack: app } });
  const n: G.Notifications = {
    settings: { waiting: true, paused: true, failed: true, follow_started: true, slack: on },
    devices: [],
    slack,
  };
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <SlackNotifications n={n} onChange={onChange} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

describe("Tell me in Slack", () => {
  it("isn't there where the instance has no Slack app", () => {
    expect(part(undefined, false, vi.fn(), false).innerHTML).toBe("");
  });

  it("stays where it's on, so that it can be turned off, though the instance has no Slack app", () => {
    const c = part(undefined, true, vi.fn(), false);
    expect(c.textContent).toContain("Tell me in Slack");
    expect(c.textContent).toContain("None of your workspaces works in Slack yet");
  });

  it("says no workspace works in Slack yet where none of yours does", () => {
    // The list comes absent when it's empty.
    const c = part(undefined, false);
    expect(c.textContent).toContain("Tell me in Slack");
    expect(c.textContent).toContain(
      "None of your workspaces works in Slack yet: its admins connect one under its Integrations.",
    );
  });

  it("says where they go, as whom, and why not where they can't", () => {
    const c = part([
      place({ account: { name: "Ada Lovelace" } }),
      place({
        team: "T2",
        team_name: "Globex",
        workspaces: [{ id: "wsp_2", slug: "globex", name: "Globex Labs" }],
      }),
      place({ team: "T3", team_name: "Initech", account: { name: "Ada" }, allowed: false }),
    ]);
    const text = c.textContent ?? "";
    expect(text).toContain("In Acme, to you as Ada Lovelace, about sessions in Acme Corp.");
    expect(text).toContain("In Globex, about sessions in Globex Labs, once fctl knows your account there.");
    expect(text).toContain("None in Initech: Acme Corp's admins keep notifications out of Slack.");
    const link = [...c.querySelectorAll("a")].find((a) => a.textContent === "Link your account");
    expect(link?.getAttribute("href")).toBe("/api/v1/slack/link?team=T2");
  });

  it("shows the direct message one comes as", () => {
    const c = part([place({ account: { name: "Ada" } })]);
    const preview = c.querySelector('.select-none[aria-hidden="true"]');
    expect(preview?.textContent).toContain("waits for your reply");
    expect(preview?.textContent).toContain("Open in fctl");
  });

  it("says what the switch does, off and on, and turns it", () => {
    const onChange = vi.fn();
    const off = part([place({ account: { name: "Ada" } })], false, onChange);
    expect(off.textContent).toContain("Off: your notifications come only to your browsers.");
    act(() => off.querySelector<HTMLButtonElement>('[role="switch"]')?.click());
    expect(onChange).toHaveBeenCalledWith({ slack: true });
    const unlinked = part([place({})]);
    expect(unlinked.textContent).toContain("Nothing goes there until you link your account");
  });

  it("knows whether any place would get them now", () => {
    expect(slackReaches([place({})])).toBe(false);
    expect(slackReaches([place({ account: { name: "Ada" }, allowed: false })])).toBe(false);
    expect(slackReaches([place({ account: { name: "Ada" } })])).toBe(true);
  });
});

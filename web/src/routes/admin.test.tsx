import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { Admin } from "./admin";

// No request is answered unless a test answers it: the page shows what the cache holds.
const answers = vi.hoisted(() => ({}) as Record<string, (...args: never[]) => Promise<unknown>>);
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: (_, k) => answers[String(k)] ?? (() => new Promise(() => {})) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
const search = vi.hoisted(() => ({ tab: "runners" }));
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => search,
  Link: ({ children }: { children: ReactNode }) => <a href="/admin">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

/** The Runners tab, with these runners in the cache as the API or the feed put them there. */
function runnersTab(runners: G.Runner[]) {
  search.tab = "runners";
  const client = new QueryClient();
  client.setQueryData<G.RunnerList>(keys.runners, { runners });
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Admin />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

/** A runner as control planes before workspaces send it: without `workspaces` or `yours`. */
const before = JSON.parse(
  '{"id":"run_1","name":"studio mac","status":"online","version":"0.8.0","hostname":"studio","drivers":[],"capacity":4,"active_sessions":1,"labels":[],"created_at":1760940000000}',
) as G.Runner;

const workspace: G.WorkspaceRef = {
  id: "ws_1",
  slug: "hyperstructure",
  name: "hyperstructure",
  color: "blue",
};

describe("the admin's runners", () => {
  it("lists a runner from a control plane before workspaces", () => {
    const page = runnersTab([before]);
    expect(page.textContent).toContain("studio mac");
    expect(page.textContent).toContain("1/4 sessions");
    // It doesn't say which workspaces it runs, so the page doesn't either.
    expect(page.textContent).not.toContain("Runs no workspace's sessions yet");
  });

  it("names the workspaces a runner runs, or says it runs none", () => {
    const page = runnersTab([
      { ...before, id: "run_1", name: "studio mac", workspaces: [workspace], yours: true },
      { ...before, id: "run_2", name: "spare mini", workspaces: [], yours: true },
    ]);
    const [studio, spare] = [...page.querySelectorAll("li")].map((li) => li.textContent);
    expect(studio).toContain("hyperstructure");
    expect(studio).not.toContain("Runs no workspace's sessions yet");
    expect(spare).toContain("Runs no workspace's sessions yet");
  });
});

const NOW = Date.now();
const DAY = 86_400_000;

/** The Early access tab of an instance that takes requests, with these in the cache. */
function earlyAccessTab(list: G.AccessRequestList) {
  search.tab = "early-access";
  const client = new QueryClient();
  client.setQueryData<G.InstanceInfo>(keys.instance, {
    name: "fctl",
    version: "0.11.0",
    setup_required: false,
    features: { google: false, email: false, github: false, publish_screenshots: false, early_access: true },
  });
  client.setQueryData<G.AccessRequestList>(keys.accessRequests, list);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Admin />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const button = (text: string) =>
    [...container.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);
  /** Waiting or Invited, above the list. */
  const filter = (label: string) =>
    [...container.querySelectorAll("button[aria-pressed]")].find((b) => b.textContent?.startsWith(label));
  const rows = () => [...container.querySelectorAll("li")].map((li) => li.textContent ?? "");
  const emails = () => rows().map((r) => /[a-z]+@example\.com/.exec(r)?.[0]);
  return { container, button, filter, rows, emails };
}

const asked = (email: string, days: number, fields: Partial<G.AccessRequest> = {}): G.AccessRequest => ({
  id: `acr_${email}`,
  email,
  created_at: NOW - days * DAY,
  asked_at: NOW - days * DAY,
  ...fields,
});

describe("the admin's early access list", () => {
  const list: G.AccessRequestList = {
    requests: [
      asked("old@example.com", 9),
      asked("new@example.com", 1),
      asked("grace@example.com", 5, {
        invited_at: NOW - 2 * DAY,
        invited_by: { id: "usr_maya", name: "Maya" },
        invite_expires_at: NOW + 12 * DAY,
      }),
      asked("alan@example.com", 30, { invited_at: NOW - 20 * DAY }),
    ],
    waiting: 2,
    invited: 2,
  };

  it("counts who waits on its tab, and lists them longest waiting first", () => {
    const page = earlyAccessTab(list);
    const tab = [...page.container.querySelectorAll("nav a")].find((a) =>
      a.textContent?.startsWith("Early access"),
    );
    expect(tab?.textContent).toBe("Early access2");
    expect(page.emails()).toEqual(["old@example.com", "new@example.com"]);
    expect(page.filter("Waiting")?.getAttribute("aria-pressed")).toBe("true");
  });

  it("shows those invited, and whether their link still works", () => {
    const page = earlyAccessTab(list);
    act(() => (page.filter("Invited") as HTMLButtonElement | undefined)?.click());
    expect(page.emails()).toEqual(["grace@example.com", "alan@example.com"]);
    const [grace, alan] = page.rows();
    expect(grace).toContain("by Maya");
    expect(grace).toContain("the link works until");
    expect(alan).toContain("the link expired");
    expect(page.button("Invite again")).toBeDefined();
  });

  it("makes someone's link with one click, to copy or email them", async () => {
    answers.createInvite = async (b: G.CreateInviteRequest) =>
      ({
        id: "inv_1",
        email: b.email,
        url: "https://app.fctl.sh/invite#token=fctl_inv_secret",
        role: "member",
        expires_at: NOW + 14 * DAY,
        emailed: false,
        workspaces: [],
      }) satisfies G.Invite;
    const page = earlyAccessTab(list);
    await act(async () => page.button("Invite")?.click());
    delete answers.createInvite;
    const [old] = [...page.container.querySelectorAll("li")];
    expect(old?.textContent).toContain("Invited just now");
    expect(old?.querySelector("code")?.textContent).toBe("https://app.fctl.sh/invite#token=fctl_inv_secret");
    const mail = old?.querySelector<HTMLAnchorElement>("a[href^='mailto:']");
    expect(mail?.textContent).toContain("Email it to them");
    expect(mail?.getAttribute("href")).toMatch(/^mailto:old@example\.com\?subject=/);
  });
});

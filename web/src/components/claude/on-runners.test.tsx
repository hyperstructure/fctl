import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { OnRunnersDetails, OnRunnersSetup } from "./on-runners";

const connected = { claude: [] } as unknown as G.Me;
const claudeOnRunners = vi.fn(() => Promise.resolve(connected));

// Requests other than connecting it stay unanswered: the cache holds what is shown.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return {
    api: new Proxy(
      {},
      { get: (_, k) => (k === "claudeOnRunners" ? claudeOnRunners : () => new Promise(() => {})) },
    ),
    workspaces: pending,
    ApiFailure: class extends Error {},
  };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, params }: { children: ReactNode; params: { slug: string } }) => (
    <a href={`/w/${params.slug}?tab=runners`}>{children}</a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  claudeOnRunners.mockClear();
});

const ref = (id: string, name: string): G.WorkspaceRef => ({ id, slug: id, name, color: "blue" });

const runner = (r: Partial<G.Runner> & { id: string }): G.Runner => ({
  name: r.id,
  status: "online",
  drivers: [],
  capacity: 4,
  active_sessions: 0,
  labels: [],
  created_at: 0,
  workspaces: [ref("ws_side", "Side project")],
  yours: true,
  ...r,
});

function render(node: ReactNode, runners?: G.Runner[], workspaces: G.Workspace[] = []) {
  const client = new QueryClient();
  if (runners) client.setQueryData<G.RunnerList>(keys.runners, { runners });
  client.setQueryData(keys.workspaces, { workspaces, limit: 5 });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={client}>{node}</QueryClientProvider>));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const button = (container: HTMLElement, text: string) =>
  [...container.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);

/** The rows of the runners list: name, then where it stands. */
const rows = (container: HTMLElement) =>
  [...container.querySelectorAll("li li, ul > li")]
    .filter((li) => li.querySelector(".rounded-full.size-2"))
    .map((li) => li.textContent);

describe("keeping the token on your runners", () => {
  it("says what it is, and shows the steps once asked", () => {
    const view = render(<OnRunnersSetup />, [runner({ id: "studio", claude_token: {} })]);
    expect(view.textContent).toContain("fctl never receives it");
    expect(view.textContent).not.toContain("fctl host claude set-token");
    act(() => button(view, "Set up")?.click());
    const text = view.textContent ?? "";
    for (const step of [
      "Create a token",
      "Keep it on the runner",
      "Check your runners",
      "Run your sessions on it",
    ])
      expect(text).toContain(step);
    const commands = [...view.querySelectorAll("code")].map((c) => c.textContent);
    expect(commands).toContain("claude setup-token");
    expect(commands).toContain("fctl host claude set-token");
    expect(text).toContain("None of your runners keeps it yet");
    // Nothing is done yet: each step shows its number.
    expect(view.querySelectorAll('[aria-label="Done"]')).toHaveLength(0);
  });

  it("ticks the steps on the runner off once one keeps it", () => {
    const view = render(<OnRunnersSetup />, [
      runner({ id: "spare", claude_token: {} }),
      runner({ id: "studio", claude_token: { saved_at: Date.now() - 60_000 } }),
      runner({ id: "old-box" }),
      runner({ id: "theirs", yours: false, claude_token: { saved_at: 1 } }),
    ]);
    act(() => button(view, "Set up")?.click());
    expect(view.querySelectorAll('[aria-label="Done"]')).toHaveLength(3);
    expect(view.textContent).not.toContain("None of your runners keeps it yet");
    const list = rows(view);
    expect(list).toHaveLength(3);
    expect(list[0]).toMatch(/^studioonline · saved 1 minute ago · Side projectKeeps your token$/);
    expect(list[1]).toMatch(/^spareonline · Side projectNo token yet$/);
    expect(list[2]).toContain("Update its fctl first");
  });

  it("connects it", async () => {
    const view = render(<OnRunnersSetup />, [runner({ id: "studio", claude_token: { saved_at: 1 } })]);
    act(() => button(view, "Set up")?.click());
    await act(async () => button(view, "Use the token on my runners")?.click());
    expect(claudeOnRunners).toHaveBeenCalledOnce();
    expect(queryClient.getQueryData(keys.me)).toBe(connected);
  });

  it("says where to bring a runner online when you have none", () => {
    const side = { id: "ws_side", slug: "side", name: "Side project" } as G.Workspace;
    const view = render(<OnRunnersSetup />, [], [side]);
    act(() => button(view, "Set up")?.click());
    expect(view.textContent).toContain("You have no runners yet");
    expect(view.querySelector("a")?.getAttribute("href")).toBe("/w/side?tab=runners");
  });

  it("under the connection, shows the steps at once while none of your runners keeps it", () => {
    const view = render(<OnRunnersDetails />, [runner({ id: "studio", claude_token: {} })]);
    expect(view.textContent).toContain(
      "None of your runners keeps the token yet, so sessions on this connection wait",
    );
    expect(view.textContent).toContain("fctl host claude set-token");
    const kept = render(<OnRunnersDetails />, [runner({ id: "studio", claude_token: { saved_at: 1 } })]);
    expect(kept.textContent).not.toContain("fctl host claude set-token");
    act(() => button(kept, "Keep it on another runner")?.click());
    expect(kept.textContent).toContain("fctl host claude set-token");
  });
});

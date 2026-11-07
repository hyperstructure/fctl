import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { ConnectClaude, type Way } from "./connect";

const connected = { claude: [{ id: "crd_new" }] } as unknown as G.Me;
const claudeToken = vi.fn((_: G.AddClaudeTokenRequest) => Promise.resolve(connected));
const awaiting: G.ClaudeLoginJob = {
  id: "job_1",
  created_at: 0,
  updated_at: 0,
  state: "awaiting_code",
  url: "https://claude.ai/oauth/authorize?code=true",
};
const claudeLogin = vi.fn(() => Promise.resolve(awaiting));

// Requests other than these stay unanswered: the cache holds what is shown.
vi.mock("@/lib/api", () => {
  const pending = () => new Promise(() => {});
  return {
    api: new Proxy(
      {},
      { get: (_, k) => (k === "claudeToken" ? claudeToken : k === "claudeLogin" ? claudeLogin : pending) },
    ),
    workspaces: new Proxy({}, { get: () => pending }),
    ApiFailure: class extends Error {},
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
  claudeToken.mockClear();
  claudeLogin.mockClear();
});

const connection = (c: Partial<G.ClaudeCredentialInfo>): G.ClaudeCredentialInfo => ({
  id: "crd_1",
  kind: "setup_token",
  remote_control: false,
  status: "active",
  is_default: true,
  created_at: 0,
  ...c,
});

/** The chooser, holding which way is open as the Claude section does; `ways` records each change. */
function render(claude: G.ClaudeCredentialInfo[], open: Way | null = null) {
  const ways: (Way | null)[] = [];
  function Harness() {
    const [way, setWay] = useState<Way | null>(open);
    return (
      <ConnectClaude
        me={{ claude }}
        way={way}
        onWay={(w) => {
          ways.push(w);
          setWay(w);
        }}
      />
    );
  }
  const client = new QueryClient();
  client.setQueryData<G.RunnerList>(keys.runners, { runners: [] });
  client.setQueryData(keys.workspaces, { workspaces: [], limit: 5 });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <Harness />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { view: container, ways };
}

const radio = (view: HTMLElement, value: string) =>
  view.querySelector<HTMLInputElement>(`input[type="radio"][value="${value}"]`);

const button = (view: HTMLElement, text: string) =>
  [...view.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);

/** Types into a controlled input, as a person does. */
function type(input: HTMLInputElement | null, value: string) {
  if (!input) throw new Error("no input");
  const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
  act(() => {
    set?.call(input, value);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

describe("connecting Claude", () => {
  it("recommends the long-lived token first, and says what each way gives and lacks", () => {
    const { view } = render([]);
    const cards = [...view.querySelectorAll("fieldset > div")];
    expect(cards.map((c) => c.querySelector('input[type="radio"]')?.getAttribute("value"))).toEqual([
      "token",
      "login",
    ]);
    expect(cards[0]?.textContent).toContain("Recommended");
    expect(cards[1]?.textContent).not.toContain("Recommended");
    expect(cards[0]?.textContent).toContain("Lasts a year");
    expect(cards[0]?.textContent).toContain("No Remote Control or claude.ai connectors");
    expect(cards[1]?.textContent).toContain("claude.ai connectors");
    expect(cards[1]?.textContent).toContain("sessions stop until you sign in again");
    expect(view.querySelector("h3")?.textContent).toBe("Connect your subscription");
  });

  it("opens a way once it is chosen", () => {
    const { view, ways } = render([connection({})]);
    expect(view.querySelector("h3")?.textContent).toBe("Add a connection");
    // Nothing is open until a way is chosen.
    expect(view.textContent).not.toContain("Create a token");
    act(() => radio(view, "token")?.click());
    expect(ways).toEqual(["token"]);
    expect(view.textContent).toContain("Create a token");
    expect(view.textContent).toContain("claude setup-token");
    act(() => radio(view, "login")?.click());
    expect(view.textContent).not.toContain("Create a token");
    expect(view.textContent).toContain("Start the sign-in");
  });

  it("connects a pasted token, and closes", async () => {
    const { view, ways } = render([], "token");
    const input = view.querySelector<HTMLInputElement>('input[aria-label="Long-lived token"]');
    expect(button(view, "Connect")?.disabled).toBe(true);
    expect(view.querySelectorAll('[aria-label="Done"]')).toHaveLength(0);
    type(input, "  sk-ant-oat01-abc  ");
    // Having a token to paste means step 1 is done.
    expect(view.querySelectorAll('[aria-label="Done"]')).toHaveLength(1);
    await act(async () => button(view, "Connect")?.click());
    expect(claudeToken).toHaveBeenCalledWith({ token: "sk-ant-oat01-abc" });
    expect(queryClient.getQueryData(keys.me)).toBe(connected);
    expect(ways.at(-1)).toBeNull();
  });

  it("keeps the token on your runners instead, unless you do already", () => {
    const { view } = render([], "token");
    expect(view.textContent).not.toContain("fctl host claude set-token");
    act(() => radio(view, "runners")?.click());
    expect(view.textContent).toContain("fctl never has it");
    expect(view.textContent).toContain("fctl host claude set-token");
    expect(button(view, "Use the token on my runners")).toBeDefined();

    const kept = render([connection({ on_runners: true })], "token").view;
    expect(radio(kept, "runners")).toBeNull();
    expect(kept.querySelector('input[aria-label="Long-lived token"]')).not.toBeNull();
  });

  it("signs in with Claude in steps", async () => {
    const { view } = render([], "login");
    expect(view.textContent).toContain("The link shows here once Claude Code is ready");
    await act(async () => button(view, "Start the sign-in")?.click());
    expect(claudeLogin).toHaveBeenCalledOnce();
    const link = view.querySelector<HTMLAnchorElement>(`a[href="${awaiting.url}"]`);
    expect(link?.textContent).toContain("Open claude.ai");
    expect(view.querySelectorAll('[aria-label="Done"]')).toHaveLength(1);
    const code = view.querySelector<HTMLInputElement>('input[aria-label="Code from claude.ai"]');
    expect(button(view, "Finish")?.disabled).toBe(true);
    type(code, "abc#def");
    expect(button(view, "Finish")?.disabled).toBe(false);
  });

  it("keeps a sign-in under way while you look at the token", async () => {
    const { view } = render([], "login");
    await act(async () => button(view, "Start the sign-in")?.click());
    act(() => radio(view, "token")?.click());
    act(() => radio(view, "login")?.click());
    expect(view.querySelector(`a[href="${awaiting.url}"]`)).not.toBeNull();
  });
});

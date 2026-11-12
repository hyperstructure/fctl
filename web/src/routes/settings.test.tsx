import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { clearLog, record } from "@/lib/app-log";
import { keys, queryClient } from "@/lib/queries";
import { SettingsPage } from "./settings";

const route = vi.hoisted(() => ({
  search: {} as Record<string, string | undefined>,
  hash: "",
}));

// No request is answered: the page shows what the cache holds.
vi.mock("@/lib/api", async (real) => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return {
    ...(await real<typeof import("@/lib/api")>()),
    api: pending,
    emails: pending,
    workspaces: pending,
  };
});
vi.mock("@tanstack/react-router", () => ({
  useSearch: () => route.search,
  useLocation: ({ select }: { select: (l: { hash: string }) => unknown }) => select({ hash: route.hash }),
  useNavigate: () => vi.fn(),
  Link: ({
    to,
    search,
    children,
    className,
    ...rest
  }: {
    to: string;
    search?: { tab?: string };
    children: ReactNode;
    className?: string;
  }) => (
    <a href={search?.tab ? `${to}?tab=${search.tab}` : to} className={className} {...rest}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  route.search = {};
  route.hash = "";
});

const me = (fields: Partial<G.Me> = {}): G.Me => ({
  user: {
    id: "usr_ada",
    email: "ada@example.com",
    name: "Ada Lovelace",
    given_name: "Ada",
    family_name: "Lovelace",
    role: "member",
    status: "active",
    has_password: true,
    two_factor: false,
    passkeys: 0,
    created_at: 0,
    emails: [],
    workspaces: 0,
  },
  identities: [],
  claude: [
    {
      id: "crd_1",
      kind: "setup_token",
      label: "Personal",
      remote_control: false,
      status: "active",
      is_default: true,
      created_at: 0,
    },
  ],
  two_factor: {
    recovery_codes: 0,
    google: false,
    google_required: false,
    required: false,
    setup_required: false,
    on: false,
  },
  passkeys: [],
  emails: [{ email: "ada@example.com", primary: true, workspaces: [], added_at: 0 }],
  claude_stand_in: false,
  ...fields,
});

/** Settings at the address `route` has, for `who`, on an instance with Linear or without. */
function settings(who = me(), { linear = true } = {}) {
  queryClient.setQueryData<G.Me>(keys.me, who);
  queryClient.setQueryData<G.InstanceInfo>(keys.instance, {
    name: "fctl",
    version: "0.32.0",
    setup_required: false,
    features: {
      google: true,
      email: false,
      github: false,
      publish_screenshots: false,
      early_access: false,
      linear,
    },
  });
  queryClient.setQueryData<G.WorkspaceList>(keys.workspaces, { workspaces: [], invitations: [], limit: 5 });
  queryClient.setQueryData<G.TokenList>(keys.tokens, { tokens: [] });
  queryClient.setQueryData<G.MyLinear>(keys.myLinear, { configured: true, accounts: [] });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <SettingsPage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const headings = () => [...container.querySelectorAll("h2, h3")].map((h) => h.textContent);
  /** Where the page's links to its pages go, in the navigation beside it. */
  const pages = () => {
    const nav = container.querySelector('nav[aria-label="Settings"]');
    return [...(nav?.querySelectorAll("a") ?? [])].map((a) => a.getAttribute("href"));
  };
  return { container, headings, pages };
}

describe("Settings", () => {
  it("lists its pages, and opens Profile beside them", () => {
    const { headings, pages } = settings();
    expect(pages()).toEqual([
      "/settings?tab=profile",
      "/settings?tab=emails",
      "/settings?tab=workspaces",
      "/settings?tab=claude",
      "/settings?tab=linear",
      "/settings?tab=notifications",
      "/settings?tab=appearance",
      "/settings?tab=log",
      "/settings?tab=security",
      "/settings?tab=tokens",
      "/settings?tab=data",
    ]);
    expect(headings()).toEqual(["Profile"]);
  });

  it("opens the page its address names, and only that page", () => {
    route.search = { tab: "security" };
    const { headings, container } = settings();
    expect(headings()).toEqual(["Security", "Sign-in methods", "Passkeys", "Two-factor sign-in", "Devices"]);
    expect(container.querySelector('[aria-current="page"]')?.textContent).toBe("Security");
    // The way back to the list of pages, for a phone.
    expect(container.querySelector('a[href="/settings"]')?.textContent).toBe("Settings");
  });

  it("shows the app's log in this browser, to copy", () => {
    record("Went to the background", 1);
    route.search = { tab: "log" };
    const { headings, container } = settings();
    expect(headings()).toEqual(["App log"]);
    expect(container.querySelector("pre")?.textContent).toContain("Went to the background");
    clearLog();
  });

  it("opens the page with the section an older link named", () => {
    route.hash = "#claude";
    expect(settings().headings()).toEqual(["Claude", "Add a connection"]);
  });

  it("opens Security for the way back from linking Google, with how it went", () => {
    route.search = { linked: "google" };
    const { headings, container } = settings();
    expect(headings()[0]).toBe("Security");
    expect(container.textContent).toContain("Google is linked");
  });

  it("leaves Linear out where the instance has no Linear app", () => {
    route.search = { tab: "linear" };
    const { headings, pages } = settings(me(), { linear: false });
    expect(pages()).not.toContain("/settings?tab=linear");
    expect(headings()).toEqual(["Profile"]);
  });

  it("marks the page that needs you", () => {
    const { container } = settings(me({ claude: [] }));
    const claude = container.querySelector('nav[aria-label="Settings"] a[href="/settings?tab=claude"]');
    expect(claude?.textContent).toBe("Claude: Not connected yet");
    const security = container.querySelector('nav[aria-label="Settings"] a[href="/settings?tab=security"]');
    expect(security?.textContent).toBe("Security");
  });

  it("says on a phone what each page holds now", () => {
    const { container } = settings();
    const list = [...container.querySelectorAll('nav[aria-label="Settings"]')][1];
    const row = (tab: string) => list?.querySelector(`a[href="/settings?tab=${tab}"]`)?.textContent;
    // After the initials her picture would be.
    expect(row("profile")).toBe("ALAda LovelaceProfile · ada@example.com");
    expect(row("claude")).toBe("ClaudePersonal");
    expect(row("security")).toBe("SecurityTwo-factor sign-in off");
    expect(row("tokens")).toBe("Access tokensNone yet");
  });

  it("changes the password in a dialog, rather than in fields always open", () => {
    route.search = { tab: "security" };
    const { container } = settings();
    expect(document.querySelector('input[autocomplete="current-password"]')).toBeNull();
    const change = [...container.querySelectorAll("button")].find((b) => b.textContent === "Change password");
    act(() => change?.click());
    const dialog = document.querySelector("dialog[open]");
    expect(dialog?.textContent).toContain("Change your password");
    expect(dialog?.querySelector('input[autocomplete="current-password"]')).not.toBeNull();
    expect(dialog?.querySelector('input[autocomplete="new-password"]')).not.toBeNull();
  });
});

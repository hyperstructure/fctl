import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { WorkspacePage } from "@/routes/workspace";

const route = vi.hoisted(() => ({ tab: "people" as string | undefined }));
const claude = vi.hoisted(() => vi.fn());
const update = vi.hoisted(() => vi.fn());

// Only what the test answers is answered: the page shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  const answered: Record<string | symbol, unknown> = { claude, update };
  const workspaces = new Proxy({}, { get: (_, k) => answered[k] ?? (() => new Promise(() => {})) });
  return { api: pending, workspaces, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ slug: "acme" }),
  useSearch: () => ({ tab: route.tab }),
  useNavigate: () => () => {},
  Link: ({ children, to }: { children: ReactNode; to: string }) => <a href={to}>{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  claude.mockReset();
  update.mockReset();
  route.tab = "people";
});

const HOUR = 3_600_000;
const DAY = 24 * HOUR;

const settings = (s: Partial<G.ClaudeOversightSettings> = {}): G.ClaudeOversightSettings => ({
  oversight: "details",
  most: "details",
  limit_pauses: true,
  limit_pauses_allowed: true,
  admin_email: "weekly",
  ...s,
});

const acme = (w: Partial<G.Workspace> = {}): G.Workspace => ({
  id: "ws_acme",
  slug: "acme",
  name: "Acme",
  color: "blue",
  role: "admin",
  email: "ada@acme.dev",
  base_role: "editor",
  require_two_factor: false,
  instance_requires_two_factor: false,
  blocked: false,
  primary: { id: "usr_ada", name: "Ada Lovelace" },
  members: 3,
  projects: 1,
  runners: 1,
  runners_online: 1,
  github: [],
  skip_invites: false,
  own_runners: false,
  autopilots: false,
  claude_oversight: settings(),
  created_at: 0,
  ...w,
});

const user = (name: string) => ({ id: `usr_${name.split(" ")[0]?.toLowerCase()}`, name });
const member = (name: string, role: G.WorkspaceRole = "member"): G.WorkspaceMember => ({
  user: user(name),
  email: `${name.split(" ")[0]?.toLowerCase()}@acme.dev`,
  role,
  joined_at: Date.now() - 90 * DAY,
});
const members: G.WorkspaceMemberList = {
  members: [member("Ada Lovelace", "owner"), member("Grace Hopper"), member("Linus Lee")],
  invitations: [
    {
      id: "wsi_1",
      workspace: { id: "ws_acme", slug: "acme", name: "Acme", color: "blue" },
      email: "margaret@acme.dev",
      role: "member",
      created_at: Date.now() - DAY,
      expires_at: Date.now() + 13 * DAY,
      needs_two_factor: false,
    },
  ],
};

const facts = (name: string, m: Partial<G.MemberClaude>): G.MemberClaude => ({
  user: user(name),
  email: `${name.split(" ")[0]?.toLowerCase()}@acme.dev`,
  state: "works",
  ...m,
});

const ada = facts("Ada Lovelace", {
  connection: {
    kind: "login",
    plan: "max",
    connected_at: Date.now() - 40 * DAY,
    answered_at: Date.now() - 300_000,
  },
  pauses: { week: 0, month: 0 },
});
const grace = facts("Grace Hopper", {
  runs_out_at: Date.now() + 21 * DAY,
  runs_out_estimated: true,
  connection: {
    kind: "token",
    set_by_admin_token: true,
    plan: "team",
    connected_at: Date.now() - 344 * DAY,
    answered_at: Date.now() - 2 * HOUR,
  },
  pauses: { week: 6, month: 9, last_at: Date.now() - DAY, last_limit: "seven_day" },
});
const linus = facts("Linus Lee", { state: "none" });

const overview = (o: Partial<G.ClaudeOverview> = {}): G.ClaudeOverview => ({
  oversight: "details",
  limit_pauses: true,
  admin_tokens: false,
  members: [ada, grace, linus],
  ...o,
});

/** Acme's People tab for Ada, with what its admins see of Claude (undefined: it hasn't come yet). */
function people(w: G.Workspace, data?: G.ClaudeOverview) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, staleTime: Infinity } } });
  client.setQueryData(keys.workspace(w.slug), w);
  client.setQueryData(keys.me, { user: { id: "usr_ada", name: "Ada Lovelace", role: "member" }, claude: [] });
  client.setQueryData(keys.workspaceMembers(w.id), members);
  if (data) client.setQueryData(keys.workspaceClaude(w.id), data);
  else if (!claude.getMockImplementation()) claude.mockReturnValue(new Promise(() => {}));
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <WorkspacePage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const row = (page: HTMLElement, name: string) =>
  [...page.querySelectorAll("li")].find((li) => li.textContent?.includes(name));
const pill = (page: HTMLElement, label: string) =>
  [...page.querySelectorAll<HTMLButtonElement>("button[aria-pressed]")].find((b) =>
    b.textContent?.startsWith(label),
  );
const factsOf = (page: HTMLElement, name: string) =>
  row(page, name)?.querySelector<HTMLButtonElement>("button[aria-expanded]");

describe("Claude on a workspace's People tab", () => {
  it("tells its admins who can run sessions, under each person, and sums it up", () => {
    const page = people(acme(), overview());
    expect(page.textContent).toContain(
      "2 of 3 people have Claude connected for Acme · 1 token runs out within a month",
    );
    expect(factsOf(page, "Grace Hopper")?.textContent).toContain(
      "Long-lived token, set by an admin token · Team · Claude answered 2 hours ago · runs out around",
    );
    expect(factsOf(page, "Ada Lovelace")?.textContent).toContain(
      "Claude login · Max · Claude answered 5 minutes ago",
    );
    const none = factsOf(page, "Linus Lee");
    expect(none?.textContent).toContain("No Claude connection for Acme: their sessions here can't start");
    expect(none?.querySelector(".text-amber")).not.toBeNull();
    // Where seats are given, and never a token.
    expect(page.textContent).toContain(
      "Claude Team and Enterprise seats are given in Claude's organization settings",
    );
    expect(page.textContent).not.toContain("sk-ant");
  });

  it("narrows the list with its pills, which count, and leaves out those nobody is under", () => {
    const page = people(acme(), overview());
    expect([...page.querySelectorAll("button[aria-pressed]")].map((b) => b.textContent)).toEqual([
      "Everyone 3",
      "No Claude 1",
      "Runs out soon 1",
      "Pausing on limits 1",
    ]);
    act(() => pill(page, "No Claude")?.click());
    expect(pill(page, "No Claude")?.getAttribute("aria-pressed")).toBe("true");
    expect(row(page, "Linus Lee")).toBeDefined();
    expect(row(page, "Grace Hopper")).toBeUndefined();
    // Invitations aren't anyone's connection.
    expect(page.textContent).not.toContain("margaret@acme.dev");
    act(() => pill(page, "Everyone")?.click());
    expect(row(page, "Grace Hopper")).toBeDefined();
    expect(page.textContent).toContain("margaret@acme.dev");
  });

  it("opens a person in place, with the whole of it and the way out", () => {
    const page = people(acme(), overview());
    const button = factsOf(page, "Grace Hopper");
    expect(button?.getAttribute("aria-expanded")).toBe("false");
    act(() => button?.click());
    expect(factsOf(page, "Grace Hopper")?.getAttribute("aria-expanded")).toBe("true");
    const opened = page.querySelector(`#${factsOf(page, "Grace Hopper")?.getAttribute("aria-controls")}`);
    expect(opened?.textContent).toContain(
      "Grace Hopper's sessions in Acme run on a long-lived token an admin token set on",
    );
    expect(opened?.textContent).toContain("fctl can't read the date from the token.");
    expect(opened?.textContent).toContain(
      "Their plan's limits are short for this work: a Max or Ultra seat has more.",
    );
    // Where the instance takes no admin tokens, they connect a new one themselves.
    expect(opened?.textContent).toContain(
      "Grace Hopper connects a new token in Settings → Claude: fctl has told them too.",
    );
    expect(opened?.textContent).toContain("Copy the link to their Settings → Claude");
    act(() => factsOf(page, "Grace Hopper")?.click());
    expect(page.textContent).not.toContain("fctl can't read the date from the token.");
  });

  it("gives the command that sets a new token where the instance takes admin tokens", () => {
    const page = people(acme(), overview({ admin_tokens: true }));
    act(() => factsOf(page, "Linus Lee")?.click());
    expect(page.querySelector("code")?.textContent).toBe(
      'fctl admin users set linus@acme.dev --claude-token - --claude-label "Acme seat"',
    );
    expect(page.textContent).toContain(
      "Set them a new one with an admin token, from a token claude setup-token printed:",
    );
  });

  it("shows the facts lines' shape while they load, never 'No Claude connection' first", () => {
    const page = people(acme());
    expect(page.textContent).not.toContain("No Claude connection");
    expect(row(page, "Linus Lee")?.querySelector(".animate-shimmer")).not.toBeNull();
    expect(page.textContent).toContain("Looking up Claude connections");
  });

  it("keeps the people listed when it didn't load, and says why, with Try again", async () => {
    claude.mockRejectedValue(new Error("The server took too long"));
    const page = people(acme());
    for (let i = 0; i < 20 && !page.textContent?.includes("didn't load"); i++)
      await act(async () => {
        await new Promise((r) => setTimeout(r, 0));
      });
    expect(page.textContent).toContain("Claude connections didn't load: The server took too long");
    expect([...page.querySelectorAll("button")].some((b) => b.textContent === "Try again")).toBe(true);
    expect(row(page, "Grace Hopper")).toBeDefined();
  });

  it("introduces it while nobody but you has Claude connected, with two made-up rows", () => {
    const page = people(
      acme(),
      overview({ members: [ada, facts("Grace Hopper", { state: "none" }), linus] }),
    );
    expect(page.textContent).toContain("Claude for everyone in Acme");
    expect(page.textContent).toContain(
      "Acme's sessions run on each person's own Claude subscription: 1 of its 3 people has connected one. You see whether their sessions can run and when their tokens run out, never the tokens.",
    );
    const preview = page.querySelector('[aria-hidden="true"].rounded-xl');
    expect(preview?.textContent).toContain("Grace Hopper");
  });

  it("says in one quiet line that its admins see nothing, and where that's decided", () => {
    const page = people(acme({ claude_oversight: settings({ oversight: "off" }) }));
    expect(page.textContent).toContain(
      "Acme's admins see nothing of its members' Claude connections. Change that in Settings.",
    );
    expect(claude).not.toHaveBeenCalled();
    const instance = people(acme({ claude_oversight: settings({ oversight: "off", most: "off" }) }));
    expect(instance.textContent).toContain(
      "This instance shows workspace admins nothing of their members' Claude connections.",
    );
  });

  it("shows members none of it about others", () => {
    const page = people(acme({ role: "member" }), overview());
    expect(page.textContent).not.toContain("Claude login");
    expect(page.textContent).not.toContain("have Claude connected");
    expect(claude).not.toHaveBeenCalled();
  });
});

describe("Claude in a workspace's settings", () => {
  it("tells each member what its admins see, and never the token", () => {
    route.tab = "settings";
    const page = people(acme({ role: "member" }));
    expect(page.textContent).toContain(
      "Acme's admins see what kind of Claude connection your sessions here run on, its plan, when Claude last answered it and when it runs out, and how often your sessions paused on usage limits: never the token or the account.",
    );
    route.tab = "settings";
    const ready = people(acme({ role: "member", claude_oversight: settings({ oversight: "readiness" }) }));
    expect(ready.textContent).toContain("Acme's admins see whether your sessions here can run on Claude");
  });

  it("lets its admins choose what they see, no more than the instance lets them, and says who decides that", () => {
    route.tab = "settings";
    const page = people(
      acme({
        claude_oversight: settings({
          oversight: "readiness",
          most: "readiness",
          limit_pauses_allowed: false,
        }),
      }),
    );
    expect(page.textContent).toContain(
      "This instance lets workspace admins see whether sessions can run at most, as its admins decided.",
    );
    const pauses = page.querySelector<HTMLButtonElement>('[role="switch"][aria-label="Limit pauses"]');
    expect(pauses?.disabled).toBe(true);
    expect(page.textContent).toContain(
      "This instance doesn't show workspace admins their members' limit pauses",
    );
  });

  it("saves how admins hear by email, and says nothing goes where the instance sends none", async () => {
    route.tab = "settings";
    update.mockResolvedValue(acme({ claude_oversight: settings({ admin_email: "each" }) }));
    const page = people(acme());
    expect(page.textContent).toContain("This instance sends no email, so nothing goes whatever this says");
    const label = [...page.querySelectorAll("label")].find((l) => l.textContent === "Email admins");
    const trigger = document.getElementById(label?.htmlFor ?? "");
    act(() => trigger?.click());
    const each = [...document.querySelectorAll<HTMLElement>("[role=option]")].find((o) =>
      o.textContent?.startsWith("Each time"),
    );
    await act(async () => each?.click());
    expect(update).toHaveBeenCalledWith("ws_acme", { claude_admin_email: "each" });
  });
});

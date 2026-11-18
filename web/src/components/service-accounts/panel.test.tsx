import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { InvitationServiceAccounts, ServiceAccountAsks } from "./ask";
import { ServiceAccountPage } from "./page";
import { ServiceAccountsPanel } from "./panel";

const { createServiceAccount, answerServiceAccount } = vi.hoisted(() => ({
  createServiceAccount: vi.fn(),
  answerServiceAccount: vi.fn(),
}));
// Only making one and answering one are answered; other requests wait, and what shows is what the
// cache holds.
vi.mock("@/lib/api", () => ({
  api: new Proxy({} as Record<string, unknown>, {
    get: (_, k) =>
      k === "createServiceAccount"
        ? createServiceAccount
        : k === "answerServiceAccount"
          ? answerServiceAccount
          : () => new Promise(() => {}),
  }),
  ApiFailure: class extends Error {},
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to, search }: { children: ReactNode; to: string; search?: Record<string, string> }) => (
    <a href={`${to}?${new URLSearchParams(search)}`}>{children}</a>
  ),
}));
// Code shows as it's written: highlighting needs Shiki's workers.
vi.mock("@/components/markdown", () => ({
  CodeBlock: ({ code }: { code: string }) => <pre>{code}</pre>,
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
const text = (c: HTMLElement) => c.textContent ?? "";
const button = (c: HTMLElement, name: string) =>
  [...c.querySelectorAll("button")].find((b) => b.textContent?.trim() === name) as HTMLButtonElement;

const ref = { id: "wsp_1", slug: "acme", name: "Acme", color: "yellow" } as G.WorkspaceRef;
const workspace = (role: G.WorkspaceRole = "owner") =>
  ({ ...ref, role, email: "ada@acme.dev", members: 12, service_accounts: [] }) as unknown as G.Workspace;

const account = (over: Partial<G.ServiceAccount> = {}): G.ServiceAccount => ({
  id: "sva_portal",
  workspace: ref,
  name: "Acme portal",
  description: "Our agent portal at portal.acme.dev",
  created_by: { id: "usr_ada", name: "Ada Lovelace" } as G.UserRef,
  created_at: Date.now() - 86_400_000,
  acts_for_you: false,
  consent: true,
  can_manage: false,
  ...over,
});

const admins = (over: Partial<G.ServiceAccount> = {}) =>
  account({
    can_manage: true,
    keeper: { id: "usr_ada", name: "Ada Lovelace" } as G.UserRef,
    secret_hint: "x8Kq",
    secret_made_at: Date.now() - 86_400_000,
    people: { allowed: 4, declined: 1, unanswered: 7 },
    sessions: 23,
    last_used_at: Date.now() - 300_000,
    ...over,
  });

function seed(accounts: G.ServiceAccount[], consent = true) {
  queryClient.setQueryData(keys.projects, { projects: [{ id: "prj_web", slug: "web", workspace: ref }] });
  queryClient.setQueryData(keys.serviceAccounts("wsp_1"), {
    service_accounts: accounts,
    consent,
    limit: 10,
  } satisfies G.ServiceAccountList);
}

describe("a workspace's service accounts, for its admins", () => {
  it("says what one does before there are any, and that people choose", async () => {
    seed([]);
    const c = render(<ServiceAccountsPanel workspace={workspace()} onOpen={() => {}} />);
    await settle();
    expect(c.querySelector("h2")?.textContent).toBe("Agents started from your own tools");
    expect(text(c)).toContain("People choose: it starts sessions only for those who allowed it.");
    expect(text(c)).toContain("If they leave, it stops until another admin makes it a new one.");
    expect(button(c, "Make a service account")).toBeTruthy();
  });

  it("says so where the instance doesn't ask people", async () => {
    seed([], false);
    const c = render(<ServiceAccountsPanel workspace={workspace()} onOpen={() => {}} />);
    await settle();
    expect(text(c)).toContain("This instance doesn't ask people: it starts sessions for anyone in Acme");
  });

  it("lists each with how its people answered, and why one stopped", async () => {
    seed([
      admins(),
      admins({
        id: "sva_bot",
        name: "Slack bot",
        description: undefined,
        stopped: "Grace Hopper, who made its secret, is no longer an admin of Acme.",
        last_used_at: undefined,
        sessions: 0,
      }),
    ]);
    const open = vi.fn();
    const c = render(<ServiceAccountsPanel workspace={workspace()} onOpen={open} />);
    await settle();
    expect(text(c)).toContain("2 service accounts · 1 stopped");
    expect(text(c)).toContain("4 of 12 allowed it · 1 didn't · 7 haven't answered · 23 sessions started");
    expect(text(c)).toContain("Stopped: Grace Hopper, who made its secret, is no longer an admin of Acme.");
    expect(text(c)).toContain("not used yet");
    act(() => [...c.querySelectorAll("button")].find((b) => b.textContent?.includes("Acme portal"))?.click());
    expect(open).toHaveBeenCalledWith("sva_portal");
  });

  it("makes one, shows its secret once with how a program uses it, and that fctl asks its people", async () => {
    seed([]);
    createServiceAccount.mockResolvedValue({
      service_account: admins({
        people: { allowed: 0, declined: 0, unanswered: 12 },
        last_used_at: undefined,
      }),
      secret: "fctl_svc_s3cr3tx8Kq",
    } satisfies G.CreatedServiceAccount);
    const c = render(<ServiceAccountsPanel workspace={workspace()} onOpen={() => {}} />);
    await settle();
    act(() => button(c, "Make a service account").click());
    const name = c.querySelector<HTMLInputElement>('input[aria-label="Name"]');
    expect(name).toBeTruthy();
    if (!name) return;
    act(() => {
      const set = Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set;
      set?.call(name, "Acme portal");
      name.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(text(c)).toContain("Then fctl asks Acme's people to let it start sessions for them.");
    await act(async () => button(c, "Make service account").click());
    await settle();
    expect(createServiceAccount).toHaveBeenCalledWith("wsp_1", {
      name: "Acme portal",
      description: undefined,
    });
    expect(text(c)).toContain("Made Acme portal");
    expect(text(c)).toContain("fctl_svc_s3cr3tx8Kq");
    expect(text(c)).toContain("it shows once, here");
    expect(text(c)).toContain("fctl asks Acme's 12 people to let it start sessions for them");
    // How a program uses it, with this secret, for someone named in the request.
    expect(text(c)).toContain("export FCTL_TOKEN=fctl_svc_s3cr3tx8Kq");
    expect(text(c)).toContain("fctl run -p web --on-behalf-of ada@acme.dev");
  });
});

describe("a service account's page", () => {
  it("says whom it acts for, and lets you answer for yourself alone", async () => {
    const a = admins();
    queryClient.setQueryData(keys.serviceAccount(a.id), a);
    queryClient.setQueryData(keys.me, { user: { id: "usr_ada", name: "Ada Lovelace" } });
    queryClient.setQueryData(keys.serviceAccountSessions(a.id), { sessions: [] });
    queryClient.setQueryData(keys.serviceAccountPeople(a.id), {
      consent: true,
      people: [
        {
          user: { id: "usr_ada", name: "Ada Lovelace" },
          email: "ada@acme.dev",
          role: "owner",
          acts_for: false,
          blocked: false,
          why_not: "Ada Lovelace hasn't allowed Acme portal to start sessions for them yet",
        },
        {
          user: { id: "usr_bob", name: "Bob" },
          email: "bob@acme.dev",
          role: "member",
          answer: "allowed",
          answered_at: Date.now(),
          acts_for: true,
          blocked: false,
        },
        {
          user: { id: "usr_cy", name: "Cy" },
          email: "cy@acme.dev",
          role: "member",
          acts_for: false,
          blocked: true,
          why_not: "Cy's account is disabled",
        },
      ],
    } satisfies G.ServiceAccountPeople);
    const c = render(<ServiceAccountPage id={a.id} workspace={workspace()} onBack={() => {}} />);
    await settle();
    expect(text(c)).toContain("Who it acts for");
    expect(text(c)).toContain("Nobody answers for anyone else.");
    expect(text(c)).toContain("Allowed it");
    expect(text(c)).toContain("Cy's account is disabled");
    // Yours, which you answer here.
    expect(button(c, "Allow")).toBeTruthy();
    expect(text(c)).toContain("None yet. The sessions it starts show here as it starts them.");
    expect(text(c)).toContain("x8Kq");
    expect(text(c)).toContain("it works while Ada is an admin of Acme");
  });

  it("says why it stopped, with the way out beside it", async () => {
    const a = admins({ stopped: "Grace Hopper, who made its secret, is no longer an admin of Acme." });
    queryClient.setQueryData(keys.serviceAccount(a.id), a);
    const c = render(<ServiceAccountPage id={a.id} workspace={workspace()} onBack={() => {}} />);
    await settle();
    expect(text(c)).toContain(
      "It stopped: Grace Hopper, who made its secret, is no longer an admin of Acme.",
    );
    expect(button(c, "Make a new secret")).toBeTruthy();
  });
});

describe("a workspace's service accounts, for its members", () => {
  it("lists them with your answer, which you change", async () => {
    seed([
      account(),
      account({
        id: "sva_bot",
        name: "Slack bot",
        answer: "allowed",
        acts_for_you: true,
        answered_at: Date.now(),
      }),
    ]);
    const c = render(<ServiceAccountsPanel workspace={workspace("member")} onOpen={() => {}} />);
    await settle();
    expect(text(c)).toContain("Each starts sessions for you only if you allow it");
    expect(text(c)).toContain("Asks to start sessions for you");
    expect(text(c)).toContain("Starts sessions for you");
    expect(c.querySelector('[role="switch"][aria-checked="true"]')).toBeTruthy();
    expect(text(c)).not.toContain("Make a service account");
  });
});

describe("what service accounts ask you", () => {
  it("asks on the Agent Floor, saying what allowing it means, and takes the answer", async () => {
    answerServiceAccount.mockResolvedValue(account({ answer: "allowed", acts_for_you: true }));
    const c = render(<ServiceAccountAsks asks={[{ workspace: workspace("member"), account: account() }]} />);
    expect(text(c)).toContain("Acme portal asks to start agents for you in Acme");
    expect(text(c)).toContain("“Our agent portal at portal.acme.dev”");
    expect(text(c)).toContain("they're yours, on your Claude subscription");
    expect(text(c)).toContain("Nothing else: not your account, your settings or other workspaces.");
    await act(async () => button(c, "Allow").click());
    await settle();
    expect(answerServiceAccount).toHaveBeenCalledWith("sva_portal", true);
  });

  it("warns on an invitation what joining means, as the instance decides", () => {
    const invitation = {
      workspace: ref,
      service_accounts: [{ id: "sva_portal", name: "Acme portal" }],
    } as G.WorkspaceInvitation;
    const asked = render(<InvitationServiceAccounts invitation={invitation} consent />);
    expect(text(asked)).toContain("Acme has a service account, Acme portal");
    expect(text(asked)).toContain(
      "It starts them for you only if you allow it: fctl asks you once you join.",
    );
    const unasked = render(<InvitationServiceAccounts invitation={invitation} consent={false} />);
    expect(text(unasked)).toContain(
      "This instance doesn't ask people: once you join, it starts them for you too.",
    );
    const none = render(
      <InvitationServiceAccounts invitation={{ ...invitation, service_accounts: [] }} consent />,
    );
    expect(text(none)).toBe("");
  });
});

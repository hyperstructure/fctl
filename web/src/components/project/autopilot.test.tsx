import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { AutopilotTab } from "./autopilot";

const { answer, autopilot, toast } = vi.hoisted(() => ({
  answer: vi.fn(),
  autopilot: vi.fn(),
  toast: vi.fn(),
}));
// The autopilot and answering it are answered; other requests wait.
vi.mock("@/lib/api", () => ({
  api: new Proxy({} as Record<string, unknown>, {
    get: (_, k) =>
      k === "autopilot" ? autopilot : k === "answerAutopilot" ? answer : () => new Promise(() => {}),
  }),
  ApiFailure: class extends Error {
    status: number;
    constructor(status: number) {
      super("Not found");
      this.status = status;
    }
  },
  workspaces: { get: () => new Promise(() => {}) },
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
  useNavigate: () => vi.fn(),
}));
vi.mock("@/components/ui", async (actual) => ({
  ...(await actual<typeof import("@/components/ui")>()),
  toast,
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

const project = (more: Partial<G.Project> = {}): G.Project => ({
  id: "prj_1",
  slug: "shop",
  name: "Shop",
  default_branch: "main",
  settings: {
    auto_pull_request: true,
    remote_control: true,
    claude_ai_connectors: true,
    credit_people: true,
    credit_claude: true,
    publish_screenshots: false,
    package_caches: false,
    project_memory: true,
    retrospectives: true,
    env: {},
  },
  role: "maintainer",
  created_by: { id: "usr_ada", name: "Ada" },
  created_at: 0,
  updated_at: 0,
  active_sessions: 0,
  total_sessions: 3,
  autopilot: { allowed: true, asks: 0, working: false },
  ...more,
});

const on = (more: Partial<G.Autopilot> = {}): G.Autopilot => ({
  project_id: "prj_1",
  project_slug: "shop",
  scope: "project",
  state: "on",
  owner: { id: "usr_ada", name: "Ada" },
  direction: "Ship the **checkout**.",
  direction_at: Date.now() - 3_600_000,
  powers: ["settings"],
  check_in_minutes: 60,
  crew_max: 3,
  wake_at: Date.now() + 1_500_000,
  wake_reason: "check_in",
  check_ins: 2,
  notes: [],
  asks: [],
  crew: [],
  crew_total: 0,
  can: { steer: true, secrets: true, read: true },
  created_at: 0,
  updated_at: 0,
  ...more,
});

describe("a project's Autopilot tab", () => {
  it("says what an autopilot does, and offers directions, before the project is on autopilot", async () => {
    autopilot.mockResolvedValue(null);
    queryClient.setQueryData(keys.autopilot("prj_1"), null);
    const page = render(<AutopilotTab project={project()} />);
    await settle();
    expect(page.textContent).toContain("A project that runs itself");
    expect(page.textContent).toContain("Experimental");
    expect(page.textContent).toContain("Build a product from a brief");
    expect(page.textContent).toContain("Write your own");
  });

  it("tells people who can't put it on who can", async () => {
    queryClient.setQueryData(keys.autopilot("prj_1"), null);
    autopilot.mockResolvedValue(null);
    const page = render(<AutopilotTab project={project({ role: "editor" })} />);
    await settle();
    expect(page.textContent).toContain("The project's maintainers and owners put it on autopilot");
    expect(page.textContent).not.toContain("Write your own");
  });

  it("says where a workspace's admins let projects have one, before they do", async () => {
    queryClient.setQueryData(keys.autopilot("prj_1"), null);
    autopilot.mockResolvedValue(null);
    const page = render(
      <AutopilotTab
        project={project({
          autopilot: { allowed: false, state: "off", asks: 0, working: false },
          workspace: { id: "wsp_1", slug: "acme", name: "Acme", color: "blue" },
        })}
      />,
    );
    await settle();
    expect(page.textContent).toContain(
      "Acme's admins let projects have autopilots in the workspace's settings",
    );
  });

  it("shows what it asks, and sends the answer clicked", async () => {
    const asked = on({
      asks: [
        {
          id: "apa_1",
          kind: "question",
          text: "Which payment provider?",
          detail: "Stripe has **Apple Pay** built in.",
          options: ["Stripe", "Adyen"],
          status: "open",
          created_at: Date.now(),
        },
        {
          id: "apa_2",
          kind: "secret",
          text: "To charge cards in tests",
          secret: { name: "STRIPE_KEY", project_id: "prj_1", project_slug: "shop", exists: false },
          status: "open",
          created_at: Date.now(),
        },
      ],
    });
    autopilot.mockResolvedValue(asked);
    answer.mockResolvedValue(on());
    queryClient.setQueryData(keys.autopilot("prj_1"), asked);
    const page = render(<AutopilotTab project={project()} />);
    await settle();
    expect(page.textContent).toContain("Waiting for you");
    expect(page.textContent).toContain("It asks you 2 things");
    expect(page.querySelector("strong")?.textContent).toBe("Apple Pay");
    expect(page.textContent).toContain("STRIPE_KEY");
    expect(page.textContent).toContain("The autopilot hears only that it's set");
    const stripe = [...page.querySelectorAll("button")].find((b) => b.textContent === "Stripe");
    await act(async () => stripe?.click());
    await settle();
    expect(answer).toHaveBeenCalledWith("prj_1", "apa_1", { answer: "Stripe" });
    expect(toast).toHaveBeenCalledWith("Answered: it hears now");
  });

  it("shows those who can't steer a workspace's autopilot that it runs, and not what it does", async () => {
    const runs = on({ scope: "workspace", can: { steer: false, secrets: false, read: false } });
    autopilot.mockResolvedValue(runs);
    queryClient.setQueryData(keys.autopilot("prj_1"), runs);
    const page = render(<AutopilotTab project={project()} />);
    await settle();
    expect(page.textContent).toContain("its plan, sessions, asks and log are for the workspace's admins");
    expect(page.textContent).not.toContain("Ship the");
    expect(page.textContent).toContain("Checks in every hour");
  });

  it("tells people who can't answer a workspace's autopilot that its admins do", async () => {
    const asked = on({
      scope: "workspace",
      can: { steer: false, secrets: true, read: true },
      asks: [
        { id: "apa_1", kind: "question", text: "Which domain?", status: "open", created_at: Date.now() },
      ],
    });
    autopilot.mockResolvedValue(asked);
    queryClient.setQueryData(keys.autopilot("prj_1"), asked);
    const page = render(<AutopilotTab project={project()} />);
    await settle();
    expect(page.textContent).toContain("Which domain?");
    expect(page.textContent).toContain("The workspace's admins answer it.");
    expect([...page.querySelectorAll("button")].some((b) => b.textContent === "Wake now")).toBe(false);
  });

  it("shows its plan's progress, and that it's paused, and by whom", async () => {
    const paused = on({
      state: "paused",
      paused_by: { id: "usr_ada", name: "Ada" },
      paused_at: Date.now() - 60_000,
      plan: "- [x] Cart\n- [ ] Checkout\n- [ ] Receipts",
      plan_at: Date.now() - 60_000,
    });
    autopilot.mockResolvedValue(paused);
    queryClient.setQueryData(keys.autopilot("prj_1"), paused);
    const page = render(<AutopilotTab project={project()} />);
    await settle();
    expect(page.textContent).toContain("Paused");
    expect(page.textContent).toContain("Ada paused it");
    expect(page.textContent).toContain("1 of 3");
    expect([...page.querySelectorAll("button")].some((b) => b.textContent === "Resume")).toBe(true);
  });
});

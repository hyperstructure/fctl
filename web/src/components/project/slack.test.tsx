import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { ProjectSlack } from "./slack";

const updateProject = vi.hoisted(() => vi.fn());

vi.mock("@/lib/api", async () => {
  const actual = await vi.importActual<typeof import("@/lib/api")>("@/lib/api");
  const never = new Proxy({}, { get: () => () => new Promise(() => {}) });
  const answered: Record<string | symbol, unknown> = { updateProject };
  const api = new Proxy({}, { get: (_, k) => answered[k] ?? (() => new Promise(() => {})) });
  return { api, workspaces: never, ApiFailure: actual.ApiFailure };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
  useBlocker: () => ({ status: "idle" }),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  updateProject.mockReset();
});

const connection = (c: Partial<G.SlackConnection> = {}): G.SlackConnection => ({
  team: "T1",
  name: "Acme",
  status: "active",
  created_at: 0,
  settings: {
    channels: true,
    direct_messages: true,
    search: true,
    agent_replies: true,
    notifications: true,
    project_events: true,
    match_email: false,
    transcripts: "keep",
  },
  channels: [
    { id: "C0INCIDENT", name: "incidents", readable: true, updated_at: 0 },
    {
      id: "C0WEBTEAM",
      name: "web-team",
      project: { id: "prj_web", slug: "web", name: "Web" },
      updated_at: 0,
    },
  ],
  reach: "",
  linked: 0,
  ...c,
});

const project = (slack?: G.SlackProject): G.Project =>
  ({
    id: "prj_web",
    slug: "web",
    name: "Web",
    role: "owner",
    workspace: { id: "wsp_1", slug: "acme", name: "Acme Corp", color: "yellow" },
    settings: { slack },
  }) as unknown as G.Project;

function page(p: G.Project, connections: G.SlackConnection[], role: G.WorkspaceRole = "member") {
  queryClient.setQueryData(keys.instance, { name: "fctl", features: { slack: true } });
  queryClient.setQueryData(keys.workspace("acme"), { id: "wsp_1", slug: "acme", name: "Acme Corp", role });
  queryClient.setQueryData(keys.workspaceSlack("wsp_1"), {
    configured: true,
    events: true,
    socket_mode: false,
    match_email: false,
    instance_admin: false,
    connections,
  });
  const container = document.createElement("div");
  // In the document, where its form submits.
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <ProjectSlack project={p} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const switchNamed = (c: HTMLElement, name: string) =>
  c.querySelector<HTMLButtonElement>(`[role="switch"][aria-label="${name}"]`);

describe("a project's Slack settings", () => {
  it("says which channels its agents read, by name", () => {
    const c = page(project(), [connection()]);
    expect(switchNamed(c, "Agents read the readable channels")?.getAttribute("aria-checked")).toBe("true");
    expect(c.textContent).toContain(
      "Besides the thread their session started from, agents read #incidents: the channels Acme Corp's admins made readable.",
    );
  });

  it("says who connects Slack where its workspace doesn't work in it", () => {
    const c = page(project(), []);
    expect(c.textContent).toContain("Acme Corp doesn't work in Slack yet");
    expect(c.textContent).toContain(
      "Acme Corp's admins add fctl to Slack under the workspace's Integrations",
    );
    expect(switchNamed(c, "Agents read the readable channels")).toBeNull();
  });

  it("posts pull requests ready for review and failures to a channel, unless it chooses others", () => {
    const c = page(project(), [connection()]);
    act(() => switchNamed(c, "Post events to a channel")?.click());
    const checked = [...c.querySelectorAll<HTMLInputElement>("input[type=checkbox]")]
      .filter((i) => i.checked)
      .map((i) => i.closest("label")?.textContent);
    expect(checked).toEqual(["Pull requests are ready for review", "A session fails"]);
    // Its maintainers search Slack's public channels too, or give a private one's ID.
    expect(c.querySelector('[aria-label="Choose the channel"]')).not.toBeNull();
    expect(c.textContent).toContain("Choose a channel first.");
    const typed = [...c.querySelectorAll("button")].find(
      (b) => b.textContent === "A private channel? Give its ID",
    );
    act(() => typed?.click());
    expect(c.querySelector('input[placeholder="C0123ABCD"]')).not.toBeNull();
    expect(c.textContent).toContain("Its members invite fctl's app first (/invite @fctl).");
  });

  it("keeps unticked events unticked, and saves none until one is", () => {
    const c = page(
      project({
        read_channels: true,
        events: { team: "T1", channel: "C0WEBTEAM", channel_name: "web-team" },
      }),
      [connection()],
    );
    const boxes = () => [...c.querySelectorAll<HTMLInputElement>("input[type=checkbox]")];
    for (const box of boxes().filter((b) => b.checked)) act(() => box.click());
    expect(boxes().filter((b) => b.checked)).toEqual([]);
    expect(c.textContent).toContain("Choose at least one.");
    expect(c.textContent).toContain("Choose below when it posts.");
    const save = [...c.querySelectorAll("button")].find((b) => b.textContent === "Save");
    expect(save?.disabled).toBe(true);
  });

  it("names the channel it posts to, and says when", () => {
    const c = page(
      project({
        read_channels: false,
        events: { team: "T1", channel: "C0ALERTS", channel_name: "web-alerts", kinds: ["session.question"] },
      }),
      [connection()],
    );
    expect(c.textContent).toContain("Off: agents read only the thread their session started from.");
    expect(c.textContent).toContain("Posts to #web-alerts when an agent asks something.");
    // The channel it posts to stays named, though the workspace's admins chose nothing for it.
    expect(c.textContent).toContain("#web-alerts");
  });

  it("can't start posting where the workspace's admins keep projects' events out of Slack, and says who can change that", () => {
    const off = connection();
    off.settings = { ...off.settings, project_events: false };
    const c = page(project(), [off]);
    expect(switchNamed(c, "Post events to a channel")?.disabled).toBe(true);
    expect(c.textContent).toContain("Acme Corp's admins keep projects' events out of Slack");
  });

  it("lets the workspace's admins search Slack's channels for it", () => {
    const c = page(project(), [connection()], "admin");
    act(() => switchNamed(c, "Post events to a channel")?.click());
    expect(
      [...c.querySelectorAll("button")].some((b) => b.getAttribute("aria-label") === "Choose the channel"),
    ).toBe(true);
  });

  it("saves its choice with the rest of the project's settings", async () => {
    updateProject.mockResolvedValue(project({ read_channels: false }));
    const c = page(project(), [connection()]);
    act(() => switchNamed(c, "Agents read the readable channels")?.click());
    const save = [...c.querySelectorAll("button")].find((b) => b.textContent === "Save");
    await act(async () => save?.click());
    expect(updateProject).toHaveBeenCalledWith("prj_web", {
      settings: { slack: { read_channels: false, events: undefined } },
    });
  });
});

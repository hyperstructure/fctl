import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { MemoryTab } from "./memory";
import { RetrospectivesTab } from "./retrospectives";

// No request is answered: the tabs show what the cache holds.
vi.mock("@/lib/api", () => ({
  api: new Proxy({}, { get: () => () => new Promise(() => {}) }),
  ApiFailure: class extends Error {},
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
  useLocation: () => "",
}));
vi.mock("@/components/markdown", () => ({
  Markdown: ({ text }: { text: string }) => <p>{text}</p>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={queryClient}>{node}</QueryClientProvider>));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const settings: G.ProjectSettings = {
  auto_pull_request: true,
  remote_control: true,
  claude_ai_connectors: true,
  credit_people: true,
  credit_claude: true,
  publish_screenshots: false,
  project_memory: true,
  retrospectives: true,
  env: {},
};

const project = (over: Partial<G.Project> = {}): G.Project => ({
  id: "prj_1",
  slug: "app",
  name: "App",
  default_branch: "main",
  settings,
  role: "editor",
  created_by: { id: "usr_ada", name: "Ada" },
  created_at: 0,
  updated_at: 0,
  active_sessions: 0,
  total_sessions: 3,
  ...over,
});

const ada: G.UserRef = { id: "usr_ada", name: "Ada" };
const notes: G.Memory[] = [
  {
    id: "mem_2",
    project_id: "prj_1",
    text: "Deploys happen on Fridays.",
    created_by: ada,
    created_at: 2,
  },
  {
    id: "mem_1",
    project_id: "prj_1",
    text: "The e2e tests need Chromium.",
    session: { id: "ses_3", number: 3, title: "Fix the login redirect" },
    created_by: ada,
    created_at: 1,
  },
  {
    id: "mem_0",
    project_id: "prj_1",
    text: "Run make test before pushing.",
    session: { id: "ses_1", number: 1, title: "Set up CI" },
    created_by: ada,
    created_at: 0,
    stale: { at: 5, reason: "The Makefile is gone.", by: ada },
  },
];

function memoryTab(p: G.Project, memories = notes) {
  queryClient.setQueryData<G.MemoryList>(keys.memories(p.id), { memories });
  return render(<MemoryTab project={p} />);
}

const button = (c: HTMLElement, text: string) =>
  [...c.querySelectorAll("button")].find((b) => b.textContent?.startsWith(text)) as HTMLButtonElement;

describe("the Memory tab", () => {
  it("lists the current notes with where each came from", () => {
    const c = memoryTab(project());
    expect(c.textContent).toContain("Deploys happen on Fridays.");
    expect(c.textContent).toContain("The e2e tests need Chromium.");
    expect(c.textContent).toContain("Ada's agent in #3 Fix the login redirect");
    expect(c.textContent).not.toContain("Run make test");
    expect(button(c, "Current").textContent).toBe("Current2");
    expect(button(c, "Stale").textContent).toBe("Stale1");
  });

  it("shows the stale notes, with why", () => {
    const c = memoryTab(project());
    act(() => button(c, "Stale").click());
    expect(c.textContent).toContain("Run make test before pushing.");
    expect(c.textContent).toContain("marked by Ada: The Makefile is gone.");
    expect(c.textContent).not.toContain("Deploys happen on Fridays.");
  });

  it("links a duplicate to the note it repeats, which it shows", () => {
    const copy: G.Memory = {
      id: "mem_3",
      project_id: "prj_1",
      text: "Deploys go out on Fridays.",
      session: { id: "ses_4", number: 4, title: "Tidy the memory" },
      created_by: ada,
      created_at: 3,
      stale: {
        at: 6,
        reason: "Duplicate of mem_2",
        session: { id: "ses_4", number: 4, title: "Tidy the memory" },
        by: ada,
        duplicate_of: "mem_2",
      },
    };
    const c = memoryTab(project(), [...notes, copy]);
    act(() => button(c, "Stale").click());
    expect(c.textContent).toContain("Deploys go out on Fridays.");
    expect(c.textContent).toContain("marked by the agent in #4 Tidy the memory: a duplicate of another note");
    expect(c.textContent).not.toContain("Duplicate of mem_2");
    act(() => button(c, "a duplicate of another note").click());
    expect(c.textContent).toContain("Deploys happen on Fridays.");
    expect(c.textContent).not.toContain("Deploys go out on Fridays.");
  });

  it("gives a duplicate's reason when the note it repeats is gone", () => {
    const copy: G.Memory = {
      id: "mem_3",
      project_id: "prj_1",
      text: "Deploys go out on Fridays.",
      created_at: 3,
      stale: { at: 6, reason: "Duplicate of mem_9", by: ada, duplicate_of: "mem_9" },
    };
    const c = memoryTab(project(), [...notes, copy]);
    act(() => button(c, "Stale").click());
    expect(c.textContent).toContain("marked by Ada: Duplicate of mem_9");
  });

  it("lets editors add notes, and not viewers", () => {
    expect(
      memoryTab(project()).querySelector('textarea[aria-label="A note for the project\'s agents"]'),
    ).not.toBeNull();
    act(() => {
      for (const unmount of roots.splice(0)) unmount();
    });
    const viewer = memoryTab(project({ role: "viewer" }));
    expect(viewer.querySelector("textarea")).toBeNull();
    expect(viewer.querySelector('button[aria-label="Note actions"]')).toBeNull();
  });

  it("says when the project's agents keep no memory", () => {
    const c = memoryTab(project({ role: "maintainer", settings: { ...settings, project_memory: false } }));
    expect(c.textContent).toContain("This project's agents keep no memory");
    expect(c.textContent).toContain("Turn it on under Settings");
  });

  it("has a word for a memory without notes", () => {
    const c = memoryTab(project(), []);
    expect(c.textContent).toContain("No notes yet");
    expect(c.textContent).toContain("You can add one too.");
  });
});

describe("the Retrospectives tab", () => {
  const retros: G.Retrospective[] = [
    {
      id: "rtr_2",
      project_id: "prj_1",
      session: { id: "ses_4", number: 4, title: "Move uploads to the new bucket" },
      started_by: ada,
      created_at: 2,
      outcome: "blocked",
      worked_on: "Moving the uploads.",
      insights: "Ask for the bucket's credentials first.",
    },
    {
      id: "rtr_1",
      project_id: "prj_1",
      session: { id: "ses_3", number: 3, title: "Fix the login redirect" },
      started_by: ada,
      created_at: 1,
      outcome: "completed",
      worked_on: "Fixed the redirect.",
    },
  ];

  function retrosTab(p: G.Project) {
    queryClient.setQueryData(keys.retrospectives(p.id, undefined), {
      pages: [{ retrospectives: retros, outcomes: { completed: 1, partial: 0, blocked: 1 } }],
      pageParams: [undefined],
    });
    return render(<RetrospectivesTab project={p} />);
  }

  it("lists them with how many ended each way", () => {
    const c = retrosTab(project());
    expect(button(c, "All").textContent).toBe("All2");
    expect(c.textContent).toContain("Completed1");
    expect(c.textContent).toContain("Partly done0");
    expect(c.textContent).toContain("Blocked1");
    expect(c.textContent).toContain("#4 Move uploads to the new bucket");
    expect(c.textContent).toContain("Worked onMoving the uploads.");
  });

  it("lets editors put an insight in the memory, and not viewers", () => {
    expect(retrosTab(project()).textContent).toContain("Remember for later agents");
    act(() => {
      for (const unmount of roots.splice(0)) unmount();
    });
    expect(retrosTab(project({ role: "viewer" })).textContent).not.toContain("Remember for later agents");
  });
});

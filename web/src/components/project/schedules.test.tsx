import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { ScheduleButton, SchedulesTab } from "./schedules";

const { createSchedule, navigate, toast } = vi.hoisted(() => ({
  createSchedule: vi.fn(),
  navigate: vi.fn(),
  toast: vi.fn(),
}));
// Only setting one up is answered; other requests wait, and the tab shows what the cache holds.
vi.mock("@/lib/api", () => ({
  api: new Proxy({} as Record<string, unknown>, {
    get: (_, k) => (k === "createSchedule" ? createSchedule : () => new Promise(() => {})),
  }),
  ApiFailure: class extends Error {},
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
  useNavigate: () => navigate,
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

/** Lets the request, React Query's notifications (a setTimeout) and React's render run. */
const settle = () => act(() => new Promise((r) => setTimeout(r, 10)));

const project: G.Project = {
  id: "prj_1",
  slug: "app",
  name: "App",
  default_branch: "main",
  settings: {
    auto_pull_request: true,
    remote_control: true,
    claude_ai_connectors: true,
    credit_people: true,
    credit_claude: true,
    publish_screenshots: false,
    project_memory: true,
    retrospectives: true,
    env: {},
  },
  role: "editor",
  created_by: { id: "usr_ada", name: "Ada" },
  created_at: 0,
  updated_at: 0,
  active_sessions: 0,
  total_sessions: 3,
};

const choice = { model: "opus", effort: "high", github: "push" } as const;
const task = "Update the dependencies that have new patch releases and open a pull request.";

const button = (c: HTMLElement) => c.querySelector<HTMLButtonElement>("button[title]");
const press = (b: HTMLButtonElement | null | undefined) =>
  act(() => b?.dispatchEvent(new MouseEvent("click", { bubbles: true })));
const named = (c: HTMLElement, text: string) =>
  [...c.querySelectorAll("button")].find((b) => b.textContent?.trim() === text);

/** The task, handed over from a task box's Schedule, then the project's Schedules tab. */
function handOverAndOpen(onSaved = vi.fn()) {
  const box = render(
    <ScheduleButton project={project} prompt={task} choice={choice} files={0} onSaved={onSaved} />,
  );
  press(button(box));
  queryClient.setQueryData<G.ScheduleList>(keys.schedules(project.id), { schedules: [] });
  return { tab: render(<SchedulesTab project={project} />), onSaved };
}

describe("Schedule, beside a task box's Start", () => {
  it("opens the project's Schedules tab", () => {
    const box = render(<ScheduleButton project={project} prompt={task} choice={choice} files={0} />);
    expect(button(box)?.disabled).toBe(false);
    expect(button(box)?.textContent).toBe("Schedule");
    press(button(box));
    expect(navigate).toHaveBeenCalledWith({
      to: "/p/$slug",
      params: { slug: "app" },
      search: { tab: "schedules" },
    });
  });

  it("waits for a task, and takes none with files", () => {
    const empty = render(<ScheduleButton project={project} prompt="  " choice={choice} files={0} />);
    expect(button(empty)?.disabled).toBe(true);
    const files = render(<ScheduleButton project={project} prompt={task} choice={choice} files={1} />);
    expect(button(files)?.disabled).toBe(true);
    expect(button(files)?.title).toBe("Schedules take no files: start it now, or remove them to schedule it");
  });

  it("says what it is in its tooltip where it shows only its symbol", () => {
    const bare = render(<ScheduleButton project={project} prompt={task} choice={choice} files={0} bare />);
    expect(button(bare)?.title).toBe("Schedule: run this task later, or again and again");
    expect(button(bare)?.querySelector(".sr-only")?.textContent).toBe("Schedule");
  });

  it("opens the tab's form with the task, and the model, effort level and GitHub access picked for it", () => {
    const { tab } = handOverAndOpen();
    expect(tab.querySelector("textarea")?.value).toBe(task);
    expect(tab.querySelector("[aria-label='Model']")?.textContent).toContain("Opus");
    expect(tab.querySelector("[aria-label='Effort']")?.textContent).toContain("High");
  });

  it("lets the task box forget its draft once a schedule is set up with it", async () => {
    const { tab, onSaved } = handOverAndOpen();
    createSchedule.mockResolvedValue({
      id: "sch_1",
      project_id: project.id,
      name: "Update the dependencies",
      prompt: task,
      cron: "0 9 * * 1-5",
      timezone: "UTC",
      enabled: true,
      state: "active",
      next_run_at: Date.now() + 3_600_000,
      created_at: 0,
      updated_at: 0,
    } satisfies G.Schedule);
    press(named(tab, "Set up schedule"));
    await settle();
    expect(createSchedule).toHaveBeenCalledWith(project.id, expect.objectContaining({ prompt: task }));
    expect(onSaved).toHaveBeenCalledTimes(1);
  });

  it("leaves the draft where it was written when the form is cancelled", () => {
    const { tab, onSaved } = handOverAndOpen();
    press(named(tab, "Cancel"));
    expect(tab.querySelector("textarea")).toBeNull();
    expect(onSaved).not.toHaveBeenCalled();
  });
});

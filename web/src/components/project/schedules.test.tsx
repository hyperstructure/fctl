import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { ApiFailure } from "@/lib/api";
import { keys, queryClient } from "@/lib/queries";
import { ScheduleButton, SchedulesTab } from "./schedules";

const { createSchedule, runSchedule, navigate, toast } = vi.hoisted(() => ({
  createSchedule: vi.fn(),
  runSchedule: vi.fn(),
  navigate: vi.fn(),
  toast: vi.fn(),
}));
// Only setting one up and running one are answered; other requests wait, and the tab shows what the
// cache holds.
vi.mock("@/lib/api", () => ({
  api: new Proxy({} as Record<string, unknown>, {
    get: (_, k) =>
      k === "createSchedule"
        ? createSchedule
        : k === "runSchedule"
          ? runSchedule
          : () => new Promise(() => {}),
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
    package_caches: false,
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
    expect(button(bare)?.title).toBe("Schedule: run this task later, again and again, or on an event");
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
      hide_sessions: false,
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

  it("keeps a new schedule's sessions off session lists when asked to", async () => {
    const { tab } = handOverAndOpen();
    const hide = tab.querySelector<HTMLButtonElement>(
      "[role='switch'][aria-label='Keep its sessions off session lists']",
    );
    expect(hide?.getAttribute("aria-checked")).toBe("false");
    press(hide);
    expect(hide?.getAttribute("aria-checked")).toBe("true");
    expect(tab.textContent).toContain("They're listed under its runs");
    createSchedule.mockReturnValue(new Promise(() => {}));
    press(named(tab, "Set up schedule"));
    await settle();
    expect(createSchedule).toHaveBeenCalledWith(project.id, expect.objectContaining({ hide_sessions: true }));
  });

  it("leaves the draft where it was written when the form is cancelled", () => {
    const { tab, onSaved } = handOverAndOpen();
    press(named(tab, "Cancel"));
    expect(tab.querySelector("textarea")).toBeNull();
    expect(onSaved).not.toHaveBeenCalled();
  });
});

describe("Triage new issues", () => {
  const onGithub = { ...project, repo: { owner: "acme", name: "app" } };
  const idea = (c: HTMLElement) =>
    [...c.querySelectorAll("button")].find((b) => b.textContent?.includes("Triage new issues"));

  it("is an idea whose agents only label and comment, for each issue as it's opened", () => {
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [] });
    const tab = render(<SchedulesTab project={onGithub} />);
    expect(idea(tab)?.textContent).toContain("Triage only");
    expect(idea(tab)?.textContent).toContain("When an issue is opened");
    // Starting from it sets up a schedule whose agents triage each new issue, off session lists.
    press(idea(tab));
    expect(tab.querySelector("textarea")?.value).toContain("Triage the issue that was just opened.");
    expect(tab.querySelector("[aria-label='On GitHub']")?.getAttribute("title")).toBe(
      "On GitHub: Triage only",
    );
    expect(named(tab, "On an event")?.getAttribute("aria-pressed")).toBe("true");
    const kind = [...tab.querySelectorAll<HTMLButtonElement>("fieldset button")].find((b) =>
      b.textContent?.includes("An issue is opened"),
    );
    expect(kind?.getAttribute("aria-pressed")).toBe("true");
    expect(
      tab
        .querySelector("[role='switch'][aria-label='Keep its sessions off session lists']")
        ?.getAttribute("aria-checked"),
    ).toBe("true");
  });

  it("is a daily one where the project's issues are in Linear, which starts no event", () => {
    const linear = {
      ...onGithub,
      id: "prj_3",
      settings: { ...onGithub.settings, tickets: { tracker: "linear" } as G.Tickets },
    };
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(linear.id), { schedules: [] });
    const tab = render(<SchedulesTab project={linear} />);
    expect(idea(tab)?.textContent).toContain("Every weekday at");
    press(idea(tab));
    expect(tab.querySelector("textarea")?.value).toContain(
      "Triage the issues opened since the previous run.",
    );
  });

  it("isn't offered where agents reach no tracker", () => {
    const none = {
      ...onGithub,
      id: "prj_2",
      settings: { ...onGithub.settings, tickets: { tracker: "none" } as G.Tickets },
    };
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(none.id), { schedules: [] });
    const tab = render(<SchedulesTab project={none} />);
    expect(idea(tab)).toBeUndefined();
    expect(tab.textContent).toContain("Update dependencies");
  });

  it("says in a schedule's row what its agents may do on GitHub, when less than pushing", () => {
    const schedule = (id: string, github_access?: G.GithubAccess): G.Schedule => ({
      id,
      project_id: onGithub.id,
      name: id === "sch_1" ? "Triage new issues" : "Update dependencies",
      prompt: "…",
      cron: "0 8 * * 1-5",
      timezone: "UTC",
      github_access,
      hide_sessions: false,
      enabled: true,
      state: "active",
      created_at: 0,
      updated_at: 0,
    });
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), {
      schedules: [schedule("sch_1", "triage"), schedule("sch_2")],
    });
    const tab = render(<SchedulesTab project={onGithub} />);
    const rows = [...tab.querySelectorAll("li")];
    expect(rows[0]?.textContent).toContain("Triage only");
    expect(rows[1]?.textContent).not.toContain("only");
  });
});

describe("Schedules that run on events", () => {
  const onGithub = { ...project, repo: { owner: "acme", name: "app" } };

  it("are offered as ideas only where the project's agents work on GitHub", () => {
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(project.id), { schedules: [] });
    const without = render(<SchedulesTab project={project} />);
    expect(without.textContent).not.toContain("When something happens on GitHub");
    expect(without.textContent).not.toContain("Learn from merged pull requests");
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [] });
    const tab = render(<SchedulesTab project={onGithub} />);
    expect(tab.textContent).toContain("When something happens on GitHub");
    for (const name of [
      "Work on labeled issues",
      "Learn from merged pull requests",
      "Fix failing checks on main",
    ])
      expect(tab.textContent).toContain(name);
  });

  it("set up what they wait for, narrowed down", async () => {
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [] });
    const tab = render(<SchedulesTab project={onGithub} />);
    press(named(tab, "Start from scratch"));
    const textarea = tab.querySelector("textarea");
    act(() => {
      if (!textarea) return;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(
        textarea,
        "Do what the issue asks.",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    press(named(tab, "On an event"));
    press(
      [...tab.querySelectorAll<HTMLButtonElement>("fieldset button")].find((b) =>
        b.textContent?.includes("gets a label"),
      ),
    );
    // It needs the label that starts a run.
    expect(tab.textContent).toContain("Name the label that starts a run");
    expect(named(tab, "Set up schedule")?.disabled).toBe(true);
    const label = [...tab.querySelectorAll("input")].find((i) => i.placeholder === "fctl");
    act(() => {
      if (!label) return;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(label, " agent, fctl ");
      label.dispatchEvent(new Event("input", { bubbles: true }));
    });
    expect(tab.textContent).toContain(
      "gets the label agent or fctl from a person who may triage the repository starts a session of its own",
    );
    expect(tab.textContent).toContain("Labels from apps, fctl's agents and issue templates start nothing.");
    createSchedule.mockReturnValue(new Promise(() => {}));
    press(named(tab, "Set up schedule"));
    await settle();
    expect(createSchedule).toHaveBeenCalledWith(
      onGithub.id,
      expect.objectContaining({
        event: { kind: "issue_labeled", labels: ["agent", "fctl"], max_per_hour: 3, max_per_day: 10 },
      }),
    );
    expect(createSchedule.mock.calls[0]?.[1]).not.toHaveProperty("cron");
  });

  it("cap the sessions their events start, 3 an hour and 10 a day unless changed", async () => {
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [] });
    const tab = render(<SchedulesTab project={onGithub} />);
    press(named(tab, "Start from scratch"));
    const textarea = tab.querySelector("textarea");
    act(() => {
      if (!textarea) return;
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(
        textarea,
        "Triage the issue.",
      );
      textarea.dispatchEvent(new Event("input", { bubbles: true }));
    });
    press(named(tab, "On an event"));
    const hour = tab.querySelector<HTMLInputElement>("[aria-label='Sessions an hour, at most']");
    const day = tab.querySelector<HTMLInputElement>("[aria-label='Sessions a day, at most']");
    expect([hour?.value, day?.value]).toEqual(["3", "10"]);
    expect(tab.textContent).toContain("Past either, events start none until there's room again");
    const type = (input: HTMLInputElement | null, text: string) =>
      act(() => {
        if (!input) return;
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
        input.dispatchEvent(new Event("input", { bubbles: true }));
      });
    // None an hour is a pause, which has its own button.
    type(hour, "0");
    expect(tab.textContent).toContain("Let it start at least 1 session an hour: pause it to start none");
    expect(hour?.getAttribute("aria-invalid")).toBe("true");
    expect(named(tab, "Set up schedule")?.disabled).toBe(true);
    type(hour, "10");
    type(day, "1001");
    expect(tab.textContent).toContain("It starts at most 1,000 sessions a day");
    type(day, "50");
    createSchedule.mockReturnValue(new Promise(() => {}));
    press(named(tab, "Set up schedule"));
    await settle();
    expect(createSchedule).toHaveBeenCalledWith(
      onGithub.id,
      expect.objectContaining({ event: { kind: "issue_opened", max_per_hour: 10, max_per_day: 50 } }),
    );
  });

  it("say before anyone picks one when fctl wouldn't hear of it", () => {
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), {
      schedules: [],
      unheard: [
        {
          kind: "issue_opened",
          reason:
            "fctl's GitHub App isn't subscribed to Issues events, so GitHub doesn't tell fctl when an issue is opened or labeled.",
        },
      ],
    });
    const tab = render(<SchedulesTab project={onGithub} />);
    press(named(tab, "Start from scratch"));
    press(named(tab, "On an event"));
    expect(tab.querySelector('[aria-label="fctl doesn\'t hear of it here"]')).not.toBeNull();
    expect(tab.querySelector("[role='status']")?.textContent).toContain("isn't subscribed to Issues events");
    // Another kind it hears of says nothing.
    press(
      [...tab.querySelectorAll<HTMLButtonElement>("fieldset button")].find((b) =>
        b.textContent?.includes("merges"),
      ),
    );
    expect(tab.querySelector("[role='status']")).toBeNull();
  });

  it("keep saying which events fctl wouldn't hear of once one is saved", async () => {
    const unheard = [
      { kind: "issue_opened" as const, reason: "fctl's GitHub App isn't subscribed to Issues events." },
    ];
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [], unheard });
    const tab = render(<SchedulesTab project={onGithub} />);
    press(
      [...tab.querySelectorAll("button")].find((b) => b.textContent?.includes("Fix failing checks on main")),
    );
    createSchedule.mockResolvedValue({
      id: "sch_9",
      project_id: onGithub.id,
      name: "Fix failing checks on main",
      prompt: "…",
      event: { kind: "checks_failed" },
      timezone: "UTC",
      hide_sessions: false,
      enabled: true,
      state: "active",
      created_at: 0,
      updated_at: 0,
    } satisfies G.Schedule);
    press(named(tab, "Set up schedule"));
    await settle();
    const list = queryClient.getQueryData<G.ScheduleList>(keys.schedules(onGithub.id));
    expect(list?.schedules.map((s) => s.id)).toEqual(["sch_9"]);
    expect(list?.unheard).toEqual(unheard);
  });

  it("say what's wrong with the repositories where the form lists none", async () => {
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [] });
    const tab = render(<SchedulesTab project={onGithub} />);
    press(
      [...tab.querySelectorAll("button")].find((b) => b.textContent?.includes("Fix failing checks on main")),
    );
    // The mock's ApiFailure is a plain Error: what the real one reads from the answer goes on as is.
    const answer = {
      message: "The project's agents write to none of its repositories",
      field: "event.repos",
    };
    createSchedule.mockRejectedValue(Object.assign(new ApiFailure(422, answer), answer));
    press(named(tab, "Set up schedule"));
    await settle();
    expect(tab.querySelector("[role='alert']")?.textContent).toBe(
      "The project's agents write to none of its repositories",
    );
  });

  it("say in their row what they wait for, and their runs what happened", () => {
    const schedule: G.Schedule = {
      id: "sch_1",
      project_id: onGithub.id,
      name: "Learn from merged pull requests",
      prompt: "…",
      event: { kind: "pull_request_merged", branches: ["main"], by: "agents" },
      timezone: "UTC",
      hide_sessions: true,
      enabled: true,
      state: "active",
      created_at: 0,
      updated_at: 0,
      last_run: {
        id: "srn_1" as G.ScheduleRunId,
        schedule_id: "sch_1" as G.ScheduleId,
        due_at: Date.now() - 3_600_000,
        at: Date.now() - 3_600_000,
        outcome: "started",
        trigger: "event",
        session: { id: "ses_1" as G.SessionId, number: 42, title: "Learn from merged pull requests: app#12" },
        event: {
          kind: "pull_request_merged",
          repo: { owner: "acme", name: "app" },
          number: 12,
          title: "Fix the login page",
          url: "https://github.com/acme/app/pull/12",
          actor: "grace",
          branch: "main",
        },
      },
    };
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [schedule] });
    queryClient.setQueryData(keys.scheduleRuns(schedule.id), {
      pages: [{ runs: schedule.last_run ? [schedule.last_run] : [] }],
      pageParams: [undefined],
    });
    const tab = render(<SchedulesTab project={onGithub} open={schedule.id} />);
    expect(tab.querySelector("li")?.textContent).toContain(
      "When a pull request merges into main, by fctl's agents",
    );
    const link = tab.querySelector<HTMLAnchorElement>("a[href='https://github.com/acme/app/pull/12']");
    expect(link?.textContent).toBe("@grace merged app#12 into main: Fix the login page");
    // Open, it says how many sessions its events start at most: from before the caps, the defaults.
    expect(tab.textContent).toContain("At most3 sessions an hour and 10 a day");
  });
});

describe("Run now, on a schedule that waits for events", () => {
  const onGithub = { ...project, repo: { owner: "acme", name: "app" } };
  const schedule: G.Schedule = {
    id: "sch_1",
    project_id: onGithub.id,
    name: "Triage new issues",
    prompt: "Triage the issue.",
    event: { kind: "issue_opened" },
    timezone: "UTC",
    hide_sessions: false,
    enabled: true,
    state: "active",
    created_at: 0,
    updated_at: 0,
  };
  const type = (input: HTMLInputElement | null | undefined, text: string) =>
    act(() => {
      if (!input) return;
      Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
      input.dispatchEvent(new Event("input", { bubbles: true }));
    });

  it("asks what to run it on, and gives its agent that", async () => {
    queryClient.setQueryData<G.ScheduleList>(keys.schedules(onGithub.id), { schedules: [schedule] });
    const tab = render(<SchedulesTab project={onGithub} />);
    press(
      tab.querySelector<HTMLButtonElement>(
        "[aria-label='Run Triage new issues now, on your Claude subscription']",
      ),
    );
    // It asks first, rather than run the task on nothing.
    expect(runSchedule).not.toHaveBeenCalled();
    const dialog = [...document.querySelectorAll("dialog")].find((d) =>
      d.textContent?.includes("On the issue"),
    );
    expect(dialog?.textContent).toContain("On the issue");
    const input = dialog?.querySelector<HTMLInputElement>("input");
    type(input, "the login bug");
    runSchedule.mockReturnValue(new Promise(() => {}));
    press(named(dialog as HTMLElement, "Run now"));
    expect(dialog?.textContent).toContain("Write the issue's number, like #41, or paste its link");
    expect(runSchedule).not.toHaveBeenCalled();
    type(input, "#41");
    press(named(dialog as HTMLElement, "Run now"));
    await settle();
    expect(runSchedule).toHaveBeenCalledWith("sch_1", {
      context:
        "Run it on issue acme/app#41 (https://github.com/acme/app/issues/41), as if it had just been opened: `gh issue view 41 -R acme/app` shows it.",
    });
  });
});

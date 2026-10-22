import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { StartButton, StartChoices, useStartChoice, useStartSession } from "./start";

const { createSession, navigate, toast } = vi.hoisted(() => ({
  createSession: vi.fn(),
  navigate: vi.fn(),
  toast: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ api: { createSession } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));
vi.mock("@/components/ui", async (actual) => ({
  ...(await actual<typeof import("@/components/ui")>()),
  Spinner: () => null,
  toast,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.resetAllMocks();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  const client = new QueryClient();
  const rerender = (n: ReactNode) =>
    act(() => root.render(<QueryClientProvider client={client}>{n}</QueryClientProvider>));
  rerender(node);
  roots.push(() => act(() => root.unmount()));
  return { container, rerender, client };
}

function mountWithClient(onStarted?: Parameters<typeof useStartSession>[0]) {
  let current: ReturnType<typeof useStartSession> | undefined;
  function Probe() {
    current = useStartSession(onStarted);
    return null;
  }
  const { client } = render(<Probe />);
  return { client, get: () => current as ReturnType<typeof useStartSession> };
}

const mount = (onStarted?: Parameters<typeof useStartSession>[0]) => mountWithClient(onStarted).get;

/** Lets the request, React Query's notifications (a setTimeout) and React's render run. */
const settle = () => act(() => new Promise((r) => setTimeout(r, 10)));

describe("starting a session", () => {
  it("sends one request, however often it is asked", async () => {
    createSession.mockReturnValue(new Promise(() => {}));
    const s = mount();
    act(() => {
      // Twice in one tick: React has not seen the pending mutation yet.
      s().start("web", "Fix the flaky test");
      s().start("web", "Fix the flaky test");
    });
    await settle();
    expect(s().starting).toBe(true);
    s().start("web", "Fix the flaky test");
    await settle();
    expect(createSession).toHaveBeenCalledTimes(1);
    expect(createSession).toHaveBeenCalledWith("web", { prompt: "Fix the flaky test" });
  });

  it("stays starting once started, until the session page takes over", async () => {
    createSession.mockResolvedValue({ id: "ses_1" });
    navigate.mockResolvedValue(undefined);
    const s = mount();
    act(() => s().start("web", "Fix the flaky test"));
    await settle();
    expect(navigate).toHaveBeenCalledWith({ to: "/s/$id", params: { id: "ses_1" } });
    expect(s().starting).toBe(true);
    s().start("web", "Fix the flaky test");
    await settle();
    expect(createSession).toHaveBeenCalledTimes(1);
  });

  it("opens the session once what has to happen first is done", async () => {
    createSession.mockResolvedValue({ id: "ses_1" });
    navigate.mockResolvedValue(undefined);
    let forgotten: () => void = () => {};
    const onStarted = vi.fn(() => new Promise<void>((r) => (forgotten = r)));
    const s = mount(onStarted);
    act(() => s().start("web", "Fix the flaky test"));
    await settle();
    expect(onStarted).toHaveBeenCalledWith({ id: "ses_1" });
    expect(navigate).not.toHaveBeenCalled();
    forgotten();
    await settle();
    expect(navigate).toHaveBeenCalledWith({ to: "/s/$id", params: { id: "ses_1" } });
  });

  it("shows the error and can start again after a failure", async () => {
    createSession.mockRejectedValueOnce(new Error("No runner is online"));
    const s = mount();
    act(() => s().start("web", "Fix the flaky test"));
    await settle();
    expect(toast).toHaveBeenCalledWith("No runner is online", "error");
    expect(s().starting).toBe(false);
    createSession.mockReturnValue(new Promise(() => {}));
    act(() => s().start("web", "Fix the flaky test"));
    await settle();
    expect(createSession).toHaveBeenCalledTimes(2);
    expect(s().starting).toBe(true);
  });

  it("starts with files alone, once their uploads are done", async () => {
    createSession.mockReturnValue(new Promise(() => {}));
    const shot = { id: "att_1", name: "shot.png", media_type: "image/png", size: 10 };
    let finish: (v: {
      files: { attachments: (typeof shot)[]; previews: Record<string, string> };
      restore: () => void;
    }) => void = () => {};
    const files = {
      drafts: [{}],
      take: vi.fn(() => new Promise<Parameters<typeof finish>[0]>((r) => (finish = r))),
    };
    const s = mount();
    act(() => s().start("web", " ", files));
    await settle();
    expect(createSession).not.toHaveBeenCalled();
    expect(s().starting).toBe(true);
    finish({ files: { attachments: [shot], previews: {} }, restore: () => {} });
    await settle();
    expect(createSession).toHaveBeenCalledWith("web", { prompt: " ", attachments: ["att_1"] });
  });

  it("puts the files back when the start fails", async () => {
    createSession.mockRejectedValueOnce(new Error("No runner is online"));
    const restore = vi.fn();
    const files = {
      drafts: [{}],
      take: () => Promise.resolve({ files: { attachments: [], previews: {} }, restore }),
    };
    const s = mount();
    act(() => s().start("web", "Fix it", files));
    await settle();
    expect(restore).toHaveBeenCalled();
    expect(s().starting).toBe(false);
  });

  it("asks for the model and effort level picked, and refreshes what the project remembers", async () => {
    createSession.mockResolvedValue({ id: "ses_1", project_id: "prj_1", project_slug: "web" });
    navigate.mockResolvedValue(undefined);
    const { client, get } = mountWithClient();
    const invalidate = vi.spyOn(client, "invalidateQueries");
    act(() => get().start("web", "Rename the button", undefined, { model: "haiku", effort: undefined }));
    await settle();
    expect(createSession).toHaveBeenCalledWith("web", { prompt: "Rename the button", model: "haiku" });
    const refreshed = invalidate.mock.calls.map(([f]) => f?.queryKey);
    expect(refreshed).toEqual(
      expect.arrayContaining([["projects"], ["project", "web"], ["project", "prj_1"]]),
    );
  });

  it("ignores a blank prompt", async () => {
    const s = mount();
    act(() => s().start("web", "  \n"));
    await settle();
    expect(createSession).not.toHaveBeenCalled();
    expect(s().starting).toBe(false);
  });
});

describe("StartButton", () => {
  /** What the button says to assistive technology: the label that is not hidden. */
  const said = (button: HTMLButtonElement) =>
    [...button.querySelectorAll(":scope > span > span")]
      .filter((el) => el.getAttribute("aria-hidden") !== "true")
      .map((el) => el.textContent?.trim())
      .join("");

  it("says Starting and takes no presses while starting", () => {
    const { container, rerender } = render(<StartButton starting={false}>Start a session</StartButton>);
    const button = container.querySelector("button") as HTMLButtonElement;
    expect(button.disabled).toBe(false);
    expect(said(button)).toBe("Start a session");
    rerender(<StartButton starting>Start a session</StartButton>);
    expect(button.disabled).toBe(true);
    expect(said(button)).toBe("Starting");
  });

  it("is disabled without a prompt", () => {
    const { container } = render(
      <StartButton starting={false} disabled>
        Start
      </StartButton>,
    );
    expect((container.querySelector("button") as HTMLButtonElement).disabled).toBe(true);
  });

  it("shows the arrow alone on a phone, and its words to screen readers", () => {
    const { container } = render(<StartButton starting={false}>Start</StartButton>);
    const button = container.querySelector("button") as HTMLButtonElement;
    const words = [...button.querySelectorAll(":scope > span > span > span")];
    expect(words.map((w) => [w.textContent, w.classList.contains("max-sm:sr-only")])).toEqual([
      ["Start", true],
      ["Starting", true],
    ]);
    expect(button.querySelector(":scope > span > span > svg")).not.toBeNull();
    expect(said(button)).toBe("Start");
  });
});

describe("StartChoices", () => {
  const project = (settings: Partial<G.ProjectSettings>, last: Partial<G.Project> = {}) => ({
    settings: settings as G.ProjectSettings,
    ...last,
  });
  function Choices({ p, kept }: { p: ReturnType<typeof project>; kept?: { model?: string } }) {
    const { choice, pick } = useStartChoice(p, kept);
    return <StartChoices project={p} choice={choice} onPick={pick} />;
  }
  const shown = (container: HTMLElement) =>
    [...container.querySelectorAll("[role=combobox]")].map((b) => [
      b.getAttribute("aria-label"),
      // The trigger's value, beside the copies that size it.
      b.querySelector(":scope > .grid > span:not([aria-hidden])")?.textContent,
    ]);

  it("names what the session asks for, or Claude's default", () => {
    const { container, rerender } = render(<Choices p={project({})} />);
    expect(shown(container)).toEqual([
      ["Model", "Default model"],
      ["Effort", "Default effort"],
    ]);
    rerender(<Choices p={project({ model: "opus", effort: "high" }, { last_model: "haiku" })} />);
    expect(shown(container)).toEqual([
      ["Model", "Haiku"],
      ["Effort", "High effort"],
    ]);
  });

  it("starts on what the draft kept", () => {
    const { container } = render(<Choices p={project({ model: "opus" })} kept={{ model: "fable" }} />);
    expect(shown(container)[0]).toEqual(["Model", "Fable"]);
  });

  it("shows the model's name and the effort level's symbol on a phone", () => {
    // What a phone shows, for the eyes only: screen readers read the labels above.
    const short = (container: HTMLElement) =>
      [...container.querySelectorAll("[role=combobox]")].map((b) => [
        b.querySelector(":scope > .grid[aria-hidden] > span:not([aria-hidden])")?.textContent,
        !!b.querySelector(":scope > span:not(.grid) > svg"),
      ]);
    const { container, rerender } = render(<Choices p={project({ model: "opus", effort: "max" })} />);
    expect(short(container)).toEqual([
      ["Opus", false],
      [undefined, true],
    ]);
    expect(shown(container)).toEqual([
      ["Model", "Opus"],
      ["Effort", "Max effort"],
    ]);
    rerender(<Choices p={project({})} />);
    expect(short(container)).toEqual([
      ["Default", false],
      [undefined, true],
    ]);
  });

  it("says what the agent may do on GitHub where the project has a repository", () => {
    const repo = { repo: { owner: "acme", name: "web" } };
    const { container, rerender } = render(<Choices p={project({})} />);
    // Without a repository there is nothing to do there.
    expect(shown(container).map(([label]) => label)).toEqual(["Model", "Effort"]);
    rerender(<Choices p={{ ...project({}), ...repo }} />);
    expect(shown(container)[2]).toEqual(["On GitHub", "Can push"]);
    // A project that holds its agents to commenting starts there.
    rerender(<Choices p={{ ...project({ github_access: "comment" }), ...repo }} />);
    expect(shown(container)[2]).toEqual(["On GitHub", "Comments only"]);
  });
});

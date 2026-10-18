import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { StartButton, useStartSession } from "./start";

const { createSession, navigate, toast } = vi.hoisted(() => ({
  createSession: vi.fn(),
  navigate: vi.fn(),
  toast: vi.fn(),
}));
vi.mock("@/lib/api", () => ({ api: { createSession } }));
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));
vi.mock("@/components/ui", () => ({ Spinner: () => null, toast }));

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
  return { container, rerender };
}

function mount() {
  let current: ReturnType<typeof useStartSession> | undefined;
  function Probe() {
    current = useStartSession();
    return null;
  }
  render(<Probe />);
  return () => current as ReturnType<typeof useStartSession>;
}

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
    [...button.querySelectorAll("span > span")]
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
});

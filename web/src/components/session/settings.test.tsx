import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { SessionChoices } from "./settings";

const { updateSession, toast } = vi.hoisted(() => ({ updateSession: vi.fn(), toast: vi.fn() }));
vi.mock("@/lib/api", () => ({ api: { updateSession } }));
vi.mock("@/components/ui", async (actual) => ({
  ...(await actual<typeof import("@/components/ui")>()),
  Spinner: () => null,
  toast,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
beforeEach(() => {
  vi.useFakeTimers();
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  vi.useRealTimers();
  vi.resetAllMocks();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const client = new QueryClient();
  act(() => root.render(<QueryClientProvider client={client}>{node}</QueryClientProvider>));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const session = (more: Partial<G.Session> = {}): G.Session =>
  ({
    id: "ses_1",
    status: "idle",
    model: "claude-opus-5-5",
    effort: "xhigh",
    ...more,
  }) as G.Session;

/** Opens the `n`th picker and picks the option labelled `label`. */
function pick(container: HTMLElement, n: number, label: string) {
  act(() => container.querySelectorAll<HTMLElement>("[role=combobox]")[n]?.click());
  const option = [...container.querySelectorAll<HTMLElement>("[role=option]")].find(
    (o) => o.querySelector(".truncate")?.textContent === label,
  );
  if (!option) throw new Error(`no option ${label}`);
  act(() => option.click());
  act(() => vi.advanceTimersByTime(1000));
}

const dialog = () => document.querySelector("dialog[open]");
const settle = async () => {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(50);
  });
};

describe("a session's model and effort level", () => {
  it("lists the model it works on, then the aliases, and the levels", () => {
    const container = render(<SessionChoices session={session()} perTurnEffort />);
    act(() => container.querySelectorAll<HTMLElement>("[role=combobox]")[0]?.click());
    expect(
      [...container.querySelectorAll("[role=option]")].map((o) => o.querySelector(".truncate")?.textContent),
    ).toEqual(["claude-opus-5-5", "Opus", "Sonnet", "Haiku", "Fable"]);
  });

  it("changes the effort level at once where Claude keeps its cache", async () => {
    updateSession.mockResolvedValue(session({ effort: "medium" }));
    const container = render(<SessionChoices session={session()} perTurnEffort />);
    pick(container, 1, "Medium");
    expect(dialog()).toBeNull();
    await settle();
    expect(updateSession).toHaveBeenCalledWith("ses_1", { effort: "medium" });
  });

  it("asks before a new model, which reads the conversation without the cache", async () => {
    updateSession.mockResolvedValue(session({ model: "sonnet" }));
    const container = render(<SessionChoices session={session()} perTurnEffort />);
    pick(container, 0, "Sonnet");
    expect(updateSession).not.toHaveBeenCalled();
    expect(dialog()?.querySelector("h2")?.textContent).toBe("Switch to Sonnet?");
    expect(dialog()?.textContent).toContain("reads the whole conversation again without it");
    const confirm = [...(dialog()?.querySelectorAll("button") ?? [])].find(
      (b) => b.textContent === "Switch to Sonnet",
    );
    act(() => confirm?.click());
    await settle();
    expect(updateSession).toHaveBeenCalledWith("ses_1", { model: "sonnet" });
    expect(dialog()).toBeNull();
  });

  it("asks before an effort level where Claude sends it for the whole conversation", () => {
    const container = render(<SessionChoices session={session()} perTurnEffort={false} />);
    pick(container, 1, "Medium");
    expect(updateSession).not.toHaveBeenCalled();
    expect(dialog()?.querySelector("h2")?.textContent).toBe("Change the effort level to Medium?");
    const cancel = [...(dialog()?.querySelectorAll("button") ?? [])].find((b) => b.textContent === "Cancel");
    act(() => cancel?.click());
    expect(dialog()).toBeNull();
    expect(updateSession).not.toHaveBeenCalled();
  });

  it("says why a change didn't go through", async () => {
    updateSession.mockRejectedValue(new Error("The runner runs an older fctl"));
    const container = render(<SessionChoices session={session()} perTurnEffort />);
    pick(container, 1, "Low");
    await settle();
    expect(toast).toHaveBeenCalledWith("The runner runs an older fctl", "error");
  });
});

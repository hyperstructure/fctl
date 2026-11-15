import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { queryClient } from "@/lib/queries";
import type { SentFiles } from "./attachments";
import { Composer } from "./composer";
import type { MovedOn } from "./moved-on";

vi.mock("@/lib/api", () => ({
  api: new Proxy({} as Record<string, unknown>, { get: () => () => new Promise(() => {}) }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, params, children }: { to: string; params?: Record<string, string>; children: ReactNode }) => (
    <a href={to.replace(/\$(\w+)/g, (_, k: string) => params?.[k] ?? "")}>{children}</a>
  ),
  useNavigate: () => () => {},
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

const NOW = Date.now();
const EDITOR: G.SessionRole = "editor";

function movedOn(more: Partial<MovedOn> = {}): MovedOn {
  return {
    session: { id: "ses_20", number: 20, status: "running", started_at: NOW - 2 * 3_600_000 },
    paused: false,
    word: { tone: "accent", word: "At work" },
    canMessage: true,
    send: vi.fn(async () => {}),
    ...more,
  };
}

/** The composer of session #7, an earlier session of the autopilot's own when `to` is given. */
function composer(to?: MovedOn, status: G.SessionStatus = "idle") {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const sent: string[] = [];
  const onSend = vi.fn(async (text: string, _files: SentFiles) => {
    sent.push(text);
  });
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <Composer
          status={status}
          role={EDITOR}
          viewers={[]}
          sessionId="ses_7"
          onSend={onSend}
          onInterrupt={() => {}}
          onTyping={() => {}}
          onResume={async () => {}}
          movedOn={to}
        />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const box = () => container.querySelector<HTMLTextAreaElement>("textarea") as HTMLTextAreaElement;
  const way = (name: string) =>
    [...container.querySelectorAll<HTMLButtonElement>("[role=alertdialog] button")].find((b) =>
      b.textContent?.startsWith(name),
    );
  return {
    container,
    sent,
    box,
    way,
    card: () => container.querySelector("[role=alertdialog]"),
    type(text: string) {
      act(() => {
        Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(box(), text);
        box().dispatchEvent(new Event("input", { bubbles: true }));
      });
    },
    async enter() {
      await act(async () => {
        box().dispatchEvent(new KeyboardEvent("keydown", { key: "Enter", bubbles: true }));
      });
    },
    async click(b?: HTMLElement) {
      await act(async () => b?.click());
    },
  };
}

describe("a message in an earlier session of the autopilot's own", () => {
  it("says where the autopilot works now, above the box", () => {
    const c = composer(movedOn());
    expect(c.container.textContent).toContain("The autopilot works in#20now, not in this earlier session");
    expect(c.container.querySelector('a[href="/s/ses_20"]')?.textContent).toBe("#20");
  });

  it("asks first, with the way to where it works now focused", async () => {
    const to = movedOn();
    const c = composer(to);
    c.type("Add link previews");
    await c.enter();
    expect(c.sent).toEqual([]);
    expect(to.send).not.toHaveBeenCalled();
    expect(c.card()?.textContent).toContain("The autopilot works in #20 now");
    expect(c.card()?.textContent).toContain("At work· started 2h ago");
    expect(document.activeElement).toBe(c.way("Send to #20"));
    // The line above the box says it once, in the card, while it asks.
    expect(c.container.textContent).not.toContain("not in this earlier session");
    // The text waits in the box meanwhile.
    expect(c.box().value).toBe("Add link previews");
  });

  it("sends it where the autopilot works now", async () => {
    const to = movedOn();
    const c = composer(to);
    c.type("Add link previews");
    await c.enter();
    await c.click(c.way("Send to #20"));
    expect(to.send).toHaveBeenCalledWith("Add link previews", { attachments: [], previews: {} });
    expect(c.sent).toEqual([]);
    expect(c.box().value).toBe("");
    expect(c.card()).toBeNull();
  });

  it("sends it here anyway, and then asks no more", async () => {
    const to = movedOn();
    const c = composer(to);
    c.type("Carry on here");
    await c.enter();
    await c.click(c.way("Send here anyway"));
    expect(c.sent).toEqual(["Carry on here"]);
    expect(to.send).not.toHaveBeenCalled();
    expect(c.card()).toBeNull();
    c.type("And this too");
    await c.enter();
    expect(c.sent).toEqual(["Carry on here", "And this too"]);
    expect(c.card()).toBeNull();
  });

  it("goes back to the message with Escape, as it was", async () => {
    const c = composer(movedOn());
    c.type("Add link previews");
    await c.enter();
    await act(async () => {
      document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(c.card()).toBeNull();
    expect(c.box().value).toBe("Add link previews");
    expect(document.activeElement).toBe(c.box());
    expect(c.sent).toEqual([]);
  });

  it("moves between the ways with the arrows", async () => {
    const c = composer(movedOn());
    c.type("Add link previews");
    await c.enter();
    const arrow = (key: string) =>
      act(() => {
        document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
      });
    arrow("ArrowDown");
    expect(document.activeElement).toBe(c.way("Send here anyway"));
    arrow("ArrowDown");
    expect(document.activeElement).toBe(c.way("Send to #20"));
    arrow("ArrowUp");
    expect(document.activeElement).toBe(c.way("Send here anyway"));
  });

  it("offers only here when the reader can't message where it works now", async () => {
    const c = composer(movedOn({ canMessage: false }));
    c.type("Add link previews");
    await c.enter();
    expect(c.way("Send to #20")?.disabled).toBe(true);
    expect(c.card()?.textContent).toContain("You can follow it, but not message it");
    expect(document.activeElement).toBe(c.way("Send here anyway"));
  });

  it("says a paused autopilot acts once resumed", async () => {
    const c = composer(movedOn({ paused: true, word: { tone: "pink", word: "Paused" } }));
    c.type("Add link previews");
    await c.enter();
    expect(c.card()?.textContent).toContain("acts again once someone resumes it");
  });

  it("asks nothing in any other session", async () => {
    const c = composer();
    c.type("Add link previews");
    await c.enter();
    expect(c.sent).toEqual(["Add link previews"]);
    expect(c.card()).toBeNull();
  });
});

import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { setSessionView } from "@/lib/session-view";
import { SessionRoute } from "./session-next";

/** The session the address names. */
const route = vi.hoisted(() => ({ id: "ses_a" }));
/** The live sessions the page made, by id, to say what their server would. */
const stores = vi.hoisted(() => new Map<string, { onMessage(m: unknown): void }>());

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ id: route.id }),
  useSearch: () => ({}),
  useNavigate: () => vi.fn(),
  Link: ({ children }: { children: ReactNode }) => <a href="/s/ses_b">{children}</a>,
}));

// The session as the cache holds it, without a connection.
vi.mock("@/lib/live", async (original) => {
  const live = await original<typeof import("@/lib/live")>();
  const { useMemo } = await import("react");
  return {
    ...live,
    useLiveSession: (id: string, initial?: G.Session) => {
      const store = useMemo(() => new live.LiveSession(id, initial), [id]);
      stores.set(id, store as unknown as { onMessage(m: unknown): void });
      return [store.getSnapshot(), store];
    },
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

beforeEach(() => {
  vi.stubGlobal("fetch", () => new Promise(() => {}));
});

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  vi.unstubAllGlobals();
  route.id = "ses_a";
  stores.clear();
  setSessionView("classic");
});

function session(status: G.SessionStatus = "idle"): G.Session {
  return {
    id: "ses_a",
    project_id: "prj_1",
    project_slug: "web",
    number: 12,
    title: "Fix the flaky login test",
    description: "The login test waits for the redirect, which CI sometimes takes longer for.",
    status,
    created_by: { id: "usr_ada", name: "Ada Lovelace" },
    created_at: 0,
    updated_at: 0,
    stats: {
      turns: 1,
      input_tokens: 0,
      output_tokens: 0,
      cache_read_tokens: 0,
      tool_calls: 0,
      files_changed: 0,
      additions: 0,
      deletions: 0,
      busy_ms: 0,
      compactions: 0,
    },
    role: "owner",
    archived: false,
  };
}

let seq = 0;
const ev = (kind: string, data: unknown): G.SessionEvent =>
  ({ seq: ++seq, ts: 1_000 + seq * 1000, kind, data }) as unknown as G.SessionEvent;
const assistant = (id: string, content: unknown[]) =>
  ev("claude", { type: "assistant", message: { id, role: "assistant", content }, parent_tool_use_id: null });
const result = (id: string, text: string) =>
  ev("claude", {
    type: "user",
    message: { role: "user", content: [{ type: "tool_result", tool_use_id: id, content: text }] },
    parent_tool_use_id: null,
  });

/** A turn: Ada asked, the agent edited a file, ran the tests and said so. */
const turn = (ask: string): G.SessionEvent[] => [
  ev("user_message", { text: ask, source: "web", author: { id: "usr_ada", name: "Ada Lovelace" } }),
  assistant(`m${seq}`, [
    {
      type: "tool_use",
      id: `e${seq}`,
      name: "Edit",
      input: { file_path: "/home/agent/workspace/login.test.ts", old_string: "a", new_string: "b\nc" },
    },
  ]),
  result(`e${seq - 1}`, "ok"),
  assistant(`m${seq}`, [{ type: "tool_use", id: `b${seq}`, name: "Bash", input: { command: "npm test" } }]),
  result(`b${seq - 1}`, "1 passed"),
  assistant(`m${seq}`, [{ type: "text", text: "The test waits for the redirect now." }]),
  ev("claude", { type: "result", subtype: "success", duration_ms: 65_000, is_error: false, result: "" }),
];

function page(view: "classic" | "new" = "new") {
  setSessionView(view);
  queryClient.setQueryData(keys.session("ses_a"), session());
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = () =>
    act(() =>
      root.render(
        <QueryClientProvider client={queryClient}>
          <SessionRoute />
        </QueryClientProvider>,
      ),
    );
  render();
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return {
    container,
    /** The session's server says this, and the page shows it. */
    hear(m: G.ViewerServer) {
      act(() => stores.get(route.id)?.onMessage(m));
      render();
    },
    snapshot(events: G.SessionEvent[], s: G.Session = session()) {
      this.hear({
        type: "snapshot",
        session: s,
        role: "owner",
        events,
        has_more_before: false,
        next_seq: (events[events.length - 1]?.seq ?? 0) + 1,
        live: [],
        viewers: [],
        runner_connected: true,
      });
    },
    text: () => container.textContent ?? "",
    click(selector: string) {
      const button = container.querySelector<HTMLButtonElement>(selector);
      expect(button).not.toBeNull();
      act(() => button?.click());
    },
  };
}

function press(key: string) {
  act(() => {
    document.body.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  });
}

describe("SessionRoute", () => {
  it("shows the classic page until the new one is picked, and v switches between them", () => {
    const p = page("classic");
    expect(p.container.querySelector(".session-next")).toBeNull();
    press("v");
    expect(p.container.querySelector(".session-next")).not.toBeNull();
    press("v");
    expect(p.container.querySelector(".session-next")).toBeNull();
  });
});

describe("the new session page", () => {
  it("opens with a cover, and reads in turns: each ask, the work on its thread, and how it ended", () => {
    const p = page();
    p.snapshot([...turn("Fix the flaky login test"), ...turn("Now the signup test")]);
    // The cover, once the session's start is there.
    expect(p.text()).toContain("The login test waits for the redirect");
    expect(p.text()).toContain("Started by Ada Lovelace");
    // Two turns, each opened by its ask.
    const starts = [...p.container.querySelectorAll("[data-turn]")];
    expect(starts.map((s) => s.querySelector("[data-find^='msg:']")?.textContent)).toEqual([
      "Fix the flaky login test",
      "Now the signup test",
    ]);
    // The work and the agent's words on the thread, which ends where the turn does.
    expect(p.container.querySelectorAll(".thread").length).toBeGreaterThan(2);
    expect(p.container.querySelectorAll(".thread-end")).toHaveLength(2);
    expect(p.text()).toContain("Finished in 1m 5s");
    expect(p.text()).toContain("2 steps");
    expect(p.text()).toContain("Edited 1 file");
  });

  it("lists the files a turn edited, each a click from its changes when the session has them", () => {
    const p = page();
    p.snapshot(turn("Fix the flaky login test"));
    p.click(".thread-end button[aria-expanded]");
    const rows = [...p.container.querySelectorAll(".thread-end li")].map((li) => li.textContent);
    expect(rows).toEqual(["login.test.ts+2 −1"]);
    // Not among the changes the session has (no git state here): nothing to open.
    expect(p.container.querySelector(".thread-end li button")).toBeNull();
  });

  it("says what the agent does now above the message box, while it works", async () => {
    const p = page();
    p.snapshot(
      [
        ev("user_message", { text: "Run the tests", source: "web" }),
        assistant("m1", [
          {
            type: "tool_use",
            id: "b1",
            name: "Bash",
            input: { command: "npm test", description: "Run the tests" },
          },
        ]),
      ],
      session("running"),
    );
    expect(p.container.querySelector(".now-line")?.textContent).toContain("Run the tests");
    // Done: it goes, once the step it showed has shown long enough to be read.
    p.hear({ type: "events", events: [result("b1", "ok")] });
    p.hear({ type: "session", session: session("idle") });
    await act(() => new Promise((r) => setTimeout(r, 700)));
    expect(p.container.querySelector(".now-line")).toBeNull();
  });

  it("shows only the conversation when asked: people's messages and the agent's words", () => {
    const p = page();
    p.snapshot(turn("Fix the flaky login test"));
    const parts = () => p.container.querySelectorAll("[data-part]").length;
    const all = parts();
    p.click('button[aria-label="Show only the conversation"]');
    expect(parts()).toBe(2);
    expect(p.text()).toContain("The test waits for the redirect now.");
    p.click(`button[aria-label="Show the agent's work"]`);
    expect(parts()).toBe(all);
  });

  it("marks where what came while the reader was away starts, until they leave again", () => {
    const p = page();
    p.snapshot(turn("Fix the flaky login test"));
    const visibility = (v: "hidden" | "visible") =>
      act(() => {
        Object.defineProperty(document, "visibilityState", { value: v, configurable: true });
        document.dispatchEvent(new Event("visibilitychange"));
      });
    visibility("hidden");
    const later = turn("Now the signup test");
    p.hear({ type: "events", events: later });
    visibility("visible");
    const line = p.container.querySelector(".new-line");
    expect(line).not.toBeNull();
    // Just above the card of the turn that came meanwhile.
    expect(line?.closest("[data-turn]")?.textContent).toContain("Now the signup test");
    // What comes once the reader is back isn't marked as new.
    p.hear({ type: "events", events: turn("And the reset test") });
    expect(p.container.querySelectorAll(".new-line")).toHaveLength(1);
    expect(line?.closest("[data-turn]")?.textContent).toContain("Now the signup test");
    // Leaving again starts over: nothing came yet.
    visibility("hidden");
    expect(p.container.querySelector(".new-line")).toBeNull();
    visibility("visible");
  });

  it("marks the parts that come after it first showed, whose thread draws as they come", () => {
    const p = page();
    const first = turn("Fix the flaky login test");
    p.snapshot(first);
    expect(p.container.querySelectorAll(".arrive")).toHaveLength(0);
    p.hear({ type: "events", events: turn("Now the signup test") });
    expect(p.container.querySelectorAll(".arrive").length).toBeGreaterThan(0);
  });
});

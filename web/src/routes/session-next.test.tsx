import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import type { LiveSession } from "@/lib/live";
import { keys, queryClient } from "@/lib/queries";
import { setSessionView } from "@/lib/session-view";
import { SessionRoute } from "./session-next";

/** The session the address names. */
const route = vi.hoisted(() => ({ id: "ses_a", search: {} as { turn?: number } }));
const navigate = vi.hoisted(() => vi.fn());
/** The live sessions the page made, by id, to say what their server would. */
const stores = vi.hoisted(() => new Map<string, { onMessage(m: unknown): void }>());

vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ id: route.id }),
  useSearch: () => route.search,
  useNavigate: () => navigate,
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
  route.search = {};
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

/** The page goes out of sight, or comes back. */
const visibility = (v: "hidden" | "visible") =>
  act(() => {
    Object.defineProperty(document, "visibilityState", { value: v, configurable: true });
    document.dispatchEvent(new Event("visibilitychange"));
  });

/** What the line above the message box says, once what rolls away from it has gone. */
function nowSays(container: HTMLElement): string {
  const copy = container.querySelector(".now-line")?.cloneNode(true) as HTMLElement | undefined;
  for (const gone of copy?.querySelectorAll(".animate-now-out, .animate-digit-out") ?? []) gone.remove();
  return copy?.textContent ?? "";
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

/** The agent writes its plan: the first step done, the second under way. */
const plan = () =>
  assistant(`m${seq}`, [
    {
      type: "tool_use",
      id: `todo${seq}`,
      name: "TodoWrite",
      input: {
        todos: [
          { content: "Read the test", status: "completed", activeForm: "Reading the test" },
          { content: "Fix the wait", status: "in_progress", activeForm: "Fixing the wait" },
          { content: "Run it on CI", status: "pending", activeForm: "Running it on CI" },
        ],
      },
    },
  ]);

describe("the new session page", () => {
  it("keeps the agent's plan in view, and says above the message box where it is in it", () => {
    const p = page();
    p.snapshot(
      [
        ev("user_message", { text: "Fix the flaky login test", source: "web" }),
        plan(),
        assistant("m-run", [
          {
            type: "tool_use",
            id: "b1",
            name: "Bash",
            input: { command: "npm test", description: "Run the test" },
          },
        ]),
      ],
      session("running"),
    );
    const rail = [...p.container.querySelectorAll("section")].find((s) => s.textContent?.startsWith("Plan"));
    expect(rail?.textContent).toContain("1 of 3");
    expect(rail?.textContent).toContain("Fixing the wait");
    const now = p.container.querySelector(".now-line")?.textContent ?? "";
    expect(now).toContain("Fixing the wait");
    expect(now).toContain("Run the test");
    expect(now).toContain("1/3");
  });

  it("says it's the reader's turn once the agent waits for them, and asks for a reply only as it hands over", () => {
    const p = page();
    // Opened waiting: whose turn it is, without the message box asking.
    p.snapshot(turn("Fix the flaky login test"));
    expect(p.container.querySelector(".your-turn")?.textContent).toContain("Your turn");
    expect(p.container.querySelector(".invite")).toBeNull();
    // The agent works again, then hands the turn over while the reader watches.
    p.hear({ type: "session", session: session("running") });
    expect(p.container.querySelector(".your-turn")).toBeNull();
    p.hear({ type: "session", session: session("idle") });
    expect(p.container.querySelector(".invite")).not.toBeNull();
  });

  it("opens with the session at a glance: its figures, and its turns as a ribbon that goes to each", () => {
    const p = page();
    const s = {
      ...session(),
      stats: { ...session().stats, turns: 2, tool_calls: 4, files_changed: 1, additions: 2, deletions: 1 },
    };
    p.snapshot([...turn("Fix the flaky login test"), ...turn("Now the signup test")], s);
    const glance = p.container.querySelector(".cover")?.textContent ?? "";
    expect(glance).toContain("turns");
    expect(glance).toContain("steps");
    expect(glance).toContain("+2");
    const ribbon = p.container.querySelectorAll("nav[aria-label='Turns'] button");
    expect(ribbon).toHaveLength(2);
    expect(ribbon[1]?.getAttribute("aria-label")).toBe("Now the signup test: finished in 1m 5s");
  });

  it("opens a link to a turn at that turn, and the address forgets it", async () => {
    const scrolled = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    try {
      const events = [...turn("Fix the flaky login test"), ...turn("Now the signup test")];
      const second = events.filter((e) => e.kind === "user_message")[1]?.seq;
      route.search = { turn: second };
      const p = page();
      p.snapshot(events);
      // Where each turn starts on the page, which happy-dom doesn't lay out: the second 2000 px down.
      for (const [i, el] of p.container.querySelectorAll("[data-turn]").entries())
        el.getBoundingClientRect = () => ({ top: 1000 * (i + 1), bottom: 1000 * (i + 1) + 100 }) as DOMRect;
      // It goes there on the next frame, past the header (here at 0) and a little room.
      await act(() => new Promise((r) => requestAnimationFrame(r)));
      expect(scrolled).toHaveBeenLastCalledWith({ top: 1992, behavior: "smooth" });
      expect(navigate).toHaveBeenCalledWith(expect.objectContaining({ search: {}, replace: true }));
    } finally {
      scrolled.mockRestore();
    }
  });

  it("loads earlier activity for a linked turn it doesn't have yet, and goes nowhere for one it can't find", async () => {
    const scrolled = vi.spyOn(window, "scrollTo").mockImplementation(() => {});
    try {
      const events = turn("Now the signup test");
      const first = events[0]?.seq ?? 0;
      route.search = { turn: first - 3 };
      const p = page();
      const store = stores.get("ses_a") as unknown as LiveSession;
      vi.spyOn(store, "loadOlder").mockResolvedValue(undefined);
      const earlier = vi.spyOn(store, "loadEarlier").mockResolvedValue(undefined);
      p.hear({
        type: "snapshot",
        session: session(),
        role: "owner",
        events,
        has_more_before: true,
        next_seq: (events[events.length - 1]?.seq ?? 0) + 1,
        live: [],
        viewers: [],
        runner_connected: true,
      });
      expect(earlier).toHaveBeenCalledWith({ until: first - 3 });
      // Before the session's first turn, with nothing earlier: nowhere to go.
      route.search = { turn: 1 };
      const q = page();
      q.snapshot(turn("Fix the flaky login test"));
      await act(() => new Promise((r) => requestAnimationFrame(r)));
      expect(scrolled.mock.calls.filter(([o]) => (o as ScrollToOptions)?.behavior === "smooth")).toEqual([]);
    } finally {
      scrolled.mockRestore();
    }
  });

  it("leads the line above the message box with a plan only in the turn it was written in", () => {
    const p = page();
    p.snapshot(
      [
        ev("user_message", { text: "Fix the flaky login test", source: "web" }),
        plan(),
        assistant("m-a", [{ type: "text", text: "Fixed." }]),
        ev("claude", { type: "result", subtype: "success", duration_ms: 1000, is_error: false }),
        ev("user_message", { text: "Now update the README", source: "web" }),
        assistant("m-b", [
          {
            type: "tool_use",
            id: "r1",
            name: "Read",
            input: { file_path: "/home/agent/workspace/README.md" },
          },
        ]),
      ],
      session("running"),
    );
    const now = p.container.querySelector(".now-line")?.textContent ?? "";
    expect(now).toContain("Reading");
    expect(now).not.toContain("Fixing the wait");
    expect(now).not.toContain("1/3");
    // The details keep the plan as it was left.
    const rail = [...p.container.querySelectorAll("section")].find((x) => x.textContent?.startsWith("Plan"));
    expect(rail?.textContent).toContain("Fixing the wait");
    expect(rail?.querySelector(".animate-pulse-soft")).toBeNull();
  });

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
    // Done: the step it showed holds until it has shown long enough to be read, then the line
    // says it's the reader's turn.
    p.hear({ type: "events", events: [result("b1", "ok")] });
    p.hear({ type: "session", session: session("idle") });
    expect(p.container.querySelector(".now-line")?.textContent).toContain("Run the tests");
    await act(() => new Promise((r) => setTimeout(r, 700)));
    expect(nowSays(p.container)).not.toContain("Run the tests");
    // The same line: whose turn it is rolls in.
    expect(nowSays(p.container)).toContain("Your turn");
    expect(p.container.querySelector(".your-turn")).not.toBeNull();
  });

  it("names the background task it waits for, and opens into a list of them that goes to each", async () => {
    const scrolled = vi.fn();
    vi.stubGlobal("scrollTo", scrolled);
    const p = page();
    p.snapshot(
      [
        ev("user_message", { text: "Start the dev stack, then wait for it", source: "web" }),
        assistant("m-bg", [
          {
            type: "tool_use",
            id: "bg1",
            name: "Bash",
            input: { command: "npm run dev", description: "Start the dev stack", run_in_background: true },
          },
        ]),
        ev("claude", {
          type: "system",
          subtype: "task_started",
          task_id: "t1",
          tool_use_id: "bg1",
          description: "Start the dev stack",
          task_type: "local_bash",
          is_backgrounded: true,
        }),
        result("bg1", "Command running in background with ID: t1"),
        assistant("m-said", [{ type: "text", text: "It runs: I'll wait for it." }]),
        ev("claude", { type: "result", subtype: "success", duration_ms: 9_000, is_error: false, result: "" }),
      ],
      { ...session("idle"), status_reason: "Waiting for a background task" },
    );
    const line = p.container.querySelector(".now-line");
    expect(line?.textContent).toContain("Waiting for");
    expect(line?.textContent).toContain("Start the dev stack");
    // Its call says so too, on its line.
    const call = () =>
      [...p.container.querySelectorAll(".turns button")].find((b) =>
        b.textContent?.includes("Start the dev stack"),
      );
    expect(call()?.textContent).toContain("In the background");
    const toggle = line?.querySelector<HTMLButtonElement>("button[aria-expanded]");
    expect(toggle?.getAttribute("aria-expanded")).toBe("false");
    act(() => toggle?.click());
    const list = () => p.container.querySelector('[role="dialog"][aria-label="In the background"]');
    expect(toggle?.getAttribute("aria-expanded")).toBe("true");
    expect(list()?.textContent).toContain("1 running");
    expect(list()?.textContent).toContain("npm run dev");
    // Escape puts it away, and the focus goes back to the line.
    act(() => {
      document.dispatchEvent(new KeyboardEvent("keydown", { key: "Escape", bubbles: true }));
    });
    expect(list()).toBeNull();
    expect(document.activeElement).toBe(toggle);
    // A task goes to the call that started it, which opens to what it ran.
    act(() => toggle?.click());
    expect(p.container.querySelector('[data-find^="cmd:"]')).toBeNull();
    act(() => list()?.querySelector<HTMLButtonElement>("li button")?.click());
    expect(list()).toBeNull();
    await act(() => new Promise((r) => setTimeout(r, 100)));
    expect(p.container.querySelector('[data-find^="cmd:"]')?.textContent).toContain("npm run dev");
    expect(scrolled).toHaveBeenCalled();
    // It ends: its line says how long it ran.
    p.hear({
      type: "events",
      events: [
        ev("claude", { type: "system", subtype: "task_notification", task_id: "t1", status: "completed" }),
      ],
    });
    expect(call()?.textContent).not.toContain("In the background");
    expect(call()?.textContent).toMatch(/\d+ s in the background/);
  });

  it("goes on from a background task to the turn that takes up its result, without a false hand-over", async () => {
    const p = page();
    p.snapshot(turn("Start the dev stack"), {
      ...session("idle"),
      status_reason: "Waiting for a background task",
    });
    expect(nowSays(p.container)).toContain("Waiting for a background task");
    // The task ends: for a moment the session waits for a message, then the agent takes it up.
    p.hear({ type: "session", session: { ...session("idle"), status_reason: "Waiting for a message" } });
    p.hear({ type: "session", session: session("running") });
    for (const ms of [100, 400, 400]) {
      await act(() => new Promise((r) => setTimeout(r, ms)));
      expect(p.container.querySelector(".now-line")).not.toBeNull();
      expect(nowSays(p.container)).not.toContain("Your turn");
    }
    expect(nowSays(p.container)).toContain("Working");
    // It hands over: the line says so once that has lasted a moment.
    p.hear({ type: "session", session: { ...session("idle"), status_reason: "Waiting for a message" } });
    await act(() => new Promise((r) => setTimeout(r, 1100)));
    expect(nowSays(p.container)).toContain("Your turn");
  });

  it("says on a folded run's line what its calls left running in the background", () => {
    const p = page();
    p.snapshot(
      [
        ev("user_message", { text: "Start the stack and look around", source: "web" }),
        assistant("m-ls", [{ type: "tool_use", id: "ls1", name: "Bash", input: { command: "ls" } }]),
        result("ls1", "src"),
        assistant("m-bg", [
          {
            type: "tool_use",
            id: "bg1",
            name: "Bash",
            input: { command: "npm run dev", description: "Start the dev stack", run_in_background: true },
          },
        ]),
        ev("claude", {
          type: "system",
          subtype: "task_started",
          task_id: "t1",
          tool_use_id: "bg1",
          task_type: "local_bash",
        }),
        result("bg1", "Command running in background with ID: t1"),
      ],
      session("running"),
    );
    const run = [...p.container.querySelectorAll(".turns button")].find((b) =>
      b.textContent?.startsWith("Ran 2 commands"),
    );
    expect(run?.textContent).toContain("1 in the background");
  });

  it("looks further back for the background tasks it waits for that started before what loaded", () => {
    const p = page();
    p.hear({
      type: "snapshot",
      session: { ...session("idle"), status_reason: "Waiting for a background task" },
      role: "owner",
      events: turn("Watch the deploy"),
      has_more_before: true,
      next_seq: seq + 1,
      live: [],
      viewers: [],
      runner_connected: true,
    });
    // It doesn't know which: its status says.
    const line = p.container.querySelector(".now-line");
    expect(line?.textContent).toContain("Waiting for a background task");
    act(() => line?.querySelector<HTMLButtonElement>("button[aria-expanded]")?.click());
    expect(p.container.querySelector('[aria-label="In the background"]')?.textContent).toContain(
      "Looking further back for it",
    );
  });

  it("shows a thought streaming in once, above the message box, where its tokens count up in place", async () => {
    const p = page();
    p.snapshot([ev("user_message", { text: "Think it through", source: "web" })], session("running"));
    const stream = (event: unknown) => p.hear({ type: "stream", event } as unknown as G.ViewerServer);
    stream({ type: "stream_event", event: { type: "message_start", message: { id: "m-think" } } });
    stream({
      type: "stream_event",
      event: { type: "content_block_start", index: 0, content_block: { type: "thinking" } },
    });
    stream({ type: "system", subtype: "thinking_tokens", estimated_tokens: 4180 });
    await act(() => new Promise((r) => setTimeout(r, 700)));
    const line = () => p.container.querySelector(".now-line");
    const shown = () => nowSays(p.container);
    expect(shown()).toContain("Thinking");
    expect(shown()).toContain("4.2K tokens");
    // Not in the transcript as well.
    expect(p.container.querySelector(".turns")?.textContent).not.toContain("Thinking");
    // As it goes on, its figures roll where they are: the step stays.
    const step = line()?.querySelector(".now-row > span:nth-child(2) > span");
    stream({ type: "system", subtype: "thinking_tokens", estimated_tokens: 4310 });
    stream({ type: "system", subtype: "thinking_tokens", estimated_tokens: 4330 });
    // At most as often as each roll can be read.
    await act(() => new Promise((r) => setTimeout(r, 500)));
    expect(shown()).toContain("4.3K tokens");
    expect(line()?.querySelector(".now-row > span:nth-child(2) > span")).toBe(step);
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

  it("asked for the conversation alone while finding, puts the find bar away and shows it", () => {
    const p = page();
    p.snapshot(turn("Fix the flaky login test"));
    const parts = () => p.container.querySelectorAll("[data-part]").length;
    const all = parts();
    p.click('button[aria-label="Show only the conversation"]');
    expect(parts()).toBe(2);
    // Finding shows all of it, and the button says so.
    p.click('button[aria-label^="Find in this session"]');
    expect(parts()).toBe(all);
    expect(p.container.querySelector("input[type=search], input[aria-label]")).not.toBeNull();
    p.click('button[aria-label="Show only the conversation"]');
    expect(parts()).toBe(2);
    expect(
      p.container.querySelector('button[aria-label^="Find in this session"]')?.getAttribute("aria-expanded"),
    ).toBe("false");
  });

  it("marks where what came while the reader was away starts, until they leave again", () => {
    const p = page();
    p.snapshot(turn("Fix the flaky login test"));
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

  it("marks what came while away in the conversation alone too, before the first of it that shows", () => {
    const p = page();
    p.snapshot(
      [
        ev("user_message", { text: "Run the tests", source: "web" }),
        assistant("m1", [{ type: "text", text: "On it." }]),
      ],
      session("running"),
    );
    p.click('button[aria-label="Show only the conversation"]');
    visibility("hidden");
    // The agent's work first, which the conversation leaves out, then what it said.
    p.hear({
      type: "events",
      events: [
        assistant("m2", [{ type: "tool_use", id: "b9", name: "Bash", input: { command: "npm test" } }]),
        result("b9", "ok"),
        assistant("m3", [{ type: "text", text: "The tests pass." }]),
      ],
    });
    visibility("visible");
    const line = p.container.querySelector(".new-line");
    expect(line?.nextElementSibling?.textContent).toContain("The tests pass.");
    // All of it again: the line goes back before the work.
    p.click(`button[aria-label="Show the agent's work"]`);
    expect(p.container.querySelector(".new-line")?.nextElementSibling?.textContent).not.toContain(
      "The tests pass.",
    );
  });

  it("marks what came while away when it starts with the turn's end, once", () => {
    const p = page();
    p.snapshot(
      [
        ev("user_message", { text: "Run the tests", source: "web" }),
        assistant("m1", [{ type: "text", text: "They pass." }]),
      ],
      session("running"),
    );
    visibility("hidden");
    p.hear({
      type: "events",
      events: [ev("claude", { type: "result", subtype: "success", duration_ms: 4000, is_error: false })],
    });
    visibility("visible");
    expect(p.container.querySelectorAll(".new-line")).toHaveLength(1);
    expect(p.container.querySelector(".new-line")?.nextElementSibling?.classList).toContain("thread-end");
    // A turn that stopped ends with its stop, which also shows as a line of its own: one line still.
    visibility("hidden");
    p.hear({ type: "events", events: [ev("user_message", { text: "Again", source: "web" })] });
    visibility("visible");
    expect(p.container.querySelectorAll(".new-line")).toHaveLength(1);
    visibility("hidden");
    p.hear({ type: "events", events: [ev("status", { status: "stopped", reason: "Stopped by Ada" })] });
    visibility("visible");
    expect(p.container.querySelectorAll(".new-line")).toHaveLength(1);
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

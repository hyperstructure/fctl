import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import type { LiveSession } from "@/lib/live";
import { keys, queryClient } from "@/lib/queries";
import { SessionPage } from "./session";

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

/**
 * The page at `/s/ses_a`, which links to `/s/ses_b`; `go` follows a link (or Back) to another.
 * `first`: the session as the cache has it before its stream says anything.
 */
function page(first: G.Session = session()) {
  queryClient.setQueryData(keys.session("ses_a"), first);
  queryClient.setQueryData(keys.session("ses_b"), {
    ...session(),
    id: "ses_b",
    number: 13,
    title: "Review of web#40",
  });
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const render = () =>
    act(() =>
      root.render(
        <QueryClientProvider client={queryClient}>
          <SessionPage />
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
    go(id: string) {
      route.id = id;
      render();
    },
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

/** A key pressed on the page, or in `target`. */
function press(key: string, mods: KeyboardEventInit = {}, target: Element = document.body) {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...mods });
  act(() => {
    target.dispatchEvent(e);
  });
  return e;
}

/** The dialogs open over the page, by their titles. */
const open = () =>
  [...document.querySelectorAll("dialog")]
    .filter((d) => d.open)
    .map((d) => d.querySelector("h2")?.textContent);

describe("the session page", () => {
  it("closes the details sheet when a link in it leads to another session", () => {
    const p = page();
    p.click('button[aria-label="Details"][aria-haspopup="dialog"]');
    expect(open()).toEqual(["Fix the flaky login test"]);

    p.go("ses_b");
    expect(open()).toEqual([]);
    expect(document.querySelector("dialog.sheet")).toBeNull();

    // Back: it doesn't open again.
    p.go("ses_a");
    expect(open()).toEqual([]);
  });

  it("closes the share dialog when Back leads to another session", () => {
    const p = page();
    p.click('button[aria-label="Share"]');
    expect(open()).toEqual(["Share this session"]);

    p.go("ses_b");
    expect(open()).toEqual([]);
  });

  it("keeps the details sheet open while the page shows the same session", () => {
    const p = page();
    p.click('button[aria-label="Details"][aria-haspopup="dialog"]');
    p.go("ses_a");
    expect(open()).toEqual(["Fix the flaky login test"]);
  });

  it("says when the session's runner is short of disk, and when its sandbox is paused for it", async () => {
    const p = page();
    const since = new Date(2026, 10, 6, 14, 40).getTime();
    p.hear({
      type: "snapshot",
      session: { ...session("running"), runner: { id: "rnr_1", name: "now" } },
      role: "owner",
      events: [],
      has_more_before: false,
      next_seq: 1,
      live: [],
      viewers: [],
      runner_connected: true,
      runner_disk: { room: "low", free_mb: 12_595, min_free_mb: 20_480, since },
    });
    expect(p.text()).toContain("Runner low on disk: 12.3 GiB free");
    expect(p.text()).toContain("Runner now has been low on disk since");
    expect(p.text()).toContain("below the 20 GiB its host keeps free");
    // The agent's work still comes through.
    expect(nowSays(p.container)).toMatch(/^Working/);

    p.hear({
      type: "runner_connected",
      connected: true,
      disk: { room: "out", free_mb: 800, min_free_mb: 20_480, since },
      sandbox_paused: { since, reason: "it is nearly out of disk, with 800 MiB free" },
    });
    expect(p.text()).toContain("Runner now has kept this session's sandbox paused since");
    expect(p.text()).toContain("The sandbox keeps what the agent wrote.");
    // Once the step it showed has shown long enough to be read, the line says why.
    await act(() => new Promise((r) => setTimeout(r, 700)));
    expect(nowSays(p.container)).toMatch(/^Sandbox paused/);

    // Room again.
    p.hear({ type: "runner_connected", connected: true });
    await act(() => new Promise((r) => setTimeout(r, 700)));
    expect(p.text()).not.toContain("disk");
    expect(nowSays(p.container)).toMatch(/^Working/);
  });

  it("opens the changes at the first file with c, and says when there are none", () => {
    const p = page();
    press("c");
    expect(open()).toEqual([]);
    expect(document.body.textContent).toContain("No changes yet");
    p.snapshot([
      ev("git", {
        ahead: 1,
        behind: 0,
        uncommitted: 0,
        pushed: true,
        files: [
          { path: "src/login.ts", status: "modified", additions: 3, deletions: 1 },
          { path: "src/login.test.ts", status: "added", additions: 20, deletions: 0 },
        ],
      }),
    ]);
    press("c");
    expect(open()).toEqual(["Changes"]);
    expect(document.querySelector("dialog[open] [aria-current=true]")?.textContent).toContain("login.ts");
    // Its own keys are its own: c there is no shortcut of the page's.
    press("c", {}, document.querySelector("dialog[open]") ?? document.body);
    expect(open()).toEqual(["Changes"]);
  });

  it("shows an autopilot's own session its autopilot, plan and sessions, and leaves out what it doesn't keep", () => {
    const pilot: G.Session = {
      ...session("running"),
      title: "Autopilot of web",
      autopilot: { role: "pilot", project_id: "prj_1", project_slug: "web" },
    };
    const autopilot: G.Autopilot = {
      project_id: "prj_1",
      project_slug: "web",
      scope: "project",
      state: "on",
      owner: { id: "usr_ada", name: "Ada Lovelace" },
      keep_owner: false,
      fresh_asked: false,
      direction: "Ship the checkout.",
      direction_at: 0,
      plan: "- [x] Cart\n- [ ] Apple Pay",
      plan_at: Date.now() - 120_000,
      powers: ["merge"],
      check_in_minutes: 60,
      crew_max: 3,
      session: { id: "ses_a", number: 12, status: "running", started_at: 0 },
      check_ins: 4,
      notes: [],
      asks: [{ id: "apa_1", kind: "question", text: "Which provider?", status: "open", created_at: 0 }],
      crew: [
        {
          id: "ses_c",
          project_id: "prj_1",
          project_slug: "web",
          number: 14,
          title: "Add Apple Pay",
          status: "running",
          created_at: 0,
        },
      ],
      crew_total: 1,
      can: { steer: true, secrets: true, read: true },
      created_at: 0,
      updated_at: 0,
    };
    queryClient.setQueryData(keys.autopilot("web"), autopilot);
    const p = page(pilot);
    p.snapshot([], pilot);
    const rail = () => document.querySelector("aside.lg\\:block")?.textContent ?? "";
    expect(rail()).toContain("At work");
    expect(rail()).toContain("It works in this session now.");
    expect(rail()).toContain("Which provider?");
    expect(rail()).toContain("Wake now");
    expect(rail()).toContain("Its plan");
    expect(rail()).toContain("1 of 2");
    expect(rail()).toContain("Its sessions");
    expect(rail()).toContain("Add Apple Pay");
    // It changed nothing: no changes, and no retrospectives or memory, which are its sessions'.
    expect(rail()).not.toContain("Changes");
    expect(rail()).not.toContain("Retrospective");
    expect(rail()).not.toContain("Memory");

    // Once it made changes, they show.
    p.snapshot(
      [
        ev("git", {
          ahead: 1,
          behind: 0,
          uncommitted: 0,
          pushed: true,
          files: [{ path: "plan.md", status: "added", additions: 3, deletions: 0 }],
        }),
      ],
      pilot,
    );
    expect(rail()).toContain("Changes");

    // An earlier session of its own says where it works now.
    queryClient.setQueryData(keys.autopilot("web"), {
      ...autopilot,
      session: { id: "ses_z", number: 20, status: "running", started_at: 0 },
    });
    p.snapshot([], pilot);
    expect(rail()).toContain("It works in its session #20 now.");
    expect(rail()).toContain("This is an earlier session of its own: fctl wakes it in #20 now.");
  });

  it("shows or hides the details with d: the rail where it fits, else the sheet", () => {
    page();
    const rail = () => !!document.querySelector("aside.lg\\:block");
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("64rem"), media: q }));
    expect(rail()).toBe(true);
    press("d");
    expect(rail()).toBe(false);
    press("d");
    expect(rail()).toBe(true);
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: false, media: q }));
    press("d");
    expect(open()).toEqual(["Fix the flaky login test"]);
    // Again in the sheet, it puts the sheet away, as Escape does: the sheet slides away first.
    const away = (target: Element) => {
      vi.useFakeTimers();
      try {
        press("d", {}, target);
        act(() => vi.advanceTimersByTime(1000));
      } finally {
        vi.useRealTimers();
      }
    };
    away(document.querySelector("dialog.sheet") ?? document.body);
    expect(open()).toEqual([]);
    // Also where a click on the sheet's text left the focus on the page's body.
    press("d");
    expect(open()).toEqual(["Fix the flaky login test"]);
    away(document.body);
    expect(open()).toEqual([]);
  });

  it("leaves d to another dialog over the page", () => {
    const p = page();
    p.click('button[aria-label="Share"]');
    expect(open()).toEqual(["Share this session"]);
    const rail = () => !!document.querySelector("aside.lg\\:block");
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: q.includes("64rem"), media: q }));
    expect(press("d", {}, document.querySelector("dialog[open]") ?? document.body).defaultPrevented).toBe(
      false,
    );
    // Nor from the page's body, where a click on the dialog's text leaves the focus.
    expect(press("d").defaultPrevented).toBe(false);
    expect(rail()).toBe(true);
    expect(open()).toEqual(["Share this session"]);
  });

  it("goes to the message box with r, and Escape leaves it for the page's keys", () => {
    page();
    const box = document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message the agent"]');
    expect(box).not.toBeNull();
    box?.blur();
    press("r");
    expect(document.activeElement).toBe(box);
    // Typed there, r is a letter.
    expect(press("r", {}, box ?? document.body).defaultPrevented).toBe(false);
    press("Escape", {}, box ?? document.body);
    expect(document.activeElement).not.toBe(box);
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

describe("the session in turns", () => {
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

  it("asks the agent's question above the message box where there is room, and at the transcript's end on a phone", () => {
    const asked: G.SessionQuestion = {
      text: "Round Danish prices to whole kroner, or keep øre?",
      options: ["Round to whole kroner", "Keep øre"],
      asked_at: Date.now(),
    };
    const card = (c: HTMLElement) => c.querySelector<HTMLElement>('section[aria-label="The agent asks you"]');
    const footer = (c: HTMLElement) =>
      c.querySelector('textarea[aria-label="Message the agent"]')?.closest(".sticky");
    // A computer: over the transcript, which scrolls under it.
    const wide = page();
    wide.snapshot(turn("Price the Danish boxes"), { ...session(), question: asked });
    expect(footer(wide.container)?.contains(card(wide.container))).toBe(true);
    expect(card(wide.container)?.className).toContain("backdrop-blur");
    for (const unmount of roots.splice(0)) unmount();
    // A phone: after the last turn's end, scrolling away with the transcript. While the
    // transcript loads, under its skeleton.
    vi.stubGlobal("matchMedia", (q: string) => ({ matches: false, media: q }));
    const phone = page({ ...session(), question: asked });
    expect(phone.container.querySelector(".animate-shimmer")).not.toBeNull();
    expect(card(phone.container)?.textContent).toContain(asked.text);
    phone.snapshot(turn("Price the Danish boxes"), { ...session(), question: asked });
    const inline = card(phone.container);
    expect(inline?.textContent).toContain(asked.text);
    expect(footer(phone.container)?.contains(inline)).toBe(false);
    const end = phone.container.querySelector(".thread-end");
    expect(
      end && inline && end.compareDocumentPosition(inline) & Node.DOCUMENT_POSITION_FOLLOWING,
    ).toBeTruthy();
    expect(inline?.className).not.toContain("backdrop-blur");
    expect(phone.container.querySelectorAll('section[aria-label="The agent asks you"]')).toHaveLength(1);
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
      "Looking for it earlier in the session",
    );
  });

  it("after a look further back that failed, looks again only when the list opens again", async () => {
    const asked: string[] = [];
    vi.stubGlobal("fetch", (url: RequestInfo | URL) => {
      const u = String(url);
      if (!u.includes("/events")) return new Promise(() => {});
      asked.push(u);
      return Promise.reject(new TypeError("offline"));
    });
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
    // A load fails, and the page shows what the store says then.
    const settle = async () => {
      await act(() => new Promise((r) => setTimeout(r, 30)));
      p.hear({ type: "presence", viewers: [] });
    };
    await settle();
    // The page's own load of earlier activity failed: it doesn't ask again by itself.
    expect(asked).toHaveLength(1);
    const toggle = () => p.container.querySelector<HTMLButtonElement>(".now-line button[aria-expanded]");
    act(() => toggle()?.click());
    await settle();
    await settle();
    expect(asked).toHaveLength(2);
    expect(p.container.querySelector('[aria-label="In the background"]')?.textContent).toContain(
      "It started earlier in the session",
    );
    // Opened again: once more.
    act(() => toggle()?.click());
    act(() => toggle()?.click());
    await settle();
    await settle();
    expect(asked).toHaveLength(3);
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

  it("lists the newest turns beside the transcript, and the earlier ones a click away, without a scroll of its own", () => {
    const p = page();
    p.snapshot(Array.from({ length: 10 }, (_, i) => turn(`Step ${i + 1} of the migration`)).flat());
    const rail = () =>
      [...p.container.querySelectorAll("section")].find((s) => s.textContent?.startsWith("Turns"));
    const rows = () =>
      [...(rail()?.querySelectorAll("button") ?? [])].filter((b) => /Step \d+/.test(b.textContent ?? ""));
    expect(rows().map((b) => b.textContent?.match(/Step (\d+)/)?.[1])).toEqual([
      "5",
      "6",
      "7",
      "8",
      "9",
      "10",
    ]);
    expect(rail()?.querySelector("[class*='overflow-y-auto']")).toBeNull();
    const more = [...(rail()?.querySelectorAll("button") ?? [])].find(
      (b) => b.textContent === "Show 4 earlier turns",
    );
    act(() => more?.click());
    expect(rows()).toHaveLength(10);
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

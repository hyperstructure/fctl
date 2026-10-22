import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { LiveSession, MAX_PAGES, SCREENFUL } from "./live";
import { parts } from "./runs";

const { events } = vi.hoisted(() => ({
  events: vi.fn<(id: string, q: { before?: number; limit?: number }) => Promise<G.EventPage>>(),
}));
vi.mock("./api", () => ({ api: { events }, wsUrl: (path: string) => `ws://fctl.test/api/v1${path}` }));
vi.mock("./queries", () => ({ patchSession: () => {}, queryClient: {} }));

/** The viewer socket: the test hands it the server's frames. */
class Socket {
  static readonly OPEN = 1;
  static last?: Socket;
  readyState = 0;
  onmessage?: (m: { data: string }) => void;
  constructor(readonly url: string) {
    Socket.last = this;
  }
  send() {}
  close() {}
}
vi.stubGlobal("WebSocket", Socket);

/** The session's events, seq 1 to `seq`. */
let history: G.SessionEvent[] = [];
let seq = 0;
const ev = (kind: string, data: unknown): G.SessionEvent =>
  ({ seq: ++seq, ts: 1_000 + seq, kind, data }) as unknown as G.SessionEvent;
const said = (text: string) => ev("user_message", { id: `d${seq}`, text, source: "web" });
const prose = (text: string) =>
  ev("claude", {
    type: "assistant",
    message: { id: `m${seq}`, role: "assistant", content: [{ type: "text", text }] },
    parent_tool_use_id: null,
  });
/**
 * A command, after `thought` events of Claude Code's word on how far the
 * agent's thinking got, which older agents stored and show nothing.
 */
const command = (thought: number) => {
  const thinking = Array.from({ length: thought }, () =>
    ev("claude", { type: "system", subtype: "thinking_tokens", estimated_tokens: 50 }),
  );
  const id = `toolu_${seq}`;
  return [
    ...thinking,
    ev("claude", {
      type: "assistant",
      message: {
        id: `m${seq}`,
        role: "assistant",
        content: [{ type: "tool_use", id, name: "Bash", input: { command: "ls" } }],
      },
      parent_tool_use_id: null,
    }),
    ev("claude", {
      type: "user",
      message: { role: "user", content: [{ type: "tool_result", tool_use_id: id, content: "ok" }] },
      parent_tool_use_id: null,
    }),
  ];
};
/** `n` commands in a row, each after `thought` events of thinking: one part, a run. */
const commands = (n: number, thought = 0) => Array.from({ length: n }, () => command(thought)).flat();
/** `n` parts, three events each: the agent says what it does, then runs a command, and again. */
const working = (n: number) =>
  Array.from({ length: n / 2 }, (_, i) => [prose(`Step ${i}.`), ...command(0)]).flat();

/** Pages of `history`, as the server sends them. */
async function page(_id: string, q: { before?: number; limit?: number }): Promise<G.EventPage> {
  const earlier = history.filter((e) => e.seq < (q.before ?? Number.POSITIVE_INFINITY));
  const events = earlier.slice(-(q.limit ?? 200));
  return { events, has_more: earlier.length > events.length };
}

beforeEach(() => {
  history = [];
  seq = 0;
  events.mockReset();
  events.mockImplementation(page);
});

const stores: LiveSession[] = [];
afterEach(() => {
  for (const s of stores.splice(0)) s.close();
});

/** Opens the session's page: its socket's snapshot has the newest 400 events. */
function open(): LiveSession {
  const store = new LiveSession("ses_1");
  stores.push(store);
  store.connect();
  const recent = history.slice(-400);
  const snapshot = {
    type: "snapshot",
    session: { id: "ses_1", model: "claude-opus-5-5" },
    role: "owner",
    events: recent,
    has_more_before: history.length > recent.length,
    next_seq: seq + 1,
    live: [],
    viewers: [],
    runner_connected: true,
  };
  Socket.last?.onmessage?.({ data: JSON.stringify(snapshot) });
  return store;
}

/** Lets the pages asked for come in. */
const settle = () => new Promise((resolve) => setTimeout(resolve, 0));

const shown = (s: LiveSession) => parts(s.state.items);
/** The calls in the transcript's last part, a run. */
const lastRun = (s: LiveSession) => {
  const p = shown(s).at(-1);
  return p?.kind === "run" ? p.steps.filter((x) => x.kind === "tool").length : 0;
};
/** Where each page asked for started. */
const befores = () => events.mock.calls.map(([, q]) => q.before);

describe("earlier activity", () => {
  it("loads as the session opens when its newest events fold into one line", async () => {
    // The task, then 8 commands, and 14 the agent thought long about: the
    // newest 400 events have only the last few of those.
    history = [said("Zoom images in the modal"), prose("Reading the modal."), ...commands(8)];
    history.push(prose("I'll plan it."), ...commands(14, 40));
    const store = open();
    expect(shown(store)).toHaveLength(1);
    expect(lastRun(store)).toBeLessThan(14);
    expect(store.state.loadingOlder).toBe(true);
    await settle();
    expect(befores()).toEqual([seq - 399]);
    expect(store.state).toMatchObject({ loadingOlder: false, hasMoreBefore: false, firstSeq: 1 });
    expect(store.state.items[0]).toMatchObject({ kind: "user", text: "Zoom images in the modal" });
    expect(shown(store).map((p) => p.kind)).toEqual(["item", "item", "run", "item", "run"]);
    expect(lastRun(store)).toBe(14);
  });

  it("loads nothing more as the session opens when its newest events fill the page", async () => {
    history = [said("Tidy up"), ...commands(200), ...working(SCREENFUL)];
    const store = open();
    expect(shown(store).length).toBeGreaterThan(SCREENFUL);
    await settle();
    expect(events).not.toHaveBeenCalled();
    expect(store.state).toMatchObject({ loadingOlder: false, hasMoreBefore: true });
  });

  it("loads page after page as the session opens, until a screenful shows", async () => {
    // A long run, and plenty before it.
    history = [said("Tidy up"), ...working(300), ...commands(250, 2)];
    const store = open();
    await settle();
    expect(befores()).toEqual([seq - 399, seq - 799]);
    expect(events.mock.calls.map(([, q]) => q.limit)).toEqual([400, 400]);
    expect(store.state).toMatchObject({ hasMoreBefore: true, firstSeq: seq - 1199 });
    expect(shown(store).length).toBeGreaterThan(SCREENFUL);
  });

  it("stops after a few pages that show nothing new, and goes on when the reader asks", async () => {
    history = [said("Tidy up"), ...working(SCREENFUL), ...commands(500, 4)];
    const store = open();
    await settle();
    expect(events).toHaveBeenCalledTimes(MAX_PAGES);
    expect(shown(store)).toHaveLength(1);
    expect(store.state.hasMoreBefore).toBe(true);

    events.mockClear();
    void store.loadOlder();
    expect(store.state.loadingOlder).toBe(true);
    await settle();
    expect(events).toHaveBeenCalledTimes(2);
    expect(store.state).toMatchObject({ loadingOlder: false, hasMoreBefore: false, firstSeq: 1 });
    expect(store.state.items[0]).toMatchObject({ kind: "user", text: "Tidy up" });
    // The task, then what the agent said and did, the last of which the long run goes on.
    expect(shown(store)).toHaveLength(1 + SCREENFUL);
  });

  it("loads a screenful more when the reader asks, in a page when it has that", async () => {
    history = [said("Tidy up"), ...working(300)];
    const store = open();
    await settle();
    expect(events).not.toHaveBeenCalled();
    const before = shown(store).length;
    void store.loadOlder();
    await settle();
    expect(befores()).toEqual([seq - 399]);
    expect(store.state.hasMoreBefore).toBe(false);
    expect(shown(store).length).toBeGreaterThanOrEqual(before + SCREENFUL);
  });

  it("changes the transcript once, when its pages are all in", async () => {
    history = [said("Tidy up"), ...working(SCREENFUL), ...commands(400, 2)];
    const store = open();
    const changes: unknown[] = [];
    store.subscribe(() => {
      if (changes.at(-1) !== store.state.items) changes.push(store.state.items);
    });
    await settle();
    expect(events).toHaveBeenCalledTimes(4);
    expect(changes).toHaveLength(1);
    expect(store.state.items[0]).toMatchObject({ kind: "user" });
  });

  it("keeps the pages that came in before one failed", async () => {
    history = [said("Tidy up"), ...working(SCREENFUL), ...commands(400, 2)];
    events.mockImplementationOnce(page).mockRejectedValueOnce(new Error("You appear to be offline."));
    const store = open();
    await settle();
    expect(events).toHaveBeenCalledTimes(2);
    expect(store.state).toMatchObject({
      loadingOlder: false,
      hasMoreBefore: true,
      firstSeq: seq - 799,
      // What came in shows: the page may go on.
      olderFailed: false,
    });
  });

  it("says when a load brought nothing, until the next one", async () => {
    history = [said("Tidy up"), ...working(SCREENFUL), ...commands(400, 2)];
    events.mockRejectedValueOnce(new Error("You appear to be offline."));
    const store = open();
    await settle();
    // The page doesn't ask again by itself, which would go on as fast as it fails.
    expect(store.state).toMatchObject({ loadingOlder: false, hasMoreBefore: true, olderFailed: true });

    void store.loadOlder();
    expect(store.state).toMatchObject({ loadingOlder: true, olderFailed: false });
    await settle();
    expect(store.state).toMatchObject({ loadingOlder: false, olderFailed: false, firstSeq: 1 });
  });
});

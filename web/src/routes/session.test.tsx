import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { SessionPage } from "./session";

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
      // One for each session, as the real one keeps it while the cached session changes.
      const store = useMemo(() => new live.LiveSession(id, initial), [id]);
      stores.set(id, store as unknown as { onMessage(m: unknown): void });
      return [store.getSnapshot(), store];
    },
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

// No request is answered: the page shows what the cache holds.
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
});

function session(id: string, number: number, title: string): G.Session {
  return {
    id,
    project_id: "prj_1",
    project_slug: "web",
    number,
    title,
    status: "idle",
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

/** The page at `/s/ses_a`, which links to `/s/ses_b`; `go` follows a link (or Back) to another. */
function page() {
  queryClient.setQueryData(keys.session("ses_a"), session("ses_a", 12, "Fix the flaky login test"));
  queryClient.setQueryData(keys.session("ses_b"), session("ses_b", 13, "Review of web#40"));
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
    go(id: string) {
      route.id = id;
      render();
    },
    /** The session's server says this, and the page shows it. */
    hear(m: G.ViewerServer) {
      act(() => stores.get(route.id)?.onMessage(m));
      render();
    },
    text: () => container.textContent ?? "",
    /** Whether the transcript shows the agent at work. */
    working: () =>
      [...container.querySelectorAll(".text-shimmer")].some((e) => e.textContent?.startsWith("Working")),
    click(selector: string) {
      const button = container.querySelector<HTMLButtonElement>(selector);
      expect(button).not.toBeNull();
      act(() => button?.click());
    },
  };
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

describe("SessionPage", () => {
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

  it("says when the session's runner is short of disk, and when its sandbox is paused for it", () => {
    const p = page();
    const running: G.Session = {
      ...session("ses_a", 12, "Fix the flaky login test"),
      status: "running",
      runner: { id: "rnr_1", name: "now" },
    };
    const since = new Date(2026, 10, 6, 14, 40).getTime();
    p.hear({
      type: "snapshot",
      session: running,
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
    expect(p.working()).toBe(true);

    p.hear({
      type: "runner_connected",
      connected: true,
      disk: { room: "out", free_mb: 800, min_free_mb: 20_480, since },
      sandbox_paused: { since, reason: "it is nearly out of disk, with 800 MiB free" },
    });
    expect(p.text()).toContain("Runner now has kept this session's sandbox paused since");
    expect(p.text()).toContain("The sandbox keeps what the agent wrote.");
    expect(p.working()).toBe(false);

    // Room again.
    p.hear({ type: "runner_connected", connected: true });
    expect(p.text()).not.toContain("disk");
    expect(p.working()).toBe(true);
  });

  it("opens the changes at the first file with c, and says when there are none", () => {
    const p = page();
    press("c");
    expect(open()).toEqual([]);
    expect(document.body.textContent).toContain("No changes yet");
    p.hear({
      type: "snapshot",
      session: session("ses_a", 12, "Fix the flaky login test"),
      role: "owner",
      events: [
        {
          seq: 1,
          ts: 0,
          kind: "git",
          data: {
            ahead: 1,
            behind: 0,
            uncommitted: 0,
            pushed: true,
            files: [
              { path: "src/login.ts", status: "modified", additions: 3, deletions: 1 },
              { path: "src/login.test.ts", status: "added", additions: 20, deletions: 0 },
            ],
          },
        },
      ],
      has_more_before: false,
      next_seq: 2,
      live: [],
      viewers: [],
      runner_connected: true,
    });
    press("c");
    expect(open()).toEqual(["Changes"]);
    expect(document.querySelector("dialog[open] [aria-current=true]")?.textContent).toContain("login.ts");
    // Its own keys are its own: c there is no shortcut of the page's.
    press("c", {}, document.querySelector("dialog[open]") ?? document.body);
    expect(open()).toEqual(["Changes"]);
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

  it("keeps the details sheet open while the page shows the same session", () => {
    const p = page();
    p.click('button[aria-label="Details"][aria-haspopup="dialog"]');
    p.go("ses_a");
    expect(open()).toEqual(["Fix the flaky login test"]);
  });
});

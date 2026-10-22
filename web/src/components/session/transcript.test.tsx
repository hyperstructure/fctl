import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode, useContext } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import type { Stall } from "@/lib/stall";
import { emptyLive, type Item, type Step, type Subagent } from "@/lib/transcript";
import { Transcript } from "./transcript";

vi.mock("@/components/ui", async () => {
  const { plainClick } =
    await vi.importActual<typeof import("@/components/ui/lightbox")>("@/components/ui/lightbox");
  const { Gallery, useLightbox } =
    await vi.importActual<typeof import("@/components/ui/gallery")>("@/components/ui/gallery");
  return {
    Avatar: () => null,
    Button: () => null,
    CopyButton: () => null,
    Dialog: () => null,
    Gallery,
    plainClick,
    Spinner: () => <svg className="animate-spin" />,
    toast: vi.fn(),
    useLightbox,
  };
});
vi.mock("@/components/markdown", async () => {
  const { ImageSizes } =
    await vi.importActual<typeof import("@/components/markdown")>("@/components/markdown");
  return {
    CodeBlock: () => null,
    ImageSizes,
    // With the images whose size it knows, which take their room before they load.
    Markdown: function Markdown({ text }: { text: string }) {
      const sizes = useContext(ImageSizes);
      return (
        <p data-sizes={[...sizes].map(([url, s]) => `${url} ${s.width}×${s.height}`).join(",")}>{text}</p>
      );
    },
  };
});
vi.mock("./attachments", () => ({ MessageFiles: () => null }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, hash }: { children: ReactNode; hash?: string }) => (
    <a href={`/p/app?tab=memory${hash ? `#${hash}` : ""}`}>{children}</a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  const client = new QueryClient();
  act(() => root.render(<QueryClientProvider client={client}>{node}</QueryClientProvider>));
  roots.push(() => act(() => root.unmount()));
  return container;
}

let seq = 0;
const tool = (result?: string): Item => ({
  kind: "tool",
  key: `tool-${++seq}`,
  seq,
  ts: 0,
  id: `toolu_${seq}`,
  name: "Bash",
  input: { command: "ls", description: "List files" },
  result: result === undefined ? undefined : { text: result, isError: false },
  children: 0,
});
const turn = (): Item => ({ kind: "turn", key: `turn-${++seq}`, seq, ts: 0, durationMs: 1000, ok: true });
const prose = (text: string): Item => ({ kind: "prose", key: `prose-${++seq}`, seq, ts: 0, text });
const published = (name = "after.png"): Item => ({
  kind: "notice",
  key: `notice-${++seq}`,
  seq,
  ts: 0,
  notice: {
    level: "info",
    code: "screenshot_published",
    message: `Published ${name}`,
    published: {
      url: `https://app.fctl.test/api/v1/published/${"a".repeat(43)}/${name}`,
      name,
      media_type: "image/png",
      size: 245_300,
      width: 1440,
      height: 900,
      expires_at: Date.now() + 86_400_000,
    },
  },
});

const ada: G.UserRef = { id: "usr_ada", name: "Ada" };
const bo: G.UserRef = { id: "usr_bo", name: "Bo" };
const remote = (user?: G.UserRef): Item => ({
  kind: "remote",
  key: `r-${++seq}`,
  seq,
  ts: 0,
  rc: { enabled: true, session_url: "https://claude.ai/code/session_01AbC", user },
});

const stall: Stall = {
  kind: "away",
  runner: "now",
  status: "running",
  since: new Date(2026, 9, 18, 2, 26).getTime(),
  showFrom: 0,
};

function transcript(
  items: Item[],
  status: "running" | "idle",
  withStall?: Stall,
  { onLoadOlder = () => {}, hasMoreBefore = false } = {},
) {
  return render(
    <Transcript
      sessionId="ses_1"
      startedBy={ada}
      items={items}
      live={emptyLive()}
      status={status}
      stall={withStall}
      onLoadOlder={onLoadOlder}
      hasMoreBefore={hasMoreBefore}
      loadingOlder={false}
    />,
  );
}

const spinners = (c: HTMLElement) => c.querySelectorAll(".animate-spin").length;

describe("the transcript", () => {
  it("shows a tool call of the turn in progress as running", () => {
    const c = transcript([tool("done"), tool()], "running");
    expect(spinners(c)).toBe(1);
    expect(c.textContent).toContain("Working");
  });

  it("never shows a tool call of a turn that ended as running, even without its result", () => {
    // A crash, or a result that never reached the control plane.
    const c = transcript([tool(), turn(), tool("done")], "running");
    expect(spinners(c)).toBe(0);
    expect(transcript([tool()], "idle").textContent).not.toContain("Working");
    expect(spinners(transcript([tool()], "idle"))).toBe(0);
  });

  it("says why the agent's work isn't coming through instead of showing it in progress", () => {
    const c = transcript([tool()], "running", stall);
    expect(spinners(c)).toBe(0);
    expect(c.textContent).not.toContain("Working");
    const [title, detail] = [...c.querySelectorAll("[role=status] p")].map((p) => p.textContent);
    expect(title).toMatch(/^Runner now hasn't been heard from since .+\.$/);
    expect(detail).toMatch(/^The agent's work shows up here when it's back/);
  });
});

describe("a subagent", () => {
  const step = (name: string, input: Record<string, string>, result?: string): Step => ({
    kind: "tool",
    key: `step-${++seq}`,
    seq,
    ts: 0,
    id: `toolu_${seq}`,
    name,
    input,
    result: result === undefined ? undefined : { text: result, isError: false },
    children: 0,
  });
  const delegated = (agent: Partial<Subagent>, result?: string, background = false): Item => ({
    kind: "tool",
    key: `agent-${++seq}`,
    seq,
    ts: 0,
    id: `toolu_${seq}`,
    name: "Agent",
    input: {
      description: "Audit the webhooks",
      prompt: "Read the webhook handler and report what it checks.",
      subagent_type: "Explore",
      ...(background ? { run_in_background: true } : {}),
    },
    result: result === undefined ? undefined : { text: result, isError: false },
    children: 2,
    childLast: "Bash",
    agent: {
      steps: [
        step("Read", { file_path: "crates/fctl-control/src/github.rs" }, "fn verify() {}"),
        { kind: "prose", key: `prose-${++seq}`, seq, ts: 0, text: "It verifies signatures first." },
        step("Bash", { command: "grep -rn verify", description: "Find the callers" }, "src/routes.rs:12"),
      ],
      background,
      ...agent,
    },
  });
  const open = (c: HTMLElement, text: string) => {
    const button = [...c.querySelectorAll("button")].find((b) => b.textContent?.startsWith(text));
    act(() => button?.click());
    return button;
  };

  it("opens into what it was asked, what it did and what it reported", () => {
    const c = transcript(
      [delegated({ ended: "completed", durationMs: 12_107 }, "The handler checks signatures first.")],
      "idle",
    );
    expect(c.textContent).toContain("Delegated");
    expect(c.textContent).toContain("2 steps · Bash");
    const row = open(c, "Delegated");
    expect(row?.getAttribute("aria-expanded")).toBe("true");
    const text = c.textContent ?? "";
    expect(text).toContain("Prompt· Explore");
    expect(text).not.toContain("Read the webhook handler and report what it checks.");
    expect(text).toContain("Read…/fctl-control/src/github.rs");
    expect(text).toContain("It verifies signatures first.");
    expect(text).toContain("RanFind the callers");
    expect(text).toContain("The handler checks signatures first.");
    expect(text).toContain("Finished in 12 s");
    open(c, "Prompt");
    expect(c.textContent).toContain("Read the webhook handler and report what it checks.");
    // Its steps open too.
    open(c, "RanFind the callers");
    expect(c.textContent).toContain("src/routes.rs:12");
  });

  it("works in the background while the session waits for it, and its call's answer stays out", () => {
    const launched = "Async agent launched successfully. agentId: a1 (internal ID - do not mention to user.)";
    const item = delegated({}, launched, true);
    const c = transcript([item, turn()], "idle");
    expect(spinners(c)).toBe(1);
    open(c, "Delegated");
    expect(c.textContent).not.toContain("Async agent launched");
    expect(c.querySelector("[data-item]")?.textContent).toContain("Working");
    const stopped = render(
      <Transcript
        sessionId="ses_1"
        startedBy={ada}
        items={[item]}
        live={emptyLive()}
        status="stopped"
        onLoadOlder={() => {}}
        hasMoreBefore={false}
        loadingOlder={false}
      />,
    );
    expect(spinners(stopped)).toBe(0);
    const done = transcript(
      [delegated({ ended: "completed", report: "Two callers." }, launched, true)],
      "idle",
    );
    expect(spinners(done)).toBe(0);
    open(done, "Delegated");
    expect(done.textContent).toContain("Two callers.");
    expect(done.textContent).not.toContain("Working");
  });

  it("shows the step it works on", () => {
    const item = delegated({
      steps: [step("Bash", { command: "cargo test", description: "Run the tests" })],
    });
    const c = transcript([item], "running");
    open(c, "Delegated");
    // The call and its step in progress.
    expect(spinners(c)).toBe(2);
    expect(c.querySelector("[data-item]")?.textContent).not.toContain("Working");
  });

  it("says it failed", () => {
    const c = transcript(
      [delegated({ ended: "failed", durationMs: 3_000 }, "Async agent launched", true)],
      "idle",
    );
    expect(c.querySelector("button")?.className).toContain("text-danger");
    open(c, "Delegated");
    expect(c.textContent).toContain("Failed after 3.0 s");
  });
});

describe("earlier activity", () => {
  // Where the reader's place stays meanwhile: lib/keep-place.test.tsx.
  it("loads when the reader asks for it", () => {
    const onLoadOlder = vi.fn();
    const c = transcript([tool("done")], "idle", undefined, { onLoadOlder, hasMoreBefore: true });
    const button = [...c.querySelectorAll("button")].find((b) => b.textContent === "Show earlier activity");
    act(() => button?.click());
    expect(onLoadOlder).toHaveBeenCalledTimes(1);
  });
});

describe("Remote Control", () => {
  /** A transcript of a session Ada started, as `me` sees it. */
  const seenBy = (me: G.UserId, items: Item[]) =>
    render(
      <Transcript
        sessionId="ses_1"
        startedBy={ada}
        me={me}
        items={items}
        live={emptyLive()}
        status="idle"
        onLoadOlder={() => {}}
        hasMoreBefore={false}
        loadingOlder={false}
      />,
    );
  const followable = (c: HTMLElement) => c.textContent?.includes("Followable in the Claude apps");

  it("is offered only to the person on whose Claude subscription the session ran", () => {
    expect(followable(seenBy(ada.id, [remote(ada)]))).toBe(true);
    expect(followable(seenBy(bo.id, [remote(ada)]))).toBe(false);
    // Bo resumed Ada's session, on Bo's subscription.
    expect(followable(seenBy(bo.id, [remote(bo)]))).toBe(true);
    expect(followable(seenBy(ada.id, [remote(bo)]))).toBe(false);
  });

  it("is the starter's where the control plane doesn't say whose it is", () => {
    const c = seenBy(ada.id, [remote()]);
    expect(c.querySelector("a")?.getAttribute("href")).toBe("claude://code/session_01AbC");
    expect(followable(seenBy(bo.id, [remote()]))).toBe(false);
  });
});

describe("a published screenshot", () => {
  const click = (el: Element, init: MouseEventInit = {}) => {
    const e = new MouseEvent("click", { bubbles: true, cancelable: true, ...init });
    act(() => {
      el.dispatchEvent(e);
    });
    return e;
  };
  const viewer = () => document.querySelector("dialog");

  it("opens over the page, not in a new tab, from its picture and its name", () => {
    const c = transcript([published()], "idle");
    const picture = c.querySelector('a[aria-label="View after.png"]') as HTMLAnchorElement;
    expect(picture.target).toBe("_blank");
    expect(viewer()).toBeNull();
    expect(click(picture).defaultPrevented).toBe(true);
    expect(viewer()?.getAttribute("aria-label")).toBe("after.png");
    for (const unmount of roots.splice(0)) unmount();
    const c2 = transcript([published()], "idle");
    const name = [...c2.querySelectorAll("a")].find(
      (a) => a.textContent === "after.png",
    ) as HTMLAnchorElement;
    expect(click(name).defaultPrevented).toBe(true);
    expect(viewer()?.getAttribute("aria-label")).toBe("after.png");
  });

  it("steps on to the session's next picture", () => {
    const c = transcript([published("before.png"), prose("Pushed."), published("after.png")], "idle");
    click(c.querySelector('a[aria-label="View before.png"]') as HTMLAnchorElement);
    const dialog = viewer() as HTMLDialogElement;
    expect(dialog.querySelector("p[aria-live]")?.textContent).toBe("before.png: 1 of 2");
    act(() => {
      dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(dialog.getAttribute("aria-label")).toBe("after.png");
  });

  it("takes its room in the agent's answers before it loads", () => {
    const c = transcript([published(), prose("The new header: ![after.png](…)")], "idle");
    expect(c.querySelector("p[data-sizes]")?.getAttribute("data-sizes")).toBe(
      `https://app.fctl.test/api/v1/published/${"a".repeat(43)}/after.png 1440×900`,
    );
  });

  it("leaves a click with a modifier key to the browser, which opens a new tab", () => {
    const c = transcript([published()], "idle");
    const picture = c.querySelector('a[aria-label="View after.png"]') as HTMLAnchorElement;
    expect(click(picture, { metaKey: true }).defaultPrevented).toBe(false);
    expect(click(picture, { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(viewer()).toBeNull();
  });
});

const memory = (over: Partial<G.Memory> = {}): G.Memory => ({
  id: "mem_1",
  project_id: "prj_1",
  text: "The e2e tests need Chromium: run npx playwright install chromium first.",
  session: { id: "ses_1", number: 3, title: "Fix the login redirect" },
  created_at: 0,
  ...over,
});
const kept = (code: string, m: G.Memory): Item => ({
  kind: "notice",
  key: `notice-${++seq}`,
  seq,
  ts: 0,
  notice: { level: "info", code, message: `Remembered: ${m.text}`, memory: m },
});

describe("what the agent keeps", () => {
  it("shows a note it remembered, with a link to it in the project's memory", () => {
    const c = render(
      <Transcript
        sessionId="ses_1"
        startedBy={ada}
        items={[kept("memory_added", memory())]}
        live={emptyLive()}
        status="idle"
        projectSlug="app"
        onLoadOlder={() => {}}
        hasMoreBefore={false}
        loadingOlder={false}
      />,
    );
    expect(c.textContent).toContain("Remembered for later agents");
    expect(c.textContent).toContain("The e2e tests need Chromium");
    expect(c.querySelector("a")?.getAttribute("href")).toBe("/p/app?tab=memory#note-mem_1");
  });

  it("says a note it marked stale gave way to a newer one, without its id", () => {
    const old = memory({ stale: { at: 1, reason: "Replaced by mem_2", replaced_by: "mem_2" } });
    const c = transcript([kept("memory_stale", old)], "idle");
    expect(c.textContent).toContain("Marked a note stale");
    expect(c.textContent).toContain("A newer note takes its place.");
    expect(c.textContent).not.toContain("mem_2");
  });

  it("says which note a duplicate repeats, with a link to it", () => {
    const copy = memory({ stale: { at: 1, reason: "Duplicate of mem_0", duplicate_of: "mem_0" } });
    const c = render(
      <Transcript
        sessionId="ses_1"
        startedBy={ada}
        items={[kept("memory_stale", copy)]}
        live={emptyLive()}
        status="idle"
        projectSlug="app"
        onLoadOlder={() => {}}
        hasMoreBefore={false}
        loadingOlder={false}
      />,
    );
    expect(c.textContent).toContain("Marked a note a duplicate");
    expect(c.textContent).toContain("It repeats another note.");
    expect(c.textContent).not.toContain("Duplicate of mem_0");
    const links = [...c.querySelectorAll("a")].map((a) => a.getAttribute("href"));
    expect(links).toContain("/p/app?tab=memory#note-mem_0");
  });

  it("shows a retrospective with its outcome and the parts that say something", () => {
    const r: G.Retrospective = {
      id: "rtr_1",
      project_id: "prj_1",
      session: { id: "ses_1", number: 3, title: "Fix the login redirect" },
      created_at: 0,
      outcome: "blocked",
      worked_on: "Moving the uploads to the new bucket.",
      challenges: "The sandbox has no credentials for it.",
      approach: " ",
    };
    const c = transcript(
      [
        {
          kind: "notice",
          key: "n-retro",
          seq: 99,
          ts: 0,
          notice: { level: "info", code: "retrospective", message: "Recorded", retrospective: r },
        },
      ],
      "idle",
    );
    expect(c.textContent).toContain("Retrospective");
    expect(c.textContent).toContain("Blocked");
    expect(c.textContent).toContain("Worked onMoving the uploads to the new bucket.");
    expect(c.textContent).toContain("ChallengesThe sandbox has no credentials for it.");
    expect(c.textContent).not.toContain("Approach");
  });
});

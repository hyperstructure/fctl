import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import type { Stall } from "@/lib/stall";
import { emptyLive, type Item } from "@/lib/transcript";
import { Transcript } from "./transcript";

vi.mock("@/components/ui", async () => {
  const { plainClick } =
    await vi.importActual<typeof import("@/components/ui/lightbox")>("@/components/ui/lightbox");
  return {
    Avatar: () => null,
    Button: () => null,
    CopyButton: () => null,
    Dialog: () => null,
    Lightbox: ({ file }: { file: { name: string } }) => <dialog open aria-label={file.name} />,
    plainClick,
    Spinner: () => <svg className="animate-spin" />,
    toast: vi.fn(),
  };
});
vi.mock("@/components/markdown", () => ({
  CodeBlock: () => null,
  Markdown: ({ text }: { text: string }) => <p>{text}</p>,
}));
vi.mock("./attachments", () => ({ MessageFiles: () => null }));

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
const published = (): Item => ({
  kind: "notice",
  key: `notice-${++seq}`,
  seq,
  ts: 0,
  notice: {
    level: "info",
    code: "screenshot_published",
    message: "Published after.png",
    published: {
      url: `https://app.fctl.test/api/v1/published/${"a".repeat(43)}/after.png`,
      name: "after.png",
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

function transcript(items: Item[], status: "running" | "idle", withStall?: Stall) {
  return render(
    <Transcript
      sessionId="ses_1"
      startedBy={ada}
      items={items}
      live={emptyLive()}
      status={status}
      stall={withStall}
      onLoadOlder={() => {}}
      hasMoreBefore={false}
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
  const viewer = (c: HTMLElement) => c.querySelector('dialog[aria-label="after.png"]');

  it("opens over the page, not in a new tab, from its picture and its name", () => {
    const c = transcript([published()], "idle");
    const picture = c.querySelector('a[aria-label="View after.png"]') as HTMLAnchorElement;
    expect(picture.target).toBe("_blank");
    expect(viewer(c)).toBeNull();
    expect(click(picture).defaultPrevented).toBe(true);
    expect(viewer(c)).not.toBeNull();
    const c2 = transcript([published()], "idle");
    const name = [...c2.querySelectorAll("a")].find(
      (a) => a.textContent === "after.png",
    ) as HTMLAnchorElement;
    expect(click(name).defaultPrevented).toBe(true);
    expect(viewer(c2)).not.toBeNull();
  });

  it("leaves a click with a modifier key to the browser, which opens a new tab", () => {
    const c = transcript([published()], "idle");
    const picture = c.querySelector('a[aria-label="View after.png"]') as HTMLAnchorElement;
    expect(click(picture, { metaKey: true }).defaultPrevented).toBe(false);
    expect(click(picture, { ctrlKey: true }).defaultPrevented).toBe(false);
    expect(viewer(c)).toBeNull();
  });
});

import { act, type ReactNode, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type { Json } from "@/lib/claude-types";
import type { LiveSession } from "@/lib/live";
import type { Item, Tool } from "@/lib/transcript";
import { FindBar, Reveal, Revealer, useFind, useWanted } from "./find";
import { Run } from "./tool";

vi.mock("@/components/markdown", () => ({
  CodeBlock: ({ code }: { code: string }) => <pre>{code}</pre>,
  Markdown: ({ text }: { text: string }) => <p>{text}</p>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.useRealTimers();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { container, rerender: (n: ReactNode) => act(() => root.render(n)) };
}

const key = (target: Element, k: string, init: KeyboardEventInit = {}) => {
  const e = new KeyboardEvent("keydown", { key: k, bubbles: true, cancelable: true, ...init });
  act(() => {
    target.dispatchEvent(e);
  });
  return e;
};

function type(input: HTMLInputElement, text: string) {
  act(() => {
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const bar = (over: Partial<ReturnType<typeof useFind>["bar"]> = {}): ReturnType<typeof useFind>["bar"] => ({
  bar: { current: null },
  input: { current: null },
  query: "redirect",
  onQuery: vi.fn(),
  position: 3,
  total: 12,
  searched: true,
  more: false,
  searching: false,
  onOlder: vi.fn(),
  onNewer: vi.fn(),
  onClose: vi.fn(),
  ...over,
});

describe("the find bar", () => {
  const count = (c: HTMLElement) => c.querySelector("#find-count")?.textContent;

  it("says where the reader is among the matches", () => {
    const { container, rerender } = render(<FindBar {...bar()} />);
    expect(count(container)).toBe("3 of 12");
    rerender(<FindBar {...bar({ more: true })} />);
    // Earlier activity hasn't been looked in yet.
    expect(count(container)).toBe("3 of 12+");
    rerender(<FindBar {...bar({ total: 0, position: undefined })} />);
    expect(count(container)).toBe("No matches");
    rerender(<FindBar {...bar({ total: 0, position: undefined, more: true, searching: true })} />);
    expect(count(container)).toBe("Looking further back…");
    rerender(<FindBar {...bar({ total: 0, position: undefined, more: true })} />);
    expect(count(container)).toBe("None so far");
    expect(container.querySelector('[aria-label="Look further back"]')).not.toBeNull();
    rerender(<FindBar {...bar({ query: "", searched: false, total: 0, position: undefined })} />);
    expect(count(container)).toBe("");
  });

  it("steps up the page with Enter and ↑, down with Shift+Enter and ↓, and closes with Escape", () => {
    const props = bar();
    const { container } = render(<FindBar {...props} />);
    const input = container.querySelector("input") as HTMLInputElement;
    expect(key(input, "Enter").defaultPrevented).toBe(true);
    key(input, "ArrowUp");
    expect(props.onOlder).toHaveBeenCalledTimes(2);
    key(input, "Enter", { shiftKey: true });
    key(input, "ArrowDown");
    expect(props.onNewer).toHaveBeenCalledTimes(2);
    key(input, "Escape");
    expect(props.onClose).toHaveBeenCalled();
    // Not while a word is being composed (Japanese, say).
    key(input, "Enter", { isComposing: true });
    expect(props.onOlder).toHaveBeenCalledTimes(2);
  });
});

let seq = 0;
const message = (text: string): Item => ({
  kind: "user",
  key: `u${++seq}`,
  seq,
  ts: seq,
  text,
  source: "web",
});

/** A transcript of messages, with the find bar over it. */
function Page({ items, store }: { items: Item[]; store: LiveSession }) {
  const transcript = useRef<HTMLDivElement>(null);
  const header = useRef<HTMLDivElement>(null);
  const footer = useRef<HTMLDivElement>(null);
  const finder = useFind({
    store,
    items,
    hasMoreBefore: false,
    loadingOlder: false,
    transcript,
    header,
    footer,
  });
  return (
    <>
      <div ref={header}>
        <button type="button" onClick={finder.show}>
          Find
        </button>
        {finder.open && <FindBar {...finder.bar} />}
      </div>
      <div ref={transcript}>
        {items.map((it) => (
          <div key={it.key} data-part={it.key}>
            <div data-find={`msg:${it.key}`}>{it.kind === "user" ? it.text : ""}</div>
          </div>
        ))}
      </div>
      <div ref={footer} />
    </>
  );
}

describe("finding in a session", () => {
  const store = { loadEarlier: vi.fn() } as unknown as LiveSession;
  const items = [message("The login redirect loops"), message("Not this one"), message("Fix the redirect")];

  function opened() {
    const page = render(<Page items={items} store={store} />);
    // ⌘F, or Ctrl+F on Linux.
    expect(key(window as unknown as Element, "f", { ctrlKey: true }).defaultPrevented).toBe(true);
    const input = page.container.querySelector("search input") as HTMLInputElement;
    expect(input).not.toBeNull();
    const count = () => page.container.querySelector("#find-count")?.textContent;
    return { ...page, input, count };
  }

  it("opens on ⌘F and counts the matches, the newest first", () => {
    const { input, count } = opened();
    type(input, "REDIRECT");
    expect(count()).toBe("1 of 2");
    key(input, "Enter");
    expect(count()).toBe("2 of 2");
    // Around at the end.
    key(input, "Enter");
    expect(count()).toBe("1 of 2");
    key(input, "Enter", { shiftKey: true });
    expect(count()).toBe("2 of 2");
  });

  it("steps with ⌘G and ⌘⇧G with the focus elsewhere, and lets the browser find on a second ⌘F", () => {
    const { input, count } = opened();
    type(input, "redirect");
    expect(count()).toBe("1 of 2");
    const page = document.body;
    key(page, "g", { ctrlKey: true });
    expect(count()).toBe("2 of 2");
    key(page, "g", { ctrlKey: true, shiftKey: true });
    expect(count()).toBe("1 of 2");
    act(() => input.focus());
    expect(key(input, "f", { ctrlKey: true }).defaultPrevented).toBe(false);
  });

  it("closes on Escape, and gives the focus back", () => {
    const { container } = render(<Page items={items} store={store} />);
    const opener = container.querySelector("button") as HTMLButtonElement;
    act(() => opener.focus());
    key(window as unknown as Element, "f", { ctrlKey: true });
    const input = container.querySelector("search input") as HTMLInputElement;
    key(input, "Escape");
    expect(container.querySelector("search")).toBeNull();
    expect(document.activeElement).toBe(opener);
  });
});

let n = 0;
const step = (name: string, input: Record<string, Json>): Tool => ({
  kind: "tool",
  key: `t${++n}`,
  seq: n,
  ts: n,
  id: `toolu_${n}`,
  name,
  input,
  result: { text: "", isError: false, ts: n },
  children: 0,
});

describe("a run of the agent's work", () => {
  const steps = [
    step("Bash", { command: "npm test", description: "Run the tests" }),
    step("Read", { file_path: "src/login.ts" }),
    step("Bash", { command: "git status", description: "Check the branch" }),
  ];
  const lines = (c: HTMLElement) => [...c.querySelectorAll("[data-find^='line:']")].map((e) => e.textContent);

  it("shows the steps finding gets to on their own, and opens them for what they open into", () => {
    const reveal = new Revealer();
    const { container } = render(
      <Reveal.Provider value={reveal}>
        <Run steps={steps} running={false} active={false} />
      </Reveal.Provider>,
    );
    // Folded: its line alone.
    expect(lines(container)).toEqual([]);
    act(() => reveal.set(new Map([[steps[1]?.key ?? "", "show"]])));
    expect(lines(container)).toEqual(["src/login.ts"]);
    act(() =>
      reveal.set(
        new Map([
          [steps[1]?.key ?? "", "show"],
          [steps[0]?.key ?? "", "open"],
        ]),
      ),
    );
    expect(lines(container)).toEqual(["Run the tests", "src/login.ts"]);
    expect(container.querySelector(`[data-find="cmd:${steps[0]?.key}"]`)?.textContent).toBe("npm test");
    // What it opened stays open after finding is done.
    act(() => reveal.set(new Map()));
    expect(lines(container)).toEqual(["Run the tests", "src/login.ts"]);
  });
});

/** What finding wants of the lines `keys`, as `key:how` for those it wants something of. */
function Wanted({ keys }: { keys: string[] }) {
  const how = useWanted(keys).split(" ");
  return (
    <output>
      {keys
        .map((k, i) => (how[i] ? `${k}:${how[i]}` : ""))
        .filter(Boolean)
        .join(" ")}
    </output>
  );
}

/** The find bar over a transcript of runs, with what finding wants open. */
function Revealing({ items, store }: { items: Item[]; store: LiveSession }) {
  const transcript = useRef<HTMLDivElement>(null);
  const none = useRef<HTMLDivElement>(null);
  const finder = useFind({
    store,
    items,
    hasMoreBefore: false,
    loadingOlder: false,
    transcript,
    header: none,
    footer: none,
  });
  return (
    <>
      {finder.open && <FindBar {...finder.bar} />}
      <Reveal.Provider value={finder.reveal}>
        <Wanted keys={items.map((it) => it.key)} />
      </Reveal.Provider>
      <div ref={transcript} />
    </>
  );
}

describe("what finding opens", () => {
  const store = { loadEarlier: vi.fn() } as unknown as LiveSession;
  const run = [
    step("Bash", { command: "npm test", description: "Run the tests" }),
    step("Read", { file_path: "src/a.ts" }),
    step("Read", { file_path: "src/b.ts" }),
  ];

  it("waits for typing to pause, and opens what the reader steps to at once", () => {
    vi.useFakeTimers();
    const { container } = render(<Revealing items={run} store={store} />);
    key(window as unknown as Element, "f", { ctrlKey: true });
    const input = container.querySelector("search input") as HTMLInputElement;
    const wanted = () => container.querySelector("output")?.textContent;
    // "src/" matches all three; the newest is the bar's.
    type(input, "src/");
    expect(wanted()).toBe("");
    act(() => vi.advanceTimersByTime(400));
    expect(wanted()).toBe(`${run[2]?.key}:show`);
    key(input, "Enter");
    expect(wanted()).toBe(`${run[1]?.key}:show ${run[2]?.key}:show`);
    // A command opens its call.
    type(input, "npm");
    act(() => vi.advanceTimersByTime(400));
    expect(wanted()).toContain(`${run[0]?.key}:open`);
  });
});

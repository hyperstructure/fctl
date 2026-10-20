import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { useTaskBoxFocus } from "@/lib/task-box";
import { CommandPalette } from "./command-palette";

const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }));
// No request is answered: the palette lists what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({ useNavigate: () => navigate }));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  vi.resetAllMocks();
});

const session: G.Session = {
  id: "ses_1",
  project_id: "prj_1",
  project_slug: "web",
  number: 12,
  title: "Fix the flaky login test",
  status: "running",
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
  },
  archived: false,
};

/** The task box, as the home page shows it under the palette. */
function TaskBox() {
  const ref = useRef<HTMLTextAreaElement>(null);
  useTaskBoxFocus(ref, true);
  return <textarea ref={ref} aria-label="Task for a new session" />;
}

/** ⌘K opened on the home page, with a session to jump to. */
function palette() {
  const client = new QueryClient();
  client.setQueryData<G.SessionList>(keys.sessions({ limit: 50 }), { sessions: [session] });
  const onClose = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <TaskBox />
        <CommandPalette open onClose={onClose} />
      </QueryClientProvider>,
    ),
  );
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const input = document.querySelector<HTMLInputElement>('input[placeholder^="Search"]');
  if (!input) throw new Error("no search field");
  input.focus();
  const entries = () =>
    [...document.querySelectorAll<HTMLButtonElement>("button")].map((b) => b.textContent ?? "");
  const press = (key: string) => {
    const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true });
    act(() => {
      input.dispatchEvent(e);
    });
    return e;
  };
  const box = container.querySelector("textarea");
  return { onClose, entries, press, input, box };
}

describe("⌘K", () => {
  it("lists Home first, before the sessions, while nothing is typed", () => {
    const { entries } = palette();
    expect(entries()[0]).toBe("Homenew session");
    expect(entries()[1]).toBe("Fix the flaky login testweb#12");
  });

  it("goes home on Enter, with the focus in the task box", () => {
    const { onClose, press, input, box } = palette();
    expect(document.activeElement).toBe(input);
    const enter = press("Enter");
    expect(onClose).toHaveBeenCalled();
    expect(navigate).toHaveBeenCalledWith({ to: "/" });
    expect(document.activeElement).toBe(box);
    // Or the key would go on to the task box, as a line break.
    expect(enter.defaultPrevented).toBe(true);
  });

  it("scrolls its list on touch screens with nothing above it that changes", () => {
    const { input } = palette();
    const list = input.parentElement?.nextElementSibling;
    const card = list?.parentElement;
    const backdrop = card?.parentElement;
    const classes = (el: Element | null | undefined) => el?.className.split(" ") ?? [];
    // A swipe past its end stays in it, and its pulsing status dots are drawn inside it.
    expect(classes(list)).toEqual(
      expect.arrayContaining(["overflow-y-auto", "overscroll-contain", "isolate"]),
    );
    // Safari on iPhones and iPads drops a swipe in it when the layers above it change.
    expect(classes(card)).toEqual(
      expect.arrayContaining([
        "pointer-coarse:animate-none",
        "pointer-coarse:backdrop-filter-none",
        "pointer-coarse:overflow-visible",
        "pointer-coarse:bg-surface",
      ]),
    );
    expect(classes(backdrop)).toEqual(
      expect.arrayContaining(["pointer-coarse:animate-fade", "pointer-coarse:backdrop-filter-none"]),
    );
  });

  it("draws its search field in a layer of its own on touch screens", () => {
    const { input } = palette();
    // Safari on iPhones and iPads puts the caret's views in the layer that draws the field:
    // in the palette's, which holds the list, they send a swipe over the list to the page.
    expect(input.className.split(" ")).toContain("pointer-coarse:will-change-transform");
  });
});

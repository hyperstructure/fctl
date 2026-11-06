import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { SessionPage } from "./session";

/** The session the address names. */
const route = vi.hoisted(() => ({ id: "ses_a" }));

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
      const store = useMemo(() => new live.LiveSession(id, initial), [id, initial]);
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
    click(selector: string) {
      const button = container.querySelector<HTMLButtonElement>(selector);
      expect(button).not.toBeNull();
      act(() => button?.click());
    },
  };
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

  it("keeps the details sheet open while the page shows the same session", () => {
    const p = page();
    p.click('button[aria-label="Details"][aria-haspopup="dialog"]');
    p.go("ses_a");
    expect(open()).toEqual(["Fix the flaky login test"]);
  });
});

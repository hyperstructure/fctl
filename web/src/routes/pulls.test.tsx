import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { PullsPage } from "./pulls";

/** The page's search: what it lists. */
const search = vi.hoisted(() => ({
  value: {} as { scope?: string; state?: "open" | "merged" | "closed"; q?: string; mine?: boolean },
}));
/** Where the page went: each search it asked for, worked out from the one before. */
const went = vi.hoisted(() => [] as object[]);

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => search.value,
  useNavigate: () => (to: { search: (s: object) => object }) => {
    went.push(to.search(search.value));
  },
  Link: ({
    children,
    to: _to,
    params: _params,
    search: _search,
    ...rest
  }: { children: ReactNode; to?: string; params?: unknown; search?: unknown } & Record<string, unknown>) => (
    <a href="/s/ses_1" {...rest}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** What the sessions endpoint answers next: a list, or a failure. */
let answer: () => Response;
/** What the page asked for, and what it sent. */
const asked: { url: string; method: string; body?: unknown }[] = [];

beforeEach(() => {
  // Without retries, a failed request fails the query at once.
  queryClient.setQueryDefaults(["sessions"], { retry: false });
  vi.stubGlobal("fetch", (url: string, init?: RequestInit) => {
    asked.push({
      url,
      method: init?.method ?? "GET",
      body: typeof init?.body === "string" ? JSON.parse(init.body) : undefined,
    });
    if (url.startsWith("/api/v1/sessions?")) return Promise.resolve(answer());
    if (url.endsWith("/messages")) return Promise.resolve(Response.json({ delivered: true }));
    return new Promise(() => {});
  });
});

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  queryClient.setQueryDefaults(["sessions"], {});
  vi.unstubAllGlobals();
  search.value = {};
  went.length = 0;
  asked.length = 0;
});

/** A session that finished, with its open pull request. */
const finished = (n: number, number: number, title: string): G.Session => ({
  id: `ses_${n}`,
  project_id: "prj_shop",
  project_slug: "shop",
  number: n,
  title: `Session ${n}`,
  status: "stopped",
  created_by: { id: "usr_noor", name: "Noor Haddad" },
  created_at: n,
  updated_at: n,
  stats: {} as G.SessionStats,
  archived: false,
  pull_requests: [
    {
      number,
      url: `https://github.com/romanesco/shop/pull/${number}`,
      title,
      state: "open",
      repo: { owner: "romanesco", name: "shop" },
    },
  ],
});
const listing = (): Response => Response.json({ sessions: [finished(231, 1291, "Danish for the shop")] });
const failure = (): Response =>
  Response.json({ code: "internal", message: "The database is busy" }, { status: 500 });

async function render() {
  const host = document.createElement("div");
  document.body.append(host);
  const root = createRoot(host);
  await act(async () =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <PullsPage />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    host.remove();
  });
  return host;
}

/** A list of `sessions` to answer with. */
const listOf =
  (...sessions: G.Session[]) =>
  (): Response =>
    Response.json({ sessions });

describe("the pull requests page", () => {
  it("asks for archived sessions' pull requests and those of schedules that hide theirs", async () => {
    answer = listing;
    await render();
    const url = asked.find((a) => a.url.startsWith("/api/v1/sessions?"))?.url ?? "";
    const params = new URLSearchParams(url.split("?")[1]);
    expect(Object.fromEntries(params)).toEqual({ pulls: "any", limit: "200", archived: "1", hidden: "1" });
  });

  it("shows what an agent asks first, and sends the answer picked as the reply", async () => {
    const question = {
      text: "Keep øre, or round to whole kroner?",
      options: ["Keep øre", "Round"],
      asked_at: 1,
    };
    answer = listOf(
      { ...finished(231, 1291, "Danish for the shop"), status: "idle", role: "owner", question },
      finished(232, 1292, "Danish for the box builder"),
    );
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.textContent).toContain("Keep øre, or round")));
    const groups = [...page.querySelectorAll("section h2")].map((h) => h.textContent);
    expect(groups[0]).toContain("Waiting for your answer");
    expect(groups[1]).toContain("Ready for review");
    const round = [...page.querySelectorAll("button")].find((b) => b.textContent === "Round");
    await act(async () => round?.click());
    await act(() => vi.waitFor(() => expect(asked.some((a) => a.url.endsWith("/messages"))).toBe(true)));
    const sent = asked.find((a) => a.url.endsWith("/messages"));
    expect(sent?.method).toBe("POST");
    expect(sent?.url).toBe("/api/v1/sessions/ses_231/messages");
    expect(sent?.body).toMatchObject({ text: "Round" });
    // Once sent, no second answer goes.
    await act(() => vi.waitFor(() => expect(round?.disabled).toBe(true)));
  });

  it("offers no answers to someone who can't message the agent", async () => {
    const question = { text: "Keep øre?", options: ["Keep øre", "Round"], asked_at: 1 };
    answer = listOf({ ...finished(231, 1291, "Danish for the shop"), role: "viewer", question });
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.textContent).toContain("Keep øre?")));
    expect([...page.querySelectorAll("button")].some((b) => b.textContent === "Round")).toBe(false);
    expect(page.textContent).toContain("Open the session");
  });

  it("marks a pull request whose session was archived", async () => {
    search.value = { state: "merged" };
    const merged = finished(231, 1291, "Danish for the shop");
    merged.archived = true;
    merged.pull_requests = merged.pull_requests?.map((pr) => ({
      ...pr,
      state: "merged",
      state_changed_at: 1,
    }));
    answer = listOf(merged);
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.textContent).toContain("Danish for the shop")));
    expect(page.textContent).toContain("Archived");
  });

  it("marks the filter's words, and counts what it finds in each tab", async () => {
    search.value = { q: "danish" };
    const merged = finished(233, 1293, "Danish for the newsletter");
    merged.pull_requests = merged.pull_requests?.map((pr) => ({ ...pr, state: "merged" }));
    answer = listOf(
      finished(231, 1291, "Danish for the shop"),
      finished(232, 1292, "Swedish for the box"),
      merged,
    );
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.querySelectorAll("[data-row]")).toHaveLength(1)));
    expect([...page.querySelectorAll("mark")].map((m) => m.textContent)).toContain("Danish");
    const tabs = [...page.querySelectorAll('[role="tab"]')].map((t) => t.textContent);
    expect(tabs).toEqual(["Open1", "Merged1", "Closed0"]);
  });

  it("says what narrows an empty list, and where the rest of what it finds is", async () => {
    search.value = { q: "newsletter" };
    const merged = finished(233, 1293, "Danish for the newsletter");
    merged.pull_requests = merged.pull_requests?.map((pr) => ({ ...pr, state: "merged" }));
    answer = listOf(finished(231, 1291, "Danish for the shop"), merged);
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.textContent).toContain("None here")));
    expect(page.textContent).toContain("Nothing open matches “newsletter”.");
    const see = [...page.querySelectorAll("button")].find((b) => b.textContent === "See 1 merged");
    await act(async () => see?.click());
    expect(went.at(-1)).toMatchObject({ state: "merged", q: "newsletter" });
  });

  it("moves between its tabs with the arrows", async () => {
    answer = listing;
    const page = await render();
    const open = page.querySelector<HTMLElement>('[role="tab"][aria-selected="true"]');
    expect(open?.textContent).toContain("Open");
    await act(async () => {
      open?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(went.at(-1)).toMatchObject({ state: "merged" });
    await act(async () => {
      open?.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowLeft", bubbles: true }));
    });
    expect(went.at(-1)).toMatchObject({ state: "closed" });
  });

  it("lets a lit row go dark after its moment, whatever else the feed brings meanwhile", async () => {
    const before = finished(231, 1291, "Danish for the shop");
    const other = finished(232, 1292, "Danish for the box builder");
    answer = listOf(before, other);
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.querySelectorAll("[data-row]")).toHaveLength(2)));
    const lit = () => page.querySelector("li.animate-row-moved");
    const refetch = () =>
      act(async () => {
        await queryClient.refetchQueries({ queryKey: ["sessions"] });
        await vi.advanceTimersByTimeAsync(10);
      });
    vi.useFakeTimers({ toFake: ["setTimeout", "clearTimeout"] });
    try {
      const failing = {
        ...before,
        pull_requests: before.pull_requests?.map((pr) => ({
          ...pr,
          checks: { passed: 2, failed: 1, pending: 0, at: 2 } as G.PullRequestChecks,
        })),
      };
      answer = listOf(failing, other);
      await refetch();
      expect(lit()).not.toBeNull();
      // A second later, another session moves on in a way the page doesn't show.
      await act(() => vi.advanceTimersByTimeAsync(1000));
      answer = listOf(failing, { ...other, updated_at: 99, last_activity_at: 99 });
      await refetch();
      expect(lit()).not.toBeNull();
      await act(() => vi.advanceTimersByTimeAsync(1100));
      expect(lit()).toBeNull();
    } finally {
      vi.useRealTimers();
    }
  });

  it("lights a row up as what it shows changes, not as the list first comes", async () => {
    const before = finished(231, 1291, "Danish for the shop");
    answer = listOf(before);
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.querySelectorAll("[data-row]")).toHaveLength(1)));
    const lit = () => page.querySelector("li.animate-row-moved");
    expect(lit()).toBeNull();
    // Its checks fail, as the feed says.
    const failing = {
      ...before,
      pull_requests: before.pull_requests?.map((pr) => ({
        ...pr,
        checks: { passed: 2, failed: 1, pending: 0, at: 2 } as G.PullRequestChecks,
      })),
    };
    answer = listOf(failing);
    await act(() => queryClient.refetchQueries({ queryKey: ["sessions"] }));
    await act(() => vi.waitFor(() => expect(lit()).not.toBeNull()));
    expect(page.textContent).toContain("Checks failing");
  });

  it("moves from the row the focus is in, from its link or from its GitHub arrow", async () => {
    answer = () =>
      Response.json({
        sessions: [
          finished(232, 1292, "Danish for the box builder"),
          finished(231, 1291, "Danish for the shop"),
        ],
      });
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.querySelectorAll("[data-row]")).toHaveLength(2)));
    const [first, second] = [...page.querySelectorAll<HTMLElement>("[data-row]")];
    const arrow = first?.closest("li")?.querySelector<HTMLElement>('a[aria-label$="on GitHub"]');
    const press = (key: string) =>
      act(() => {
        document.activeElement?.dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true }));
      });
    const open = vi.spyOn(window, "open").mockImplementation(() => null);
    arrow?.focus();
    press("o");
    expect(open).toHaveBeenCalledWith(first?.dataset.github, "_blank", "noopener,noreferrer");
    press("j");
    expect(document.activeElement).toBe(second);
    second?.closest("li")?.querySelector<HTMLElement>('a[aria-label$="on GitHub"]')?.focus();
    press("k");
    expect(document.activeElement).toBe(first);
    open.mockRestore();
  });

  it("says what it lists by name", async () => {
    answer = listing;
    queryClient.setQueryData(keys.workspaces, {
      workspaces: [{ id: "ws_r", slug: "romanesco", name: "Romanesco" }],
      invitations: [],
      limit: 5,
    } as unknown as G.WorkspaceList);
    queryClient.setQueryData(keys.projects, {
      projects: [{ id: "prj_shop", slug: "shop", name: "Shop" }],
    } as unknown as G.ProjectList);
    const lead = async (scope?: string) => {
      search.value = { scope };
      const page = await render();
      return page.querySelector("header p")?.textContent ?? "";
    };
    expect(await lead()).toContain("What agents opened in your projects,");
    expect(await lead("w:romanesco")).toContain("What agents opened in Romanesco's projects,");
    expect(await lead("p:shop")).toContain("What agents opened in Shop,");
  });

  it("says so when the list doesn't load", async () => {
    answer = failure;
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.textContent).toContain("didn't load")));
    expect(page.textContent).toContain("The pull requests didn't load: The database is busy");
    expect(page.textContent).toContain("Try again");
  });

  it("keeps what loaded when a refresh fails, and says it may be behind", async () => {
    answer = listing;
    const page = await render();
    await act(() => vi.waitFor(() => expect(page.textContent).toContain("Danish for the shop")));

    answer = failure;
    await act(() => queryClient.refetchQueries({ queryKey: ["sessions"] }));
    await act(() => vi.waitFor(() => expect(page.textContent).toContain("didn't refresh")));
    expect(page.textContent).toContain("This list didn't refresh: The database is busy");
    expect(page.textContent).toContain("Danish for the shop");
    expect(page.textContent).not.toContain("didn't load");

    // Once a refresh works again, the note goes.
    answer = listing;
    const again = [...page.querySelectorAll("button")].find((b) => b.textContent === "Try again");
    await act(async () => again?.click());
    await act(() => vi.waitFor(() => expect(page.textContent).not.toContain("didn't refresh")));
    expect(page.textContent).toContain("Danish for the shop");
  });
});

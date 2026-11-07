import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { PullsPage } from "./pulls";

/** The page's search: what it lists. */
const search = vi.hoisted(() => ({ value: {} as { scope?: string } }));

vi.mock("@tanstack/react-router", () => ({
  useSearch: () => search.value,
  useNavigate: () => vi.fn(),
  Link: ({ children }: { children: ReactNode }) => <a href="/s/ses_1">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

/** What the sessions endpoint answers next: a list, or a failure. */
let answer: () => Response;

beforeEach(() => {
  // Without retries, a failed request fails the query at once.
  queryClient.setQueryDefaults(["sessions"], { retry: false });
  vi.stubGlobal("fetch", (url: string) =>
    url.startsWith("/api/v1/sessions?") ? Promise.resolve(answer()) : new Promise(() => {}),
  );
});

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
  queryClient.setQueryDefaults(["sessions"], {});
  vi.unstubAllGlobals();
  search.value = {};
});

const listing = (): Response =>
  Response.json({
    sessions: [
      {
        id: "ses_1",
        project_id: "prj_shop",
        project_slug: "shop",
        number: 231,
        title: "Add Danish to the shop",
        status: "stopped",
        created_by: { id: "usr_noor", name: "Noor Haddad" },
        created_at: 0,
        updated_at: 0,
        stats: {} as G.SessionStats,
        archived: false,
        pull_requests: [
          {
            number: 1291,
            url: "https://github.com/romanesco/shop/pull/1291",
            title: "Danish for the shop",
            state: "open",
            repo: { owner: "romanesco", name: "shop" },
          },
        ],
      } satisfies G.Session,
    ],
  });
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

describe("the pull requests page", () => {
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

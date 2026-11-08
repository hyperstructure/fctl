import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode, useRef } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { usePageTaskBox } from "@/lib/task-box";
import { AppShell } from "./app-shell";

// The page the address names, what the shell shows in it, and where it goes.
const route = vi.hoisted(() => ({ path: "/", page: null as ReactNode, navigate: vi.fn() }));
// No request is answered: the shell shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children, onClick }: { to: string; children: ReactNode; onClick?: () => void }) => (
    <a
      href={to}
      onClick={(e) => {
        e.preventDefault();
        onClick?.();
      }}
    >
      {children}
    </a>
  ),
  Outlet: () => route.page,
  useNavigate: () => route.navigate,
  useRouter: () => ({ history: { push: () => {} } }),
  useRouterState: ({ select }: { select: (s: { location: { pathname: string } }) => unknown }) =>
    select({ location: { pathname: route.path } }),
}));
vi.mock("@/lib/feed", () => ({ useFeed: () => {}, feed: { onPulse: () => () => {} } }));
vi.mock("@/lib/push", () => ({
  syncPush: async () => {},
  onPushOpen: () => () => {},
  forgetPush: async () => {},
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const cleanups: (() => void)[] = [];
afterEach(() => {
  for (const clean of cleanups.splice(0)) clean();
  localStorage.clear();
  route.path = "/";
  route.page = null;
  route.navigate.mockReset();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

const workspace = { id: "ws_frog", slug: "frog", name: "Frog", color: "blue", role: "member" } as G.Workspace;
const project = (slug: string): G.Project =>
  ({
    id: `prj_${slug}`,
    slug,
    name: slug,
    workspace: { id: workspace.id, slug: workspace.slug, name: workspace.name },
    role: "editor",
    settings: {},
  }) as G.Project;

/** A session of Ada's in `project`, which asks her something when `asks`. */
const session = (id: string, project: string, asks?: boolean): G.Session =>
  ({
    id,
    project_id: `prj_${project}`,
    project_slug: project,
    number: 1,
    title: id,
    status: "idle",
    created_by: { id: "usr_ada", name: "Ada Lovelace" },
    created_at: 0,
    updated_at: 1,
    last_activity_at: 1,
    question: asks ? { text: "Which one?", options: [], asked_at: 1 } : undefined,
    archived: false,
  }) as unknown as G.Session;

/** The shell for Ada, beside the page on a wide screen, at `route.path`. */
function shell(cached: (client: QueryClient) => void = () => {}) {
  vi.stubGlobal("matchMedia", (query: string) => ({
    matches: query.includes("min-width"),
    media: query,
    addEventListener: () => {},
    removeEventListener: () => {},
  }));
  const client = new QueryClient();
  client.setQueryData(keys.me, {
    user: { id: "usr_ada", name: "Ada Lovelace", role: "member" },
    claude: [],
  } as unknown as G.Me);
  client.setQueryData(keys.workspaces, { workspaces: [workspace], invitations: [], limit: 5 });
  client.setQueryData(keys.projects, { projects: [project("cook-it"), project("frog-api")] });
  cached(client);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <AppShell />
      </QueryClientProvider>,
    ),
  );
  cleanups.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return { container, client };
}

function press(key: string, mods: KeyboardEventInit = {}, target: Element = document.body) {
  const e = new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true, ...mods });
  act(() => {
    target.dispatchEvent(e);
  });
  return e;
}

/** The dialogs open over the page, by their names. */
const dialogs = () =>
  [...document.querySelectorAll("dialog")]
    .filter((d) => d.open)
    .map((d) => document.getElementById(d.getAttribute("aria-labelledby") ?? "")?.textContent);

const sheet = () =>
  [...document.querySelectorAll("dialog")].find(
    (d) => d.open && d.textContent?.includes("Keyboard shortcuts"),
  );

/** The sheet's groups, by their headings, with what each row does. */
const groups = () =>
  [...(sheet()?.querySelectorAll("section") ?? [])].map((s) => [
    s.querySelector("h3")?.textContent,
    [...s.querySelectorAll("li")].map((li) => li.querySelector("span span")?.textContent),
  ]);

describe("the keyboard shortcuts sheet", () => {
  it("opens with ?, and lists every shortcut where it works", () => {
    shell();
    // ? is Shift and + on a Swedish keyboard.
    press("?", { shiftKey: true });
    expect(dialogs()).toEqual(["Keyboard shortcuts"]);
    const listed = Object.fromEntries(groups());
    expect(listed.Anywhere).toEqual([
      "Search or jump to…",
      "New session",
      "Keyboard shortcuts",
      "Fold or unfold the sidebar",
    ]);
    expect(listed["Go to"]).toEqual(["Agent Floor", "Next in Needs you", "Pull requests", "Settings"]);
    expect(listed["On a session's page"]).toEqual([
      "Message the agent",
      "Open the changes",
      "Show or hide the details",
      "Next turn / previous",
    ]);
  });

  it("opens with ⌘/ from a text box too, and closes with it", () => {
    route.page = <textarea aria-label="Message the agent" />;
    const { container } = shell();
    const box = container.querySelector("textarea");
    if (!box) throw new Error("no text box");
    expect(press("?", { shiftKey: true }, box).defaultPrevented).toBe(false);
    expect(dialogs()).toEqual([]);
    // / is Shift and 7 on a Swedish keyboard.
    press("/", { ctrlKey: true, shiftKey: true }, box);
    expect(dialogs()).toEqual(["Keyboard shortcuts"]);
    const filter = sheet()?.querySelector("input");
    if (!filter) throw new Error("no filter");
    expect(document.activeElement).toBe(filter);
    press("/", { ctrlKey: true, shiftKey: true }, filter);
    expect(dialogs()).toEqual([]);
  });

  it("lists what works on the page it opens on first, after what works anywhere", () => {
    route.path = "/pulls";
    shell();
    press("?", { shiftKey: true });
    expect(groups().map(([title]) => title)).toEqual([
      "Anywhere",
      "Go to",
      "On the pull requests pageHere",
      "On a session's page",
      "Writing",
      "Finding",
      "In the changes",
    ]);
  });

  it("finds a shortcut by what it does or its keys", () => {
    shell();
    press("?", { shiftKey: true });
    const filter = sheet()?.querySelector("input");
    if (!filter) throw new Error("no filter");
    const type = (text: string) =>
      act(() => {
        Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(filter, text);
        filter.dispatchEvent(new Event("input", { bubbles: true }));
      });
    type("fold");
    expect(groups()).toEqual([["Anywhere", ["Fold or unfold the sidebar"]]]);
    type("ctrl+b");
    expect(groups()).toEqual([["Anywhere", ["Fold or unfold the sidebar"]]]);
    type("g then p");
    expect(groups()).toEqual([["Go to", ["Pull requests"]]]);
    type("nothing like it");
    expect(sheet()?.textContent).toContain("No shortcut matches “nothing like it”.");
    // Escape clears the filter before it closes the sheet.
    press("Escape", {}, filter);
    expect(filter.value).toBe("");
  });

  it("opens from ⌘K", () => {
    shell();
    press("k", { ctrlKey: true });
    const entry = [...document.querySelectorAll("button")].find((b) =>
      b.textContent?.startsWith("Keyboard shortcuts"),
    );
    expect(entry?.getAttribute("aria-keyshortcuts")).toBe("? Control+/");
    act(() => entry?.click());
    expect(dialogs()).toEqual(["Keyboard shortcuts"]);
    // The palette goes.
    expect(document.querySelector('input[placeholder^="Search"]')).toBeNull();
  });
});

describe("a new session from anywhere", () => {
  it("opens the Agent Floor's task box over the page with n, in the page's project", () => {
    route.path = "/p/frog-api";
    shell();
    press("n");
    expect(dialogs()).toEqual(["New session"]);
    const box = document.querySelector<HTMLTextAreaElement>(
      'dialog[open] textarea[aria-label="Task for a new session"]',
    );
    expect(document.activeElement).toBe(box);
    const picker = document.querySelector('dialog[open] [role=combobox][aria-label="Workspace and project"]');
    expect(picker?.textContent).toContain("frog-api");
  });

  it("keeps the Agent Floor's draft", () => {
    localStorage.setItem(
      "fctl:draft:usr_ada",
      JSON.stringify({ prompt: "Add a dark mode", project: "cook-it" }),
    );
    route.path = "/p/frog-api";
    shell();
    press("o", { ctrlKey: true, shiftKey: true });
    const box = document.querySelector<HTMLTextAreaElement>("dialog[open] textarea");
    expect(box?.value).toBe("Add a dark mode");
    // Its project: the task was written for it.
    expect(document.querySelector("dialog[open] [role=combobox]")?.textContent).toContain("cook-it");
  });

  it("opens with ⌘⇧O from a text box, where n types", () => {
    route.page = <textarea aria-label="Message the agent" />;
    const { container } = shell();
    const box = container.querySelector("textarea");
    if (!box) throw new Error("no text box");
    expect(press("n", {}, box).defaultPrevented).toBe(false);
    expect(dialogs()).toEqual([]);
    // O is where it is on every Latin layout.
    press("O", { ctrlKey: true, shiftKey: true }, box);
    expect(dialogs()).toEqual(["New session"]);
  });

  it("goes to the task box the page shows instead", () => {
    function Page() {
      const ref = useRef<HTMLTextAreaElement>(null);
      usePageTaskBox(ref, true);
      return <textarea ref={ref} aria-label="Task for a new session" />;
    }
    route.page = <Page />;
    const { container } = shell();
    press("n");
    expect(dialogs()).toEqual([]);
    expect(document.activeElement).toBe(container.querySelector("textarea"));
  });

  it("closes as another page opens, as a session it starts does", async () => {
    route.path = "/p/frog-api";
    const { client } = shell();
    press("n");
    expect(dialogs()).toEqual(["New session"]);
    route.path = "/s/ses_new";
    // The shell renders again for the new page.
    await act(async () => {
      client.setQueryData<G.Me>(keys.me, (me) => me && { ...me, user: { ...me.user, name: "Ada King" } });
    });
    expect(dialogs()).toEqual([]);
  });
});

describe("going places", () => {
  it("goes to the Agent Floor, the pull requests and Settings with g and a letter", () => {
    shell();
    press("g");
    press("f");
    expect(route.navigate).toHaveBeenLastCalledWith({ to: "/" });
    press("g");
    press("p");
    expect(route.navigate).toHaveBeenLastCalledWith({ to: "/pulls", search: {} });
    press("g");
    press("s");
    expect(route.navigate).toHaveBeenLastCalledWith({
      to: "/settings",
      search: { linked: undefined, error: undefined },
    });
  });

  it("goes from one session that needs you to the next with g then n, and round again", () => {
    route.path = "/s/ses_b";
    shell((client) =>
      client.setQueryData(keys.sessions({ status: "active", limit: 50 }), {
        sessions: [session("ses_a", "cook-it", true), session("ses_b", "cook-it", true)],
      }),
    );
    press("g");
    press("n");
    const went = route.navigate.mock.calls.at(-1)?.[0];
    expect(went).toEqual({ to: "/s/$id", params: { id: expect.stringMatching(/^ses_[ab]$/) } });
    // Not the one the page shows.
    expect(went.params.id).not.toBe("ses_b");
  });
});

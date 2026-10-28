import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { ReviewsTab } from "./reviews";

const repo = (name: string): G.RepoRef => ({ owner: "acme", name });
const me = { id: "usr_ada" as G.UserId, name: "Ada" };

// The project's review agents, as the control plane keeps them: what each PATCH sends is kept too.
const state = vi.hoisted(() => ({
  agents: undefined as unknown as G.ReviewAgents,
  sent: [] as G.UpdateReviewAgentsRequest[],
  /** How often the form asked which apps opened pull requests lately. */
  botsAsked: 0,
}));

vi.mock("@/lib/api", () => {
  class ApiFailure extends Error {}
  return {
    ApiFailure,
    api: {
      reviewAgents: async () => state.agents,
      reviews: async () => ({ reviews: [] }),
      reviewBots: async () => {
        state.botsAsked++;
        return { bots: [] };
      },
      reviewBot: async () => {
        throw new ApiFailure("GitHub shows no app by that name.");
      },
      updateReviewAgents: async (_project: string, b: G.UpdateReviewAgentsRequest) => {
        state.sent.push(b);
        const all = [...state.agents.repos, ...state.agents.skipped_repos];
        const skipped = b.skipped_repos?.map((n) => n.toLowerCase());
        state.agents = {
          ...state.agents,
          ...(b.instructions !== undefined && { instructions: b.instructions || undefined }),
          ...(skipped && {
            repos: all.filter((r) => !skipped.includes(`${r.owner}/${r.name}`.toLowerCase())),
            skipped_repos: all.filter((r) => skipped.includes(`${r.owner}/${r.name}`.toLowerCase())),
          }),
        };
        return state.agents;
      },
    },
  };
});
vi.mock("@tanstack/react-router", () => ({
  Link: ({ to, children }: { to: string; children: ReactNode }) => <a href={to}>{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
beforeEach(() => {
  state.sent = [];
  state.botsAsked = 0;
  state.agents = {
    enabled: true,
    operator: me,
    bots: ["dependabot[bot]"],
    repos: [repo("web"), repo("api")],
    skipped_repos: [repo("shared-lib")],
  };
});
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

const project = {
  id: "prj_1",
  slug: "platform",
  name: "Platform",
  role: "owner",
  repo: repo("web"),
  settings: {},
} as G.Project;

/** Lets queries and mutations answer. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

async function render() {
  queryClient.setQueryData(keys.me, { user: me } as G.Me);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <ReviewsTab project={project} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  await settle();
  await settle();
  return container;
}

const button = (c: HTMLElement, text: string) =>
  [...c.querySelectorAll("button")].find((b) => b.textContent?.trim() === text) as HTMLButtonElement;
const switches = (c: HTMLElement) =>
  [...c.querySelectorAll<HTMLButtonElement>('[role="switch"][aria-label^="Review "]')].map(
    (s) => `${s.getAttribute("aria-label")}: ${s.getAttribute("aria-checked") === "true" ? "on" : "off"}`,
  );
const click = (el: HTMLElement) => act(() => el.click());

async function openSettings(c: HTMLElement) {
  click(button(c, "Settings"));
  await settle();
}

async function save(c: HTMLElement) {
  const form = c.querySelector("form") as HTMLFormElement;
  act(() => form.requestSubmit());
  await settle();
  await settle();
}

describe("the repositories review agents review", () => {
  it("say which they review and which they skip", async () => {
    const c = await render();
    const status = c.textContent ?? "";
    expect(status).toContain("They review each new commit of the pull requests in acme/web and acme/api");
    expect(status).toContain("They skip acme/shared-lib.");
  });

  it("each have a switch, the primary repository first", async () => {
    const c = await render();
    await openSettings(c);
    expect(switches(c)).toEqual([
      "Review acme/web: on",
      "Review acme/api: on",
      "Review acme/shared-lib: off",
    ]);
    const rows = [...c.querySelectorAll('ul[aria-label="Repositories they review"] > li')].map(
      (li) => li.textContent,
    );
    expect(rows).toEqual(["acme/webPrimary", "acme/api", "acme/shared-libSkipped"]);
  });

  it("save the ones turned off, in place of those skipped", async () => {
    const c = await render();
    await openSettings(c);
    const save_ = button(c, "Save");
    expect(save_.disabled).toBe(true);
    click(c.querySelector('[aria-label="Review acme/api"]') as HTMLElement);
    click(c.querySelector('[aria-label="Review acme/shared-lib"]') as HTMLElement);
    expect(switches(c)).toEqual([
      "Review acme/web: on",
      "Review acme/api: off",
      "Review acme/shared-lib: on",
    ]);
    expect(save_.disabled).toBe(false);
    await save(c);
    expect(state.sent.at(-1)?.skipped_repos).toEqual(["acme/api"]);
    const status = c.textContent ?? "";
    expect(status).toContain("in acme/web and acme/shared-lib");
    expect(status).toContain("They skip acme/api.");
    // The apps that opened pull requests lately are asked for again, in the repositories reviewed now.
    expect(state.botsAsked).toBe(1);
    await openSettings(c);
    expect(state.botsAsked).toBe(2);
  });

  it("are left as they are when only something else changes", async () => {
    const c = await render();
    await openSettings(c);
    // Turned off and on again: nothing to save.
    const api_ = c.querySelector('[aria-label="Review acme/api"]') as HTMLElement;
    click(api_);
    click(api_);
    expect(button(c, "Save").disabled).toBe(true);
    const field = c.querySelector("textarea") as HTMLTextAreaElement;
    act(() => {
      Object.getOwnPropertyDescriptor(HTMLTextAreaElement.prototype, "value")?.set?.call(
        field,
        "Mind the migrations.",
      );
      field.dispatchEvent(new Event("input", { bubbles: true }));
    });
    await save(c);
    expect(state.sent.at(-1)?.instructions).toBe("Mind the migrations.");
    expect(state.sent.at(-1)).not.toHaveProperty("skipped_repos");
    // Nor are the apps that opened pull requests lately asked for again.
    await openSettings(c);
    expect(state.botsAsked).toBe(1);
  });

  it("say so when they skip every one", async () => {
    state.agents = {
      ...state.agents,
      repos: [],
      skipped_repos: [repo("web"), repo("api")],
      problem:
        "They skip each of this project's repositories, so no pull request gets a review. Its owners turn one back on under Settings.",
    };
    const c = await render();
    const status = c.textContent ?? "";
    // Past the avatar.
    expect(status).toMatch(/They run on \w?your Claude subscription\./);
    expect(status).toContain("They skip each of this project's repositories");
    expect(status).not.toContain("They review each new commit");
  });
});

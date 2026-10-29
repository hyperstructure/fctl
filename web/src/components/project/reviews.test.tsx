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
  reviews: [] as G.Review[],
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
      reviews: async () => ({ reviews: state.reviews }),
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
          ...(b.description_summary !== undefined && { description_summary: b.description_summary }),
          ...(b.drafts !== undefined && { drafts: b.drafts }),
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
  state.reviews = [];
  state.botsAsked = 0;
  state.agents = {
    enabled: true,
    operator: me,
    bots: ["dependabot[bot]"],
    repos: [repo("web"), repo("api")],
    skipped_repos: [repo("shared-lib")],
    drafts: true,
    description_summary: false,
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

async function render(p: G.Project = project) {
  queryClient.setQueryData(keys.me, { user: me } as G.Me);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={queryClient}>
        <ReviewsTab project={p} />
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
  [
    ...c.querySelectorAll<HTMLButtonElement>(
      'ul[aria-label="Repositories they review"] [role="switch"][aria-label^="Review "]',
    ),
  ].map(
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

describe("the app reviews post as on GitHub", () => {
  const inWorkspace = {
    ...project,
    workspace: { id: "wsp_1", slug: "acme-co", name: "Acme", color: "yellow" },
  } as G.Project;
  const app = (slug: string): G.GithubAppRef => ({ slug });
  const reviewApp = (acme: Partial<G.ReviewAppAccount>): G.WorkspaceReviewApp => ({
    configured: true,
    app: app("fctl-review"),
    github_app: app("fctl-sh"),
    accounts: [{ account: "acme", state: "installed", unreached: [], ...acme }],
  });
  const line = (c: HTMLElement) =>
    [...c.querySelectorAll("p")].find((p) => p.textContent?.startsWith("On GitHub"))?.textContent;

  it("is the review app where it reaches each repository they review", async () => {
    queryClient.setQueryData(keys.workspaceReviewApp("wsp_1"), reviewApp({}));
    const c = await render(inWorkspace);
    expect(line(c)).toBe("On GitHub they post as fctl-review, apart from what your agents write as fctl-sh.");
    expect(c.querySelector('a[href*="github-review"]')).toBeNull();
  });

  it("says where it isn't, with the way to install it", async () => {
    queryClient.setQueryData(
      keys.workspaceReviewApp("wsp_1"),
      reviewApp({ state: "not_installed", install_url: "/api/v1/github-review/install?state=s" }),
    );
    const c = await render(inWorkspace);
    expect(line(c)).toBe(
      "On GitHub they post as fctl-sh, as your agents do: fctl-review isn't installed on acme.",
    );
    const install = c.querySelector('a[href*="github-review"]');
    expect(install?.textContent).toBe("Install fctl-review");
  });

  it("names the repositories it doesn't reach, and sends the others to the workspace's admins", async () => {
    queryClient.setQueryData(
      keys.workspaceReviewApp("wsp_1"),
      reviewApp({ state: "partly", unreached: ["api"] }),
    );
    const c = await render(inWorkspace);
    expect(line(c)).toBe(
      "On GitHub they post as fctl-review, but as fctl-sh in acme/api, which it doesn't reach.",
    );
    // No link of their own: the GitHub tab, where its admins add it.
    expect(c.querySelector('a[href*="github-review"]')).toBeNull();
    expect(
      [...c.querySelectorAll("a")]
        .find((a) => a.textContent === "Where it's installed")
        ?.getAttribute("href"),
    ).toBe("/w/$slug");
  });

  it("leaves out the repositories of accounts the workspace no longer works in", async () => {
    // acme/web and acme/api are reviewed; the workspace works in globex alone now.
    queryClient.setQueryData(keys.workspaceReviewApp("wsp_1"), {
      ...reviewApp({}),
      accounts: [{ account: "globex", state: "not_installed", unreached: [] }],
    });
    const c = await render(inWorkspace);
    expect(line(c)).toBeUndefined();
    expect(c.textContent).not.toContain("isn't installed on .");
  });

  it("counts those in accounts it still works in alone", async () => {
    state.agents = { ...state.agents, repos: [repo("web"), { owner: "globex", name: "site" }] };
    queryClient.setQueryData(keys.workspaceReviewApp("wsp_1"), {
      ...reviewApp({}),
      accounts: [{ account: "globex", state: "not_installed", unreached: [] }],
    });
    const c = await render(inWorkspace);
    expect(line(c)).toBe(
      "On GitHub they post as fctl-sh, as your agents do: fctl-review isn't installed on globex.",
    );
  });

  it("claims nothing of all of them where GitHub didn't say for some", async () => {
    const hooli = { owner: "hooli", name: "web" };
    state.agents = { ...state.agents, repos: [repo("web"), hooli] };
    queryClient.setQueryData(keys.workspaceReviewApp("wsp_1"), {
      ...reviewApp({}),
      accounts: [
        { account: "acme", state: "installed", unreached: [] },
        { account: "hooli", state: "unknown", unreached: [] },
      ],
    });
    const c = await render(inWorkspace);
    expect(line(c)).toBeUndefined();
  });

  it("names those that post as the GitHub App alone where GitHub didn't say for others", async () => {
    const hooli = { owner: "hooli", name: "web" };
    state.agents = { ...state.agents, repos: [repo("web"), hooli] };
    queryClient.setQueryData(keys.workspaceReviewApp("wsp_1"), {
      ...reviewApp({ state: "not_installed" }),
      accounts: [
        { account: "acme", state: "not_installed", unreached: [] },
        { account: "hooli", state: "unknown", unreached: [] },
      ],
    });
    const c = await render(inWorkspace);
    expect(line(c)).toBe(
      "On GitHub, reviews in acme/web post as fctl-sh: fctl-review isn't installed on acme.",
    );
  });

  it("isn't said without a review app", async () => {
    queryClient.setQueryData(keys.workspaceReviewApp("wsp_1"), {
      configured: false,
      github_app: app("fctl-sh"),
      accounts: [],
    } satisfies G.WorkspaceReviewApp);
    const c = await render(inWorkspace);
    expect(line(c)).toBeUndefined();
  });
});

describe("drafts", () => {
  const draftsSwitch = (c: HTMLElement) =>
    c.querySelector('[role="switch"][aria-label="Review drafts"]') as HTMLElement;

  it("get reviews until the owners turn them off, which shows when they get one", async () => {
    const c = await render();
    expect(c.textContent).not.toContain("Drafts wait");
    await openSettings(c);
    expect(draftsSwitch(c).getAttribute("aria-checked")).toBe("true");
    expect(c.textContent).toContain("Each new commit of a draft gets a review");
    expect(c.textContent).not.toContain("Ready for reviewfctl review");
    click(draftsSwitch(c));
    expect(draftsSwitch(c).getAttribute("aria-checked")).toBe("false");
    const text = c.textContent ?? "";
    expect(text).toContain("Drafts get no review until they're marked ready for review");
    // A draft's commits get none; marking it ready starts one.
    expect(text).toContain("Opened as a draftNo review");
    expect(text).toContain("2 new commitsNo review");
    expect(text).toContain("Ready for reviewfctl review");
    await save(c);
    expect(state.sent.at(-1)?.drafts).toBe(false);
    expect(c.textContent).toContain(
      "They skip acme/shared-lib. Drafts wait until they're marked ready for review.",
    );
  });

  it("are turned on again as they were turned off", async () => {
    state.agents = { ...state.agents, drafts: false };
    const c = await render();
    expect(c.textContent).toContain("Drafts wait until they're marked ready for review.");
    await openSettings(c);
    expect(button(c, "Save").disabled).toBe(true);
    click(draftsSwitch(c));
    expect(button(c, "Save").disabled).toBe(false);
    // Off and on again: nothing to save.
    click(draftsSwitch(c));
    expect(button(c, "Save").disabled).toBe(true);
    click(draftsSwitch(c));
    await save(c);
    expect(state.sent.at(-1)?.drafts).toBe(true);
    expect(c.textContent).not.toContain("Drafts wait");
  });

  it("say so where no review ran yet", async () => {
    state.agents = { ...state.agents, drafts: false };
    const c = await render();
    expect(c.textContent).toContain(
      "The next commit pushed to a pull request in acme/web and acme/api that isn't a draft starts one.",
    );
  });
});

describe("summaries in pull requests' descriptions", () => {
  const summarySwitch = (c: HTMLElement) =>
    c.querySelector(
      '[role="switch"][aria-label="Sum up pull requests in their descriptions"]',
    ) as HTMLElement;

  it("are off until the owners turn them on, which shows what they add", async () => {
    const c = await render();
    expect(c.textContent).not.toContain("they sum the pull request up");
    await openSettings(c);
    expect(summarySwitch(c).getAttribute("aria-checked")).toBe("false");
    expect(c.textContent).not.toContain("What it does.");
    click(summarySwitch(c));
    expect(summarySwitch(c).getAttribute("aria-checked")).toBe("true");
    // What it looks like on GitHub: a note, led by the risk.
    const text = c.textContent ?? "";
    expect(text).toContain("NoteMedium risk. It changes how uploads retry");
    expect(text).toContain("What it does. Makes the upload worker give up");
    await save(c);
    expect(state.sent.at(-1)?.description_summary).toBe(true);
    expect(c.textContent).toContain(
      "Before each review, they sum the pull request up at the end of its description.",
    );
  });

  it("are turned off as they were turned on", async () => {
    state.agents = { ...state.agents, description_summary: true };
    const c = await render();
    expect(c.textContent).toContain("they sum the pull request up at the end of its description");
    await openSettings(c);
    expect(button(c, "Save").disabled).toBe(true);
    click(summarySwitch(c));
    expect(button(c, "Save").disabled).toBe(false);
    await save(c);
    expect(state.sent.at(-1)?.description_summary).toBe(false);
    expect(c.textContent).not.toContain("they sum the pull request up");
  });

  it("say how risky each reviewed pull request looked at first sight", async () => {
    const reviewed = (n: number, risk?: G.Risk): G.Review => ({
      session_id: `ses_${n}` as G.SessionId,
      session_number: n,
      status: "stopped",
      state: "passed",
      repo: repo("web"),
      number: n,
      url: `https://github.com/acme/web/pull/${n}`,
      title: `Pull request ${n}`,
      draft: false,
      head_sha: "3f2a1c9d00000000000000000000000000000000",
      findings: 0,
      resolved: 0,
      still_open: 0,
      operator: me,
      created_at: 1,
      busy_ms: 0,
      ...(risk && {
        pull_request_summary: { risk, risk_reason: `Why ${risk}.`, overview: "It does things." },
      }),
    });
    state.reviews = [reviewed(1, "critical"), reviewed(2)];
    const c = await render();
    const rows = [...c.querySelectorAll("li")].filter((li) => li.textContent?.includes("Pull request"));
    const risk = rows[0]?.querySelector('[title^="At first sight"]');
    expect(risk?.textContent).toBe("Critical risk");
    expect(risk?.getAttribute("title")).toBe("At first sight: Why critical.");
    expect(risk?.className).toContain("text-danger");
    expect(rows[1]?.textContent).not.toContain("risk");
  });
});

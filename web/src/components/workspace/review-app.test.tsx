import { QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { ReviewAppReturned, ReviewAppSection } from "./review-app";

// No request is answered: the section shows what the cache holds.
vi.mock("@/lib/api", () => {
  const never = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: never, workspaces: never, ApiFailure: class extends Error {} };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

const w = {
  id: "wsp_1",
  slug: "acme",
  name: "Acme",
  color: "yellow",
  role: "admin",
} as unknown as G.Workspace;

const app = (slug: string, id: number): G.GithubAppRef => ({
  slug,
  avatar_url: `https://avatars.githubusercontent.com/in/${id}`,
});

const reviewApp = (accounts: G.ReviewAppAccount[]): G.WorkspaceReviewApp => ({
  configured: true,
  app: app("fctl-review", 2),
  github_app: app("fctl-sh", 1),
  accounts,
});

const accounts: G.ReviewAppAccount[] = [
  {
    account: "acme",
    state: "installed",
    unreached: [],
    install_url: "/api/v1/github-review/install?state=a",
  },
  {
    account: "globex",
    state: "partly",
    unreached: ["docs"],
    install_url: "/api/v1/github-review/install?state=g",
  },
  {
    account: "initech",
    state: "not_installed",
    unreached: [],
    install_url: "/api/v1/github-review/install?state=i",
  },
  { account: "hooli", state: "unknown", unreached: [] },
];

function render(node: React.ReactNode, data?: G.WorkspaceReviewApp) {
  if (data) queryClient.setQueryData(keys.workspaceReviewApp("wsp_1"), data);
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={queryClient}>{node}</QueryClientProvider>));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const rows = (c: HTMLElement) => [...c.querySelectorAll("li")].map((li) => li.textContent);
const links = (c: HTMLElement) =>
  [...c.querySelectorAll("a")].map((a) => [a.textContent, a.getAttribute("href")]);

describe("ReviewAppSection", () => {
  it("says which app posts the reviews on each account, and why", () => {
    const c = render(<ReviewAppSection w={w} admin />, reviewApp(accounts));
    expect(c.textContent).toContain(
      "Review agents post their findings as fctl-review and your agents answer them as fctl-sh, so that each pull request shows who said what. Where fctl-review isn't installed, reviews post as fctl-sh too.",
    );
    expect(rows(c)).toEqual([
      "acmefctl-review reaches every repository Acme's projects use here.Posts asfctl-review",
      "globexExcept in docs, which fctl-review doesn't reach: its reviews post as fctl-sh.Posts asfctl-reviewAdd docs",
      "initechfctl-review isn't installed here.Posts asfctl-shInstall fctl-review",
      "hooliGitHub didn't say whether fctl-review is installed here.",
    ]);
    // Each app's logo, as GitHub shows it.
    const logos = [...c.querySelectorAll("img")].map((i) => i.getAttribute("src"));
    expect(logos).toContain("https://avatars.githubusercontent.com/in/2?s=32");
    expect(logos).toContain("https://avatars.githubusercontent.com/in/1?s=32");
  });

  it("gives admins the links that install it, or choose its repositories, on GitHub", () => {
    const c = render(<ReviewAppSection w={w} admin />, reviewApp(accounts));
    expect(links(c)).toEqual([
      ["", "/api/v1/github-review/install?state=a"],
      ["Add docs", "/api/v1/github-review/install?state=g"],
      ["Install fctl-review", "/api/v1/github-review/install?state=i"],
    ]);
    const install = c.querySelector('a[href$="state=i"]');
    expect(install?.className).toContain("rounded-sm");
    expect(c.querySelector('a[href$="state=a"]')?.getAttribute("aria-label")).toBe(
      "Choose the repositories fctl-review reaches on acme, on GitHub",
    );
  });

  it("tells the others who installs it", () => {
    const c = render(<ReviewAppSection w={w} admin={false} />, reviewApp(accounts));
    expect(links(c)).toEqual([]);
    expect(c.textContent).toContain("Acme's admins install fctl-review from here.");
    const done = render(
      <ReviewAppSection w={w} admin={false} />,
      reviewApp([accounts[0] as G.ReviewAppAccount]),
    );
    expect(done.textContent).not.toContain("admins install");
  });

  it("says why when fctl can't sign in as it, and that the GitHub App posts every review", () => {
    const problem =
      "fctl can't sign in to GitHub as fctl-review, so every review posts as fctl-sh: GitHub refused the key in GITHUB_REVIEW_APP_PRIVATE_KEY (A JSON web token could not be decoded).";
    const c = render(<ReviewAppSection w={w} admin />, {
      ...reviewApp([
        {
          account: "acme",
          state: "unknown",
          unreached: [],
          install_url: "/api/v1/github-review/install?state=a",
        },
        { account: "hooli", state: "unknown", unreached: [] },
      ]),
      problem,
    });
    expect(c.querySelector('[role="alert"]')?.textContent).toBe(problem);
    expect(rows(c)).toEqual([
      "acmefctl can't tell whether fctl-review is installed here.Posts asfctl-sh",
      "hoolifctl can't tell whether fctl-review is installed here.Posts asfctl-sh",
    ]);
    expect(links(c)).toEqual([["", "/api/v1/github-review/install?state=a"]]);
  });

  it("shows nothing without a review app, or an account", () => {
    const none = render(<ReviewAppSection w={w} admin />, {
      configured: false,
      github_app: app("fctl-sh", 1),
      accounts: [],
    });
    expect(none.textContent).toBe("");
    expect(render(<ReviewAppSection w={w} admin />, reviewApp([])).textContent).toBe("");
  });
});

describe("ReviewAppReturned", () => {
  it("says how installing it went, as GitHub shows it now", () => {
    const say = (outcome: Parameters<typeof ReviewAppReturned>[0]["outcome"], account?: string) =>
      render(<ReviewAppReturned w={w} outcome={outcome} account={account} />, reviewApp(accounts))
        .textContent;
    expect(say("review_installed", "ACME")).toBe(
      "fctl-review is installed on acme: its reviews post as fctl-review from the next review on.",
    );
    expect(say("review_installed", "globex")).toBe(
      "fctl-review is installed on globex, but doesn't reach docs, whose reviews still post as fctl-sh. Add it below.",
    );
    expect(say("review_installed", "initech")).toBe(
      "GitHub doesn't show fctl-review on initech yet, so its reviews still post as fctl-sh. If you installed it there, look again in a minute.",
    );
    expect(say("review_changed")).toBe("fctl-review changed on GitHub. Below is where it posts reviews now.");
    expect(say("review_asked", "initech")).toBe(
      "GitHub asked initech's owners to install fctl-review. Once they do, reviews there post as fctl-review; until then, as fctl-sh.",
    );
    expect(say("review_expired", "initech")).toBe(
      "That link to install fctl-review on initech ran out. Try again below.",
    );
    expect(say("review_failed", "initech")).toBe(
      "Installing fctl-review on initech didn't work: Acme doesn't work in that account now, or you aren't one of its admins.",
    );
  });
});

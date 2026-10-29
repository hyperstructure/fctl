import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { PullRequestSummarizedNotice, ReviewFacts } from "./review";

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    params,
    title,
    className,
  }: {
    children: ReactNode;
    to: string;
    params: Record<string, string>;
    title?: string;
    className?: string;
  }) => (
    <a href={to.replace(/\$(\w+)/g, (_, k: string) => params[k] ?? "")} title={title} className={className}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const session = {
  id: "ses_review",
  project_slug: "web",
  status: "stopped",
  review: {
    repo: { owner: "acme", name: "web" },
    number: 12,
    url: "https://github.com/acme/web/pull/12",
    head_sha: "3f2a1c9d00000000000000000000000000000000",
  },
} as G.Session;

const review = (r: Partial<G.Review> = {}): G.Review => ({
  session_id: "ses_review" as G.SessionId,
  session_number: 46,
  status: "stopped",
  state: "commented",
  repo: { owner: "acme", name: "web" },
  number: 12,
  url: "https://github.com/acme/web/pull/12",
  title: "Fix the login redirect",
  draft: false,
  head_sha: "3f2a1c9d00000000000000000000000000000000",
  findings: 1,
  resolved: 0,
  still_open: 0,
  operator: { id: "usr_1" as G.UserId, name: "Ada" },
  created_at: 1,
  busy_ms: 0,
  ...r,
});

const sessionLinks = (c: HTMLElement) =>
  [...c.querySelectorAll("a")].filter((a) => a.getAttribute("href")?.startsWith("/s/"));

describe("ReviewFacts", () => {
  it("links the session whose agent opened the pull request", () => {
    const opened_by = { id: "ses_opener" as G.SessionId, number: 45, title: "Fix the login redirect" };
    const c = render(<ReviewFacts session={session} review={review({ opened_by })} />);
    const [a, ...rest] = sessionLinks(c);
    expect(rest).toEqual([]);
    expect(a?.getAttribute("href")).toBe("/s/ses_opener");
    expect(a?.textContent).toBe("#45 Fix the login redirect");
    expect(a?.getAttribute("title")).toBe("#45 Fix the login redirect");
    expect(a?.parentElement?.textContent).toBe("Opened by#45 Fix the login redirect");
  });

  it("says nothing of it where none of the project's sessions opened it", () => {
    const c = render(<ReviewFacts session={session} review={review()} />);
    expect(sessionLinks(c)).toEqual([]);
    expect(c.textContent).not.toContain("Opened by");
    // Nor while its review loads.
    expect(render(<ReviewFacts session={session} />).textContent).not.toContain("Opened by");
  });
});

const summary: G.PullRequestSummary = {
  risk: "high",
  risk_reason: "It changes who may delete projects.",
  overview: "Lets maintainers delete projects, with `delete_project` checking their role.",
  highlight: { topic: "Data loss", text: "A deleted project's sessions go with it." },
};

describe("a review's summary of its pull request", () => {
  it("shows in the rail as the risk it saw at first sight", () => {
    const c = render(<ReviewFacts session={session} review={review({ pull_request_summary: summary })} />);
    const risk = [...c.querySelectorAll("span")].find((s) => s.textContent === "High risk");
    expect(risk?.parentElement?.textContent).toBe("First lookHigh risk");
    expect(risk?.getAttribute("title")).toBe("It changes who may delete projects.");
    expect(risk?.className).toContain("text-amber");
    expect(render(<ReviewFacts session={session} review={review()} />).textContent).not.toContain(
      "First look",
    );
  });

  it("shows in the transcript as it reads on GitHub", () => {
    const summarized: G.SummarizedPullRequest = {
      repo: { owner: "acme", name: "web" },
      number: 12,
      url: "https://github.com/acme/web/pull/12",
      summary,
    };
    const c = render(<PullRequestSummarizedNotice summarized={summarized} />);
    const text = c.textContent ?? "";
    expect(text).toContain("Summary added to web#12's description");
    // A warning, at high risk, as GitHub shows it.
    expect(text).toContain("Warning");
    expect([...c.querySelectorAll("strong")].map((b) => b.textContent)).toEqual([
      "High risk.",
      "What it does.",
      "Data loss.",
    ]);
    expect(c.querySelector("code")?.textContent).toBe("delete_project");
    expect(c.querySelector('a[href="https://github.com/acme/web/pull/12"]')?.textContent).toContain(
      "View on GitHub",
    );
    const again = render(
      <PullRequestSummarizedNotice
        summarized={{ ...summarized, replaced: true, summary: { ...summary, risk: "critical" } }}
      />,
    );
    expect(again.textContent).toContain("Summary updated in web#12's description");
    expect(again.textContent).toContain("Caution");
    expect(again.textContent).toContain("Critical risk.");
  });
});

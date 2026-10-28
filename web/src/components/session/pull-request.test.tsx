import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { PullRequestChecksIcon, PullRequestReviewLine, PullRequestReviewMark } from "./pull-request";

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    params,
    className,
  }: {
    children: ReactNode;
    params: { id: string };
    className?: string;
  }) => (
    <a href={`/s/${params.id}`} className={className}>
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

const review = (state: G.ReviewState, more: Partial<G.ReviewCheck> = {}): G.ReviewCheck => ({
  state,
  findings: 0,
  still_open: 0,
  session_id: "ses_review",
  ...more,
});

const pr = (r?: G.ReviewCheck, passed = 9, state = "open"): G.PullRequestRef => ({
  number: 302,
  url: "https://github.com/hyperstructure/fctl/pull/302",
  repo: { owner: "hyperstructure", name: "fctl" },
  state,
  checks: { passed, failed: 0, pending: 0, at: 1, review: r },
});

describe("PullRequestReviewMark", () => {
  it("shows an eye while fctl's review waits or reads, and a comment with what it found", () => {
    const reading = render(<PullRequestReviewMark pr={pr(review("running"))} />);
    expect(reading.querySelector("circle.eye-reading")).not.toBeNull();
    const waiting = render(<PullRequestReviewMark pr={pr(review("running", { waiting: true }))} />);
    expect(waiting.querySelector("circle")).not.toBeNull();
    expect(waiting.querySelector(".eye-reading")).toBeNull();
    const found = render(<PullRequestReviewMark pr={pr(review("commented", { findings: 3 }))} />);
    expect(found.textContent).toBe("3");
    expect(found.querySelector("svg")?.getAttribute("class")).toContain("text-blue");
  });

  it("leaves the rest to the checks' mark and the review's line", () => {
    for (const r of [review("passed"), review("stopped"), review("failed"), undefined])
      expect(render(<PullRequestReviewMark pr={pr(r)} />).innerHTML).toBe("");
    // Merged, it no longer matters.
    const merged = pr(review("commented", { findings: 3 }), 9, "merged");
    expect(render(<PullRequestReviewMark pr={merged} />).innerHTML).toBe("");
    // A review that passed passes the checks' mark when nothing else checks it.
    expect(
      render(<PullRequestChecksIcon pr={pr(review("passed"), 0)} />).querySelector("svg"),
    ).not.toBeNull();
    expect(render(<PullRequestChecksIcon pr={pr(review("commented"), 0)} />).innerHTML).toBe("");
  });
});

describe("PullRequestReviewLine", () => {
  it("says how the review stands and opens its session", () => {
    const c = render(<PullRequestReviewLine review={review("commented", { findings: 2 })} />);
    const a = c.querySelector("a");
    expect(a?.getAttribute("href")).toBe("/s/ses_review");
    expect(a?.textContent).toBe("fctl's review agent found 2 issues");
  });

  it("says it without a link where the review's session is unknown", () => {
    const c = render(<PullRequestReviewLine review={review("running", { session_id: undefined })} />);
    expect(c.querySelector("a")).toBeNull();
    expect(c.textContent).toBe("fctl's review agent is reviewing it");
  });
});

import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys, queryClient } from "@/lib/queries";
import { PullRequestChecksIcon, PullRequestReviewLine, PullRequestReviewMark } from "./pull-request";

// The projects of the person who looks, and the reviews they ask for.
const state = vi.hoisted(() => ({ projects: [] as G.Project[] }));
const { startReview, toast } = vi.hoisted(() => ({ startReview: vi.fn(), toast: vi.fn() }));
vi.mock("@/lib/api", () => ({
  ApiFailure: class extends Error {},
  api: { projects: async () => ({ projects: state.projects }), startReview },
}));
vi.mock("@/components/ui", async (actual) => ({
  ...(await actual<typeof import("@/components/ui")>()),
  toast,
}));
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
  queryClient.clear();
  vi.resetAllMocks();
  state.projects = [];
});

function mount(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  const rerender = (n: ReactNode) =>
    act(() => root.render(<QueryClientProvider client={queryClient}>{n}</QueryClientProvider>));
  rerender(node);
  roots.push(() => act(() => root.unmount()));
  return { container, rerender };
}

const render = (node: ReactNode) => mount(node).container;

/** Lets queries and mutations answer, and React render what they say. */
const settle = () => act(() => new Promise<void>((r) => setTimeout(r, 0)));

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
    const r = review("commented", { findings: 2 });
    const c = render(<PullRequestReviewLine pr={pr(r)} review={r} />);
    const a = c.querySelector("a");
    expect(a?.getAttribute("href")).toBe("/s/ses_review");
    expect(a?.textContent).toBe("fctl's review agent found 2 issues");
  });

  it("says it without a link where the review's session is unknown", () => {
    const r = review("running", { session_id: undefined });
    const c = render(<PullRequestReviewLine pr={pr(r)} review={r} />);
    expect(c.querySelector("a")).toBeNull();
    expect(c.textContent).toBe("fctl's review agent is reviewing it");
  });
});

describe("reviewing a pull request again", () => {
  const project = (role: G.ProjectRole) =>
    ({ id: "prj_web", slug: "web", name: "Web", role, settings: {} }) as G.Project;
  const again = (c: HTMLElement) => c.querySelector<HTMLButtonElement>('button[aria-label="Review again"]');

  async function line(r: G.ReviewCheck, role: G.ProjectRole = "editor") {
    state.projects = [project(role)];
    queryClient.setQueryData(keys.projects, { projects: state.projects });
    const m = mount(<PullRequestReviewLine pr={pr(r)} review={r} />);
    await settle();
    return { ...m, again: () => again(m.container) };
  }

  it("asks the review's project for another review once the review ended", async () => {
    startReview.mockResolvedValue({
      repo: { owner: "hyperstructure", name: "fctl" },
      number: 302,
      head_sha: "abc1234def",
    });
    const ended = review("commented", { still_open: 1, project_id: "prj_web" });
    const { again, rerender } = await line(ended);
    const button = again();
    expect(button?.disabled).toBe(false);
    act(() => button?.click());
    await settle();
    expect(startReview).toHaveBeenCalledTimes(1);
    expect(startReview).toHaveBeenCalledWith("prj_web", {
      pull_request: "https://github.com/hyperstructure/fctl/pull/302",
    });
    expect(toast).toHaveBeenCalledWith("Reviewing fctl#302 at abc1234");
    // Asked, it stays off until the line shows the new review, which it doesn't offer to repeat
    // while it runs.
    expect(again()?.disabled).toBe(true);
    const next = review("running", { waiting: true, session_id: "ses_next", project_id: "prj_web" });
    rerender(<PullRequestReviewLine pr={pr(next)} review={next} />);
    expect(again()).toBeNull();
    const done = review("passed", { session_id: "ses_next", project_id: "prj_web" });
    rerender(<PullRequestReviewLine pr={pr(done)} review={done} />);
    expect(again()?.disabled).toBe(false);
  });

  it("says the review started when the line shows it before the answer comes", async () => {
    let answer: (r: unknown) => void = () => {};
    startReview.mockReturnValue(new Promise((r) => (answer = r)));
    const { again, rerender } = await line(review("commented", { still_open: 1, project_id: "prj_web" }));
    act(() => again()?.click());
    await settle();
    // GitHub's word of the new review comes first, and takes the button away.
    const next = review("running", { waiting: true, session_id: "ses_next", project_id: "prj_web" });
    rerender(<PullRequestReviewLine pr={pr(next)} review={next} />);
    expect(again()).toBeNull();
    answer({ repo: { owner: "hyperstructure", name: "fctl" }, number: 302, head_sha: "abc1234def" });
    await settle();
    expect(toast).toHaveBeenCalledWith("Reviewing fctl#302 at abc1234");
  });

  it("says why the review didn't start, and can ask again", async () => {
    startReview.mockRejectedValue(
      new Error("Review agents are off in this project. Its owners turn them on under Reviews."),
    );
    const { again } = await line(review("failed", { project_id: "prj_web" }));
    act(() => again()?.click());
    await settle();
    expect(toast).toHaveBeenCalledWith(
      "Review agents are off in this project. Its owners turn them on under Reviews.",
      "error",
    );
    expect(again()?.disabled).toBe(false);
  });

  it("isn't offered while the review runs, nor to those who can't ask for one", async () => {
    expect((await line(review("running", { project_id: "prj_web" }))).again()).toBeNull();
    expect((await line(review("passed", { project_id: "prj_web" }), "viewer")).again()).toBeNull();
    // The control plane didn't say whose review it was.
    expect((await line(review("passed"))).again()).toBeNull();
    expect(startReview).not.toHaveBeenCalled();
  });
});

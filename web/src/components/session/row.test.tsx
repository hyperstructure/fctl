import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { SessionRow } from "./row";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, params }: { children: ReactNode; params: { id: string } }) => (
    <a href={`/s/${params.id}`}>{children}</a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function row(s: G.Session) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(<SessionRow s={s} />));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const pr = (repo: string, number: number, title?: string): G.PullRequestRef => {
  const [owner = "", name = ""] = repo.split("/");
  return { number, url: `https://github.com/${repo}/pull/${number}`, title, repo: { owner, name } };
};

const session: G.Session = {
  id: "ses_1",
  project_id: "prj_1",
  project_slug: "fctl",
  number: 103,
  title: "Add the app screenshot to the README",
  status: "stopped",
  status_reason: "Stopped after being idle",
  created_by: { id: "usr_ada", name: "Ada Lovelace" },
  created_at: 0,
  updated_at: 0,
  stats: {
    turns: 1,
    input_tokens: 0,
    output_tokens: 0,
    cache_read_tokens: 0,
    tool_calls: 0,
    files_changed: 1,
    additions: 1,
    deletions: 1,
    busy_ms: 0,
    compactions: 0,
  },
  archived: false,
};

describe("a session's row", () => {
  it("opens the session from its title, and each pull request on GitHub", () => {
    const el = row({
      ...session,
      pull_requests: [
        pr("hyperstructure/fctl", 166, "Add a screenshot of app.fctl.sh to the README"),
        pr("hyperstructure/homebrew-tap", 7),
      ],
    });
    const links = [...el.querySelectorAll("a")];
    expect(links.map((a) => [a.textContent, a.getAttribute("href")])).toEqual([
      ["Add the app screenshot to the README", "/s/ses_1"],
      ["fctl#166", "https://github.com/hyperstructure/fctl/pull/166"],
      ["homebrew-tap#7", "https://github.com/hyperstructure/homebrew-tap/pull/7"],
    ]);
    // A link can't hold another: the session's covers the row, and the pull requests' lie over it.
    expect(el.querySelector("a a")).toBeNull();
    const [, first, second] = links;
    expect(first?.target).toBe("_blank");
    expect(first?.rel).toBe("noreferrer");
    expect(first?.title).toBe("hyperstructure/fctl#166: Add a screenshot of app.fctl.sh to the README");
    expect(second?.title).toBe("hyperstructure/homebrew-tap#7");
    expect(el.textContent).toContain("· fctl#166 homebrew-tap#7");
  });

  it("names one pull request, and none without any", () => {
    const one = row({ ...session, pull_request: pr("hyperstructure/fctl", 165) });
    expect(one.textContent).toContain("· fctl#165");
    expect(one.querySelectorAll("a")).toHaveLength(2);
    const none = row(session);
    expect(none.textContent).not.toContain("#165");
    expect(none.querySelectorAll("a")).toHaveLength(1);
  });

  it("shows each one's state in GitHub's colors, and its checks until it merges", () => {
    const el = row({
      ...session,
      pull_requests: [
        {
          ...pr("hyperstructure/fctl", 171),
          state: "open",
          checks: { passed: 3, failed: 1, pending: 0, at: 1 },
        },
        {
          ...pr("hyperstructure/fctl", 172),
          state: "draft",
          checks: { passed: 1, failed: 0, pending: 2, at: 1 },
        },
        {
          ...pr("hyperstructure/fctl", 173),
          state: "queued",
          checks: { passed: 4, failed: 0, pending: 0, at: 1 },
        },
        {
          ...pr("hyperstructure/fctl", 174),
          state: "merged",
          checks: { passed: 4, failed: 0, pending: 0, at: 1 },
        },
        { ...pr("hyperstructure/fctl", 175), state: "closed" },
        pr("hyperstructure/fctl", 176),
      ],
    });
    const links = [...el.querySelectorAll("a")].slice(1);
    const colors = (a: Element) =>
      [...a.querySelectorAll("svg")].map((svg) => svg.getAttribute("class")?.match(/text-[a-z-]+/)?.[0]);
    expect(links.map(colors)).toEqual([
      ["text-pr-open", "text-checks-failed"],
      ["text-pr-draft", "text-checks-pending"],
      ["text-pr-queued", "text-checks-passed"],
      ["text-pr-merged"],
      ["text-pr-closed"],
      ["text-pr-unknown"],
    ]);
    // What the icons say, said to screen readers and on hover.
    expect(links.map((a) => a.getAttribute("aria-label"))).toEqual([
      "fctl#171, open, 1 of 4 checks failed",
      "fctl#172, draft, 2 of 3 checks pending",
      "fctl#173, queued, all 4 checks passed",
      "fctl#174, merged",
      "fctl#175, closed",
      "fctl#176",
    ]);
    expect(links[0]?.getAttribute("title")).toBe("hyperstructure/fctl#171\nOpen · 1 of 4 checks failed");
  });
});

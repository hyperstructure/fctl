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
    expect(el.textContent).toContain("· PRs fctl#166, homebrew-tap#7");
  });

  it("names one pull request, and none without any", () => {
    const one = row({ ...session, pull_request: pr("hyperstructure/fctl", 165) });
    expect(one.textContent).toContain("· PR fctl#165");
    expect(one.querySelectorAll("a")).toHaveLength(2);
    const none = row(session);
    expect(none.textContent).not.toContain("PR");
    expect(none.querySelectorAll("a")).toHaveLength(1);
  });
});

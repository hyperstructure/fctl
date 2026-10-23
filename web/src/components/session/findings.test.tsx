import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { FindingsMessage } from "./findings";

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

function render(node: ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const url = "https://github.com/acme/web/pull/12";
const summary = `${url}#issuecomment-79`;
const review = (findings: G.PassedOnFinding[]): G.PassedOnReview => ({
  repo: { owner: "acme", name: "web" },
  number: 12,
  url,
  title: "Fix the login redirect",
  sha: "3f2a1c9d00000000000000000000000000000000",
  summary_url: summary,
  session: "ses_review",
  findings,
});
/** Each finding's title, and what the line says of it after. */
const items = (c: HTMLElement) =>
  [...c.querySelectorAll("li")].map((li) => [...li.children].map((e) => e.textContent));
const text =
  "fctl's review agent reviewed commit 3f2a1c9 of your pull request acme/web#12, “Fix the login redirect”, and found 2 issues";

describe("review findings in the transcript", () => {
  it("show a line for each finding, with what fctl told the agent a click away", () => {
    const r = review([
      {
        severity: "high",
        title: "Uploads never stop retrying",
        path: "src/sync/upload.rs",
        start_line: 38,
        line: 42,
        url: `${url}#discussion_r101`,
      },
      {
        severity: "low",
        title: "The removed check guarded admins",
        path: "src/auth.rs",
        line: 9,
        deleted: true,
      },
    ]);
    const c = render(<FindingsMessage review={r} text={text} ts={1} />);
    const shown = c.textContent ?? "";
    expect(shown).toContain("fctl's review agent found 2 issues");
    expect(shown).toContain("web#12");
    expect(shown).toContain("3f2a1c9");
    expect(items(c)).toEqual([
      ["Uploads never stop retrying", "High · src/sync/upload.rs:38–42"],
      ["The removed check guarded admins", "Low · src/auth.rs:9 (old version)"],
    ]);
    // Each finding opens its comment, else the review's summary; the agent's name opens its session.
    const links = [...c.querySelectorAll("a")].map((a) => [a.textContent, a.getAttribute("href")]);
    expect(links).toContainEqual(["Uploads never stop retrying", `${url}#discussion_r101`]);
    expect(links).toContainEqual(["The removed check guarded admins", summary]);
    expect(links).toContainEqual(["fctl's review agent", "/s/ses_review"]);
    expect(links).toContainEqual(["3f2a1c9", `${url}/commits/3f2a1c9d00000000000000000000000000000000`]);
    // The message itself, closed until asked for.
    expect(shown).not.toContain("fctl's review agent reviewed commit");
    const told = [...c.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("What fctl told the agent"),
    );
    expect(told?.getAttribute("aria-expanded")).toBe("false");
    act(() => told?.click());
    expect(told?.getAttribute("aria-expanded")).toBe("true");
    expect(c.textContent).toContain("fctl's review agent reviewed commit 3f2a1c9");
  });

  it("say one issue as one, and link the pull request without a summary", () => {
    const r = {
      ...review([{ severity: "medium", title: "A slow query", path: "src/db.rs", line: 3 }]),
      summary_url: undefined,
    };
    const c = render(<FindingsMessage review={r} text={text} ts={1} />);
    expect(c.textContent).toContain("found 1 issue");
    expect(items(c)).toEqual([["A slow query", "Medium · src/db.rs:3"]]);
    const finding = [...c.querySelectorAll("a")].find((a) => a.textContent === "A slow query");
    expect(finding?.getAttribute("href")).toBe(url);
  });
});

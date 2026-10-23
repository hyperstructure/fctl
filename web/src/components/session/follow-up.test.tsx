import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { FollowUpMessage, HeldFollowUp } from "./follow-up";

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

const url = "https://github.com/acme/web/pull/13";
const said = (c: Partial<G.FollowUpComment> & { id: number }): G.FollowUpEvent => ({
  kind: "comment",
  login: "grace",
  url: `${url}#discussion_r${c.id}`,
  body: "",
  ...c,
});
const followUp = (events: G.FollowUpEvent[]): G.FollowUp => ({
  repo: { owner: "acme", name: "web" },
  number: 13,
  url,
  title: "Keep the hash on the login redirect",
  state: "open",
  base: "main",
  events,
});

describe("follow-ups in the transcript", () => {
  it("show a review as GitHub has it, with what fctl told the agent a click away", () => {
    const f = followUp([
      said({
        id: 799,
        review: "changes_requested",
        body: "agent: two small things.",
        url: `${url}#pullrequestreview-799`,
      }),
      said({
        id: 811,
        review_id: 799,
        path: "src/login/redirect.ts",
        start_line: 14,
        line: 18,
        body: "Keep `location.hash`.",
      }),
      said({
        id: 812,
        review_id: 799,
        path: "src/login/redirect.test.ts",
        line: 7,
        deleted: true,
        body: "Add a case.",
      }),
    ]);
    const c = render(
      <FollowUpMessage followUp={f} text="On GitHub, @grace reviewed your pull request." ts={1} />,
    );
    const text = c.textContent ?? "";
    expect(text).toContain("grace");
    expect(text).toContain("reviewed");
    expect(text).toContain("web#13");
    expect(text).toContain("Changes requested");
    expect(text).toContain("agent: two small things.");
    expect(text).toContain("src/login/redirect.ts:14–18");
    expect(text).toContain("src/login/redirect.test.ts:7");
    expect(text).toContain("old version");
    // Its comments on lines open on GitHub, and the review's pull request links to the review.
    const links = [...c.querySelectorAll("a")].map((a) => a.getAttribute("href"));
    expect(links).toContain(`${url}#pullrequestreview-799`);
    expect(links).toContain(`${url}#discussion_r811`);
    // What fctl told the agent opens.
    expect(text).not.toContain("On GitHub, @grace");
    const told = [...c.querySelectorAll("button")].find((b) =>
      b.textContent?.includes("What fctl told the agent"),
    );
    expect(told?.getAttribute("aria-expanded")).toBe("false");
    act(() => told?.click());
    expect(told?.getAttribute("aria-expanded")).toBe("true");
    expect(c.textContent).toContain("On GitHub, @grace reviewed your pull request.");
  });

  it("show pictures in comments as links: the app loads them only from where it trusts", () => {
    const f = followUp([
      said({
        id: 1,
        login: "ada",
        body: "@fctl like this: ![the redirect](https://github.com/user-attachments/assets/a.png)",
      }),
    ]);
    const c = render(<FollowUpMessage followUp={f} text="" ts={1} />);
    // Only the avatar, from GitHub's avatars, shows as a picture.
    const pictures = [...c.querySelectorAll("img")].map((i) => i.getAttribute("src") ?? "");
    expect(pictures.filter((src) => !src.startsWith("https://avatars.githubusercontent.com/"))).toEqual([]);
    const link = [...c.querySelectorAll("a")].find((a) => a.textContent === "the redirect");
    expect(link?.getAttribute("href")).toBe("https://github.com/user-attachments/assets/a.png");
  });

  it("show failed checks by name, and a merge", () => {
    const f = followUp([
      {
        kind: "checks_failed",
        sha: "1bf46d85abe1",
        names: ["Rust tests", "Clippy"],
        passed: 4,
        failed: 2,
        pending: 1,
      },
      { kind: "merged", by: "ada", base: "main" },
      { kind: "other" },
    ]);
    const c = render(<FollowUpMessage followUp={f} text="" ts={1} />);
    const text = c.textContent ?? "";
    expect(text).toContain("2 checks failed");
    expect(text).toContain("Rust tests");
    expect(text).toContain("Clippy");
    expect(text).toContain("1 still running · 4 passed");
    expect(text).toContain("1bf46d8");
    expect(text).toContain("Merged into main by ada");
    const links = [...c.querySelectorAll("a")].map((a) => a.getAttribute("href"));
    expect(links).toContain(`${url}/checks`);
    expect(links).toContain(`${url}/commits/1bf46d85abe1`);
  });

  it("show conflicts with the base branch, at their commit", () => {
    const f = followUp([{ kind: "conflicts", sha: "8e1d2b7c6a90", base: "main" }]);
    const c = render(<FollowUpMessage followUp={f} text="" ts={1} />);
    const text = c.textContent ?? "";
    expect(text).toContain("Conflicts with main");
    expect(text).toContain("web#13");
    expect(text).toContain("8e1d2b7");
    const links = [...c.querySelectorAll("a")].map((a) => a.getAttribute("href"));
    expect(links).toContain(url);
    expect(links).toContain(`${url}/commits/8e1d2b7c6a90`);
    expect(c.querySelector("svg.text-amber")).not.toBeNull();
  });

  it("say why the agent didn't hear of failed checks it held back", () => {
    const f = followUp([
      { kind: "checks_failed", sha: "6a51204d", names: ["Rust tests"], passed: 0, failed: 1, pending: 0 },
    ]);
    const c = render(<HeldFollowUp followUp={f} ts={1} />);
    expect(c.textContent).toContain("A check failed");
    expect(c.textContent).toContain("Not passed on: the agent heard of failed checks three times in a row");
  });

  it("say why the agent didn't hear of conflicts it held back", () => {
    const f = followUp([{ kind: "conflicts", sha: "8e1d2b7c6a90" }]);
    const c = render(<HeldFollowUp followUp={f} ts={1} />);
    expect(c.textContent).toContain("Conflicts with main");
    expect(c.textContent).toContain("Not passed on: the agent heard of conflicts three times in a row");
    expect(c.textContent).toContain("Message it to have it resolve them.");
    // Muted, as what the agent didn't hear of.
    expect(c.querySelector("svg.text-amber")).toBeNull();
  });
});

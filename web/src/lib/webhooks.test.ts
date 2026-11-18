import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  againIn,
  attemptWords,
  describeWebhook,
  hookState,
  hostOf,
  PRESETS,
  phraseOf,
  presetOf,
  pretty,
  statusText,
  urlProblem,
  verifySnippet,
  webhookTitle,
} from "./webhooks";

/** The catalog as the control plane serves it (fctl_domain::hooks::catalog), in its order. */
const CATALOG: G.WebhookEventInfo[] = (
  [
    ["session.started", "sessions", "A session starts", true],
    ["session.finished", "sessions", "An agent finishes a turn", true],
    ["session.ready_for_review", "sessions", "Pull requests are ready for review", true],
    ["session.question", "sessions", "An agent asks something", true],
    ["session.follow_up", "sessions", "A follow-up brings an agent back", true],
    ["session.paused", "sessions", "A session pauses", true],
    ["session.stopped", "sessions", "A session stops", true],
    ["session.failed", "sessions", "A session fails", true],
    ["session.status_changed", "sessions", "A session's status changes", true],
    ["pull_request.opened", "pull_requests", "An agent opens a pull request", true],
    ["pull_request.merged", "pull_requests", "An agent's pull request merges", true],
    ["pull_request.closed", "pull_requests", "An agent's pull request closes", true],
    ["review.queued", "reviews", "A review is queued", false],
    ["review.started", "reviews", "A review starts", false],
    ["review.finished", "reviews", "A review is posted", false],
    ["review.failed", "reviews", "A review fails", false],
    ["review.cancelled", "reviews", "A review is cancelled", false],
    ["schedule.run_skipped", "schedules", "A schedule's run starts nothing", false],
    ["schedule.paused", "schedules", "fctl pauses a schedule", false],
  ] as const
).map(([type, group, label, sessions]) => ({ type, group, label, description: "", sessions }));

const URL = "https://hooks.slack.com/services/x";

describe("describeWebhook", () => {
  // The same sentences as fctl_domain::hooks's a_webhook_is_described_in_a_sentence.
  it("says what a webhook posts and where, as the CLI does", () => {
    expect(describeWebhook(URL, [], [], [], CATALOG)).toBe("Posts every event to hooks.slack.com.");
    expect(
      describeWebhook(
        URL,
        ["session.question", "session.ready_for_review"],
        ["Web", "Docs"],
        ["person"],
        CATALOG,
      ),
    ).toBe(
      "Posts to hooks.slack.com when pull requests are ready for review or an agent asks something, in Web and Docs, for sessions started by people.",
    );
    expect(describeWebhook(URL, ["review.finished"], [], ["schedule"], CATALOG)).toBe(
      "Posts to hooks.slack.com when a review is posted.",
    );
    expect(
      describeWebhook(
        URL,
        ["session.stopped", "session.failed", "session.started", "review.failed"],
        ["A", "B", "C", "D"],
        ["schedule", "autopilot"],
        CATALOG,
      ),
    ).toBe(
      "Posts 4 kinds of events to hooks.slack.com, in 4 projects, for sessions started by schedules or an autopilot.",
    );
    expect(describeWebhook(URL, [], ["Web"], ["ticket", "person"], CATALOG)).toBe(
      "Posts every event to hooks.slack.com, in Web, for sessions started by tickets or people.",
    );
  });

  it("keeps fctl's name lowercase in a sentence", () => {
    expect(phraseOf("fctl pauses a schedule")).toBe("fctl pauses a schedule");
    expect(describeWebhook(URL, ["schedule.paused"], [], [], CATALOG)).toBe(
      "Posts to hooks.slack.com when fctl pauses a schedule.",
    );
  });
});

const hook = (over: Partial<G.Webhook> = {}): G.Webhook => ({
  id: "whk_1",
  workspace_id: "wsp_1",
  url: URL,
  events: [],
  projects: [],
  started_by: [],
  enabled: true,
  created_at: 0,
  updated_at: 0,
  can_manage: true,
  ...over,
});

describe("how a webhook stands", () => {
  it("goes by its name, else its host", () => {
    expect(webhookTitle(hook())).toBe("hooks.slack.com");
    expect(webhookTitle(hook({ name: "Slack" }))).toBe("Slack");
    expect(hostOf("not a url")).toBe("not a url");
  });

  it("is off, failing, taking what it gets, or new", () => {
    expect(
      hookState(hook({ enabled: false, health: { delivered: 1, failed: 0, pending: 0, dropped: 0 } })),
    ).toBe("off");
    expect(
      hookState(hook({ health: { delivered: 0, failed: 1, pending: 2, dropped: 0, failing_since: 5 } })),
    ).toBe("failing");
    expect(
      hookState(
        hook({
          health: { delivered: 3, failed: 0, pending: 0, dropped: 0, last: { at: 1, ms: 9, status: 200 } },
        }),
      ),
    ).toBe("ok");
    expect(hookState(hook())).toBe("new");
  });
});

describe("attempts", () => {
  it("says an answer's status the way people know it", () => {
    expect(statusText(200)).toBe("200 OK");
    expect(statusText(502)).toBe("502 Bad Gateway");
    expect(statusText(299)).toBe("299");
    expect(attemptWords({ at: 0, ms: 182, status: 204 })).toBe("204 No Content in 182 ms");
    expect(attemptWords({ at: 0, ms: 12, status: 500, error: "HTTP 500" })).toBe(
      "500 Internal Server Error in 12 ms",
    );
    expect(
      attemptWords({
        at: 0,
        ms: 12,
        status: 302,
        error: "HTTP 302: a redirect, which fctl doesn't follow. Give the address it redirects to.",
      }),
    ).toBe("302 Found in 12 ms: a redirect, which fctl doesn't follow. Give the address it redirects to.");
    expect(attemptWords({ at: 0, ms: 10_000, error: "No answer within 10 seconds." })).toBe(
      "No answer within 10 seconds.",
    );
  });

  it("says when a delivery goes again", () => {
    expect(againIn(10_000, 0)).toBe("in 10s");
    expect(againIn(300_000, 0)).toBe("in 5m");
    expect(againIn(2 * 3_600_000, 0)).toBe("in 2h");
    expect(againIn(150 * 60_000, 0)).toBe("in 2h 30m");
  });
});

describe("the form's starting points", () => {
  it("finds the preset a choice of events is", () => {
    expect(presetOf(["session.failed", "session.question", "session.ready_for_review"])?.key).toBe("people");
    expect(presetOf(["session.failed"])).toBeUndefined();
    for (const p of PRESETS) for (const e of p.events) expect(CATALOG.some((c) => c.type === e)).toBe(true);
  });

  it("says what's wrong with an address before the server does", () => {
    expect(urlProblem("")).toBeUndefined();
    expect(urlProblem("https://example.com/hook")).toBeUndefined();
    expect(urlProblem("example.com")).toMatch(/isn't an address/);
    expect(urlProblem("http://example.com")).toMatch(/Use https/);
    expect(urlProblem("http://127.0.0.1:9099/hook", true)).toBeUndefined();
    expect(urlProblem("https://u:p@example.com")).toMatch(/user name and password/);
  });
});

describe("verifySnippet", () => {
  it("checks the signature, the timestamp, then where a post says it comes from", () => {
    const node = verifySnippet("node", "wsp_1", "prj_1");
    expect(node).toContain('headers["webhook-signature"]');
    expect(node).toContain('event.workspace.id === "wsp_1" && event.project?.id === "prj_1"');
    const python = verifySnippet("python", "wsp_1");
    expect(python).toContain('event["workspace"]["id"] == "wsp_1"');
    expect(python).not.toContain("project");
    expect(verifySnippet("any", "wsp_1")).toContain("HMAC-SHA256");
  });
});

it("lays JSON out to read, and leaves the rest as it is", () => {
  expect(pretty('{"a":1}')).toBe('{\n  "a": 1\n}');
  expect(pretty("not json")).toBe("not json");
});

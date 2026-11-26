import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { SlackMark, SlackNoticeView, SlackPlaceLink, slackHref } from "./slack";

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

const thread = (s: Partial<G.SessionSlack> = {}): G.SessionSlack => ({
  team: "T1",
  team_name: "Acme",
  channel: "C1",
  channel_name: "web-team",
  place: "channel",
  thread_ts: "1700000000.000100",
  url: "https://acme.slack.com/archives/C1/p1700000000000100",
  ...s,
});

describe("links into Slack", () => {
  it("are Slack's own addresses, over https, and nothing else", () => {
    expect(slackHref("https://acme.slack.com/archives/C1/p1")).toBe("https://acme.slack.com/archives/C1/p1");
    expect(slackHref("https://app.slack.com/client/T1/C1")).toBe("https://app.slack.com/client/T1/C1");
    expect(slackHref("http://acme.slack.com/archives/C1/p1")).toBeUndefined();
    expect(slackHref("https://slack.com.evil.example/x")).toBeUndefined();
    expect(slackHref("https://evilslack.com/x")).toBeUndefined();
    expect(slackHref("javascript:alert(1)")).toBeUndefined();
    expect(slackHref(undefined)).toBeUndefined();
  });
});

describe("where a session started in Slack", () => {
  it("names the channel, and opens its thread there", () => {
    const c = render(<SlackPlaceLink session={{ slack: thread() }} />);
    const a = c.querySelector("a");
    expect(a?.textContent).toBe("#web-team");
    expect(a?.getAttribute("href")).toBe("https://acme.slack.com/archives/C1/p1700000000000100");
    expect(a?.getAttribute("target")).toBe("_blank");
    expect(a?.getAttribute("aria-label")).toBe(
      "Started in #web-team in Slack in Acme: open its thread in Slack",
    );
  });

  it("says a direct message, and marks a private channel", () => {
    expect(
      render(<SlackPlaceLink session={{ slack: thread({ place: "dm", channel_name: undefined }) }} />)
        .textContent,
    ).toBe("Direct message");
    const locked = render(
      <SlackPlaceLink session={{ slack: thread({ place: "private", channel_name: "secret" }) }} />,
    );
    expect(locked.querySelectorAll("svg")).toHaveLength(2);
    expect(locked.querySelector("a")?.getAttribute("aria-label")).toContain("the private channel #secret");
  });

  it("names it without a link where the address isn't Slack's", () => {
    const c = render(<SlackPlaceLink session={{ slack: thread({ url: "https://evil.example/x" }) }} />);
    expect(c.querySelector("a")).toBeNull();
    expect(c.textContent).toBe("#web-team");
  });

  it("shows nothing for a session that didn't start in Slack, and lies over the row in lists", () => {
    expect(render(<SlackPlaceLink session={{}} />).innerHTML).toBe("");
    expect(render(<SlackMark s={{ slack: thread() }} />).querySelector("a")?.className).toContain("z-[2]");
  });
});

describe("what fctl did in the thread", () => {
  it("says why it couldn't, in the control plane's words", () => {
    const c = render(
      <SlackNoticeView
        notice={{ place: "channel", channel_name: "web-team" }}
        code="slack_failed"
        message="Couldn't post the reply in #web-team: fctl's app was removed from the channel."
      />,
    );
    expect(c.textContent).toBe(
      "Couldn't post the reply in #web-team: fctl's app was removed from the channel.",
    );
    expect(c.firstElementChild?.className).toContain("text-danger");
  });

  it("links what it posted without a body straight to Slack", () => {
    const c = render(
      <SlackNoticeView
        notice={{ place: "dm", url: "https://acme.slack.com/archives/D1/p1" }}
        code="slack_asked"
        message="Asked in Slack"
      />,
    );
    expect(c.textContent).toBe("Asked in the direct messages with fctl");
    expect(c.querySelector("a")?.getAttribute("aria-label")).toBe(
      "Asked in the direct messages with fctl: open it in Slack",
    );
  });
});

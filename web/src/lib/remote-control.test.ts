import { describe, expect, it } from "vitest";
import { claudeAppLink, claudeAppUrl } from "./remote-control";

describe("claudeAppUrl", () => {
  it("links a claude.ai/code session to the Claude apps", () => {
    expect(claudeAppUrl("https://claude.ai/code/session_01AbC-d_9")).toBe("claude://code/session_01AbC-d_9");
    expect(claudeAppUrl("https://claude.ai/code/cse_01AbC")).toBe("claude://code/cse_01AbC");
  });

  it("leaves other URLs to the browser", () => {
    for (const url of [
      "https://claude.ai/code",
      "https://claude.ai/code/session_",
      "https://claude.ai/code/new?q=hello",
      "https://claude.ai/code/session_01AbC?org=1",
      "https://claude.ai/code/session_01AbC/extra",
      "https://claude.ai/code/local_01AbC",
      "https://claude.ai/chat/session_01AbC",
      "https://claude-ai.staging.ant.dev/code/session_01AbC",
      "http://claude.ai/code/session_01AbC",
      "https://claude.ai.example.com/code/session_01AbC",
      "claude://code/session_01AbC",
    ]) {
      expect(claudeAppUrl(url), url).toBeUndefined();
    }
  });
});

describe("claudeAppLink", () => {
  it("opens the app in place", () => {
    expect(claudeAppLink("https://claude.ai/code/session_01AbC")).toEqual({
      href: "claude://code/session_01AbC",
    });
  });

  it("opens anything else in a new tab", () => {
    expect(claudeAppLink("https://claude-ai.staging.ant.dev/code/session_01AbC")).toEqual({
      href: "https://claude-ai.staging.ant.dev/code/session_01AbC",
      target: "_blank",
      rel: "noreferrer",
    });
  });
});

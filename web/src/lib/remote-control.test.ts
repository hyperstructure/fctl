import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { claudeAppLink, claudeAppUrl, remoteControlUrl, remoteControlUser } from "./remote-control";

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

describe("remoteControlUrl", () => {
  const ada: G.UserRef = { id: "usr_ada", name: "Ada" };
  const bo: G.UserRef = { id: "usr_bo", name: "Bo" };
  const url = "https://claude.ai/code/session_01AbC";
  const link = { enabled: true, session_url: url };

  it("is only for the person on whose subscription it came about", () => {
    // Bo resumed Ada's session.
    const rc = { ...link, user: bo };
    expect(remoteControlUser(rc, ada)).toBe(bo);
    expect(remoteControlUrl(rc, ada, "usr_bo")).toBe(url);
    expect(remoteControlUrl(rc, ada, "usr_ada")).toBeUndefined();
  });

  it("is for the person who started the session when the state doesn't say", () => {
    expect(remoteControlUser(link, ada)).toBe(ada);
    expect(remoteControlUser(undefined, ada)).toBe(ada);
    expect(remoteControlUrl(link, ada, "usr_ada")).toBe(url);
    expect(remoteControlUrl(link, ada, "usr_bo")).toBeUndefined();
  });

  it("is for nobody before it is known who is signed in", () => {
    expect(remoteControlUrl({ ...link, user: ada }, ada, undefined)).toBeUndefined();
  });

  it("is absent without a link", () => {
    expect(
      remoteControlUrl({ enabled: false, unavailable_reason: "No login", user: ada }, ada, "usr_ada"),
    ).toBeUndefined();
    expect(remoteControlUrl(undefined, ada, "usr_ada")).toBeUndefined();
  });
});

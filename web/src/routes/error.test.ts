import { describe, expect, it } from "vitest";
import { ApiFailure } from "@/lib/api";
import { explain } from "./error";

describe("explain", () => {
  it("shows the server's status for server errors", () => {
    const e = explain(new ApiFailure(503, { message: "Service unavailable" }));
    expect(e.code).toBe("503");
    expect(e.title).toBe("The factory lost power.");
    expect(e.detail).toContain("answered 503");
    expect(e.bug).toBe(false);
  });

  it("says the server can't be reached when the request never got there", () => {
    const e = explain(new ApiFailure(0, { code: "unavailable", message: "You appear to be offline." }));
    expect(e.code).toBe("off");
    expect(e.title).toBe("Can’t reach fctl.");
  });

  it("passes on what the server said about other failures", () => {
    const e = explain(new ApiFailure(429, { message: "Too many attempts. Try again in a minute." }));
    expect(e.code).toBe("429");
    expect(e.detail).toBe("Too many attempts. Try again in a minute.");
  });

  it("hides secrets the server's message might quote", () => {
    const e = explain(new ApiFailure(400, { message: "sk-ant-api03-abcdef is an API key, not a token" }));
    expect(e.detail).toBe("[hidden] is an API key, not a token");
  });

  it("calls anything else a bug", () => {
    const e = explain(new TypeError("x is undefined"));
    expect(e.code).toBe("bug");
    expect(e.bug).toBe(true);
  });
});

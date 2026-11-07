import { afterEach, describe, expect, it } from "vitest";
import { otherView, sessionView, setSessionView } from "./session-view";

afterEach(() => {
  setSessionView("classic");
  localStorage.clear();
});

describe("the session view this browser shows", () => {
  it("is the classic one until the new one is picked, which it keeps", () => {
    expect(sessionView()).toBe("classic");
    setSessionView("new");
    expect(sessionView()).toBe("new");
    expect(localStorage.getItem("fctl:session-view")).toBe("new");
    // The classic one forgets the pick.
    setSessionView("classic");
    expect(localStorage.getItem("fctl:session-view")).toBeNull();
  });

  it("switches to the other one", () => {
    expect(otherView("classic")).toBe("new");
    expect(otherView("new")).toBe("classic");
  });
});

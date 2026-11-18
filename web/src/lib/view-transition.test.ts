import { afterEach, describe, expect, it, vi } from "vitest";
import { catchSkippedTransitions } from "./view-transition";

afterEach(() => {
  delete (document as { startViewTransition?: unknown }).startViewTransition;
});

describe("catchSkippedTransitions", () => {
  it("catches the ready promise of a transition skipped while hidden, and starts it as before", async () => {
    const skipped = new DOMException(
      "View transition was skipped because document visibility state is hidden.",
      "InvalidStateError",
    );
    const transition = {
      ready: Promise.reject(skipped),
      finished: Promise.resolve(),
      updateCallbackDone: Promise.resolve(),
      skipTransition: () => {},
    };
    const start = vi.fn(() => transition);
    Object.defineProperty(document, "startViewTransition", {
      value: start,
      configurable: true,
      writable: true,
    });
    catchSkippedTransitions();
    const update = () => {};
    expect(document.startViewTransition(update)).toBe(transition);
    expect(start).toHaveBeenCalledWith(update);
    // vitest fails a run that leaves a rejection unhandled.
    await new Promise((done) => setTimeout(done, 0));
  });

  it("leaves a browser without view transitions as it is", () => {
    catchSkippedTransitions();
    expect("startViewTransition" in document).toBe(false);
  });
});

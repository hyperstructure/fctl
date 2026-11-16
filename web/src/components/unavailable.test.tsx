import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { ApiFailure } from "@/lib/api";
import { AdminsOnly, isMissing, Unavailable } from "./unavailable";

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const notFound = new ApiFailure(404, { code: "not_found", message: "Project not found" });
const offline = new ApiFailure(0, { code: "unavailable", message: "You appear to be offline." });

describe("a page whose object didn't load", () => {
  it("tells one that isn't there for the reader from one that couldn't be reached", () => {
    expect(isMissing(notFound)).toBe(true);
    expect(isMissing(new ApiFailure(403, { code: "forbidden", message: "Forbidden" }))).toBe(true);
    expect(isMissing(offline)).toBe(false);
    expect(isMissing(new ApiFailure(500, { code: "internal", message: "Something went wrong" }))).toBe(false);
    // A refusal that asks for something says what in its message, and is not a missing page.
    expect(isMissing(new ApiFailure(403, { code: "two_factor_setup_required", message: "Set it up" }))).toBe(
      false,
    );
    expect(isMissing(new Error("boom"))).toBe(false);
  });

  it("says a missing project may not exist or be hidden, without saying which, and leads back", () => {
    const c = render(<Unavailable what="project" error={notFound} onRetry={() => {}} />);
    expect(c.querySelector("h1")?.textContent).toBe("No project here");
    expect(c.textContent).toContain("This project does not exist, or you cannot see it.");
    expect(c.textContent).toContain("Go to the Agent Floor");
    expect(c.textContent).not.toContain("Try again");
  });

  it("offers to try again when it couldn't be reached, with the reason", () => {
    const retry = vi.fn();
    const c = render(<Unavailable what="session" error={offline} onRetry={retry} />);
    expect(c.querySelector("h1")?.textContent).toBe("Couldn't open this session");
    expect(c.textContent).toContain("You appear to be offline.");
    const again = [...c.querySelectorAll("button")].find((b) => b.textContent === "Try again");
    act(() => again?.click());
    expect(retry).toHaveBeenCalledOnce();
  });

  it("tells someone who isn't an admin whom Admin is for", () => {
    const c = render(<AdminsOnly />);
    expect(c.querySelector("h1")?.textContent).toBe("Admin is for this instance's admins");
    expect(c.textContent).toContain("Go to the Agent Floor");
  });
});

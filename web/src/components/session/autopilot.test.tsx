import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { queryClient } from "@/lib/queries";
import { AutopilotAction } from "./autopilot";

// Sessions' live state never arrives: the cards show what the notice says.
vi.mock("@/lib/api", () => ({
  api: new Proxy({} as Record<string, unknown>, { get: () => () => new Promise(() => {}) }),
}));
// A link says where it goes in its href: the route, its params, and its search.
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    search,
    children,
  }: {
    to: string;
    params?: Record<string, string>;
    search?: Record<string, string>;
    children: ReactNode;
  }) => {
    const path = to.replace(/\$(\w+)/g, (_, k: string) => params?.[k] ?? "");
    const query = search ? `?${new URLSearchParams(search)}` : "";
    return <a href={`${path}${query}`}>{children}</a>;
  },
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={queryClient}>{node}</QueryClientProvider>));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const notice = (message: string, autopilot: G.AutopilotNotice): G.Notice =>
  ({ level: "info", code: "autopilot_action", message, autopilot }) as G.Notice;

const links = (page: HTMLElement) =>
  [...page.querySelectorAll("a")].map((a) => [a.textContent, a.getAttribute("href")]);

describe("what an autopilot did, in its own session", () => {
  it("shows a schedule it ran with the session it started, and links to both", () => {
    const page = render(
      <AutopilotAction
        notice={notice("Ran the schedule “Nightly deps” now shop#42: Update the dependencies", {
          kind: "schedule",
          session: { id: "ses_42", number: 42, title: "Update the dependencies" },
          project_slug: "shop",
          schedule: { id: "sch_1", name: "Nightly deps" },
        })}
      />,
    );
    expect(page.textContent).toContain("Ran the schedule “Nightly deps” now");
    expect(links(page)).toEqual([
      ["“Nightly deps”", "/p/shop?tab=schedules&schedule=sch_1"],
      [expect.stringContaining("Update the dependencies"), "/s/ses_42"],
    ]);
    expect(page.textContent).toContain("shop#42");
  });

  it("names a deleted schedule's run without a link to it", () => {
    const page = render(
      <AutopilotAction
        notice={notice("Ran the schedule “Nightly deps” now shop#42: Update the dependencies", {
          kind: "schedule",
          session: { id: "ses_42", number: 42, title: "Update the dependencies" },
          project_slug: "shop",
          schedule: { id: "sch_1" },
        })}
      />,
    );
    expect(page.textContent).toContain("Ran the schedule “Nightly deps” now");
    expect(links(page).map(([, href]) => href)).toEqual(["/s/ses_42"]);
  });

  it("keeps a run from before fctl knew its session as a line", () => {
    const page = render(
      <AutopilotAction notice={notice("Ran a schedule now", { kind: "schedule", project_slug: "shop" })} />,
    );
    expect(page.textContent).toBe("Ran a schedule now");
    expect(page.querySelector("a")).toBeNull();
  });
});

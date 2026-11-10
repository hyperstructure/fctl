import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { PageTabs, TabCount } from "./page-tabs";

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    search,
    activeOptions: _,
    ...props
  }: {
    children: ReactNode;
    search?: { tab?: string };
    activeOptions?: unknown;
  }) => (
    <a href={search?.tab ? `/admin?tab=${search.tab}` : "/admin"} {...props}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(open: "people" | "access") {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <PageTabs
        label="Admin"
        open={open}
        link={(t) => ({ to: "/admin", search: { tab: t === "people" ? undefined : t } })}
        tabs={[
          { id: "people", label: "People", icon: (c) => <svg data-icon="people" className={c} /> },
          {
            id: "access",
            label: "Access",
            icon: (c) => <svg data-icon="access" className={c} />,
            badge: <TabCount n={2} label="2 waiting" />,
          },
        ]}
      />,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

describe("PageTabs", () => {
  it("links each tab, the default one without a tab of its own, each with its icon", () => {
    const page = render("people");
    expect(page.querySelector("nav")?.getAttribute("aria-label")).toBe("Admin");
    const links = [...page.querySelectorAll("a")];
    expect(links.map((a) => a.getAttribute("href"))).toEqual(["/admin", "/admin?tab=access"]);
    expect(links.map((a) => a.querySelector("svg")?.getAttribute("data-icon"))).toEqual(["people", "access"]);
    expect(links[1]?.querySelector('[role="img"]')?.getAttribute("aria-label")).toBe("2 waiting");
  });

  it("says which tab is open, and draws its icon in the accent", () => {
    const page = render("access");
    const [people, access] = [...page.querySelectorAll("a")];
    expect(people?.getAttribute("aria-current")).toBeNull();
    expect(access?.getAttribute("aria-current")).toBe("page");
    expect(access?.querySelector("svg")?.getAttribute("class")).toContain("text-accent");
    expect(people?.querySelector("svg")?.getAttribute("class")).toContain("text-ink-4");
  });
});

import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { edgeFade, overflowing, PageTabs, TabCount } from "./page-tabs";

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

describe("a strip of tabs too wide for the screen", () => {
  const strip = (scrollLeft: number) => ({ scrollLeft, scrollWidth: 700, clientWidth: 390 });

  it("knows which of its edges have more tabs past them", () => {
    expect(overflowing(strip(0))).toEqual({ left: false, right: true });
    expect(overflowing(strip(150))).toEqual({ left: true, right: true });
    expect(overflowing(strip(310))).toEqual({ left: true, right: false });
    // A zoomed page scrolls by fractions of a pixel.
    expect(overflowing(strip(309.5))).toEqual({ left: true, right: false });
    expect(overflowing({ scrollLeft: 0, scrollWidth: 390, clientWidth: 390 })).toEqual({
      left: false,
      right: false,
    });
  });

  it("fades only the edges with more past them", () => {
    expect(edgeFade({ left: false, right: false })).toBeUndefined();
    const right = edgeFade({ left: false, right: true })?.maskImage;
    expect(right).toBe("linear-gradient(to right, #000 0, #000 calc(100% - 40px), transparent)");
    expect(edgeFade({ left: true, right: false })?.maskImage).toBe(
      "linear-gradient(to right, transparent, #000 40px, #000 100%)",
    );
  });
});

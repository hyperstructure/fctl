import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { HeaderMenuButton, MenuButton, ShellContext, useHeaderButtons } from "./menu-button";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

/** The shell as AppShell lays it out: the top bar, then the page. */
function Shell({ openSidebar, children }: { openSidebar: () => void; children: ReactNode }) {
  const [headerButton, shell] = useHeaderButtons(openSidebar);
  return (
    <>
      {!headerButton && (
        <nav aria-label="Top bar">
          <MenuButton onClick={openSidebar} />
        </nav>
      )}
      <main>
        <ShellContext value={shell}>{children}</ShellContext>
      </main>
    </>
  );
}

/** The shell around `page`, which `show` replaces as navigating does. */
function render(page: ReactNode) {
  const openSidebar = vi.fn();
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  const show = (node: ReactNode) => act(() => root.render(<Shell openSidebar={openSidebar}>{node}</Shell>));
  show(page);
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  const topBar = () => container.querySelector("nav");
  const buttons = () => [...container.querySelectorAll<HTMLButtonElement>('button[aria-label="Open menu"]')];
  return { openSidebar, show, topBar, buttons };
}

/** A page whose header sticks to the top, as the session's does. */
const SessionPage = () => (
  <header>
    <HeaderMenuButton />
    <h1>Fix the flaky login test</h1>
  </header>
);

describe("the menu button", () => {
  it("is in the top bar on a page without one in its header", () => {
    const { openSidebar, topBar, buttons } = render(<h1>Home</h1>);
    expect(topBar()).not.toBeNull();
    expect(buttons()).toHaveLength(1);
    act(() => buttons()[0]?.click());
    expect(openSidebar).toHaveBeenCalledOnce();
  });

  it("is in the header of a page that has it there, and the top bar is gone", () => {
    const { openSidebar, topBar, buttons } = render(<SessionPage />);
    expect(topBar()).toBeNull();
    expect(buttons()).toHaveLength(1);
    expect(buttons()[0]?.closest("header")).not.toBeNull();
    act(() => buttons()[0]?.click());
    expect(openSidebar).toHaveBeenCalledOnce();
  });

  it("moves between the top bar and the header as pages change", () => {
    const { show, topBar } = render(<h1>Home</h1>);
    show(<SessionPage />);
    expect(topBar()).toBeNull();
    show(<h1>Home</h1>);
    expect(topBar()).not.toBeNull();
  });

  it("brings the top bar back only once no header has it", () => {
    const { show, topBar } = render([<SessionPage key="a" />, <SessionPage key="b" />]);
    expect(topBar()).toBeNull();
    show([<SessionPage key="a" />]);
    expect(topBar()).toBeNull();
    show([]);
    expect(topBar()).not.toBeNull();
  });
});

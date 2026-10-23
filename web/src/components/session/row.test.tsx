import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import { TicketMark } from "./row";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: React.ReactNode) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() => root.render(node));
  roots.push(() => act(() => root.unmount()));
  return container;
}

describe("TicketMark", () => {
  it("links a session's ticket in its tracker, by its key", () => {
    const c = render(
      <TicketMark
        s={{
          ticket: {
            tracker: "linear",
            id: "a1b2",
            key: "ENG-3",
            title: "Retry failed uploads",
            url: "https://linear.app/acme/issue/ENG-3",
          },
        }}
      />,
    );
    const a = c.querySelector("a");
    expect(a?.getAttribute("href")).toBe("https://linear.app/acme/issue/ENG-3");
    expect(a?.textContent).toBe("ENG-3");
    expect(a?.getAttribute("title")).toBe("ENG-3: Retry failed uploads, on Linear");
    expect(a?.getAttribute("target")).toBe("_blank");
  });

  it("shows nothing for a session no ticket started", () => {
    expect(render(<TicketMark s={{}} />).innerHTML).toBe("");
  });
});

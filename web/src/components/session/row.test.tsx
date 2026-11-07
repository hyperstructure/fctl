import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { stamp } from "@/lib/format";
import { SessionRow, TicketMark } from "./row";

vi.mock("@tanstack/react-router", () => ({
  Link: ({
    children,
    to,
    params,
    ...rest
  }: {
    children: ReactNode;
    to: string;
    params: Record<string, string>;
    className?: string;
    "data-cover"?: boolean;
  }) => (
    <a href={to.replace(/\$(\w+)/g, (_, k: string) => params[k] ?? "")} {...rest}>
      {children}
    </a>
  ),
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

function render(node: ReactNode) {
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

const ada = { id: "usr_ada", name: "Ada Lovelace" } as G.UserRef;

const session = (s: Partial<G.Session> = {}): G.Session =>
  ({
    id: "ses_1",
    project_id: "prj_web",
    project_slug: "web",
    number: 12,
    title: "Retry failed uploads",
    status: "running",
    created_by: ada,
    created_at: Date.now() - 3_600_000,
    updated_at: Date.now(),
    last_activity_at: Date.now() - 4 * 60_000,
    archived: false,
    stats: {
      turns: 1,
      input_tokens: 0,
      output_tokens: 0,
      cache_read_tokens: 0,
      tool_calls: 0,
      files_changed: 0,
      additions: 0,
      deletions: 0,
      busy_ms: 0,
      compactions: 0,
    },
    ...s,
  }) as G.Session;

describe("SessionRow", () => {
  it("opens the session from its title's line, with when it last moved and who started it at its end", () => {
    const s = session();
    const c = render(<SessionRow s={s} />);
    const link = c.querySelector<HTMLAnchorElement>("a[data-cover]");
    expect(link?.getAttribute("href")).toBe("/s/ses_1");
    // In the link, so that a click there opens the session too, and hidden from screen readers,
    // which read the sentence after it.
    const time = link?.querySelector("time");
    expect(time?.textContent).toBe("4m");
    expect(time?.getAttribute("title")).toBe(`Last active ${stamp(s.last_activity_at ?? 0)}`);
    expect(time?.closest("[aria-hidden=true]")).not.toBeNull();
    expect(link?.querySelector("[title='Started by Ada Lovelace']")).not.toBeNull();
    expect(c.querySelector(".sr-only")?.textContent).toBe(
      "Working. Started by Ada Lovelace, last active 4 minutes ago.",
    );
  });

  it("has no arrow: the row itself opens the session", () => {
    const c = render(<SessionRow s={session()} />);
    expect(c.querySelector("svg.lucide-arrow-right")).toBeNull();
  });

  it("names the session by its project and number, or in its project's list by its number and branch", () => {
    const s = session({ branch: "fctl/12-retry-failed-uploads" });
    expect(render(<SessionRow s={s} />).textContent).toContain("web#12");
    expect(render(<SessionRow s={s} />).textContent).not.toContain("fctl/12-retry-failed-uploads");
    const inProject = render(<SessionRow s={s} inProject />).textContent;
    expect(inProject).not.toContain("web#12");
    expect(inProject).toContain("#12");
    expect(inProject).toContain("fctl/12-retry-failed-uploads");
  });

  it("puts one separator before its pull requests, and none without", () => {
    const pr = {
      number: 120,
      url: "https://github.com/acme/web/pull/120",
      state: "open",
      repo: { owner: "acme", name: "web" },
    };
    const c = render(<SessionRow s={session({ pull_requests: [pr] })} />);
    const a = c.querySelector<HTMLAnchorElement>("a[href='https://github.com/acme/web/pull/120']");
    expect(a?.textContent).toBe("web#120");
    expect(a?.parentElement?.previousElementSibling?.textContent).toBe("·");
    expect(render(<SessionRow s={session()} />).textContent).not.toContain("·");
  });
});

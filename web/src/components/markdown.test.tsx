import { act, type ReactNode } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import { Gallery } from "@/components/ui/gallery";
import { ImageSizes, Markdown, Pictures } from "./markdown";

vi.mock("@/components/ui", async () => ({
  ...(await vi.importActual<typeof import("@/components/ui/gallery")>("@/components/ui/gallery")),
  CopyButton: () => null,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

/**
 * `sizes`: the images whose size the page knows, as a session's published
 * screenshots. `around`: what the page has around the answer.
 */
function render(
  text: string,
  sizes = new Map<string, { width?: number; height?: number }>(),
  around = (answer: ReactNode) => answer,
) {
  const container = document.createElement("div");
  const root = createRoot(container);
  act(() =>
    root.render(
      around(
        <ImageSizes value={sizes}>
          <Markdown text={text} />
        </ImageSizes>,
      ),
    ),
  );
  roots.push(() => act(() => root.unmount()));
  return container;
}

const src = "https://app.fctl.test/api/v1/published/abc/after.png";

describe("images in the agent's answers", () => {
  it("show at full size over the page when clicked", () => {
    const c = render(`The new header:\n\n![after.png](${src})`);
    const button = c.querySelector<HTMLButtonElement>('button[aria-label="View after.png"]');
    expect(button?.querySelector("img")?.getAttribute("src")).toBe(src);
    expect(document.querySelector("dialog")).toBeNull();
    act(() => button?.click());
    expect(document.querySelector('dialog[aria-label="after.png"] img')?.getAttribute("src")).toBe(src);
  });

  it("step from one to the next in the gallery around them", () => {
    const before = src.replace("after", "before");
    const c = render(`![before.png](${before}) ![after.png](${src})`, undefined, (answer) => (
      <Gallery>{answer}</Gallery>
    ));
    act(() => c.querySelector<HTMLButtonElement>('button[aria-label="View before.png"]')?.click());
    const dialog = document.querySelector("dialog") as HTMLDialogElement;
    expect(dialog.querySelector("p[aria-live]")?.textContent).toBe("before.png: 1 of 2");
    act(() => {
      dialog.dispatchEvent(new KeyboardEvent("keydown", { key: "ArrowRight", bubbles: true }));
    });
    expect(dialog.getAttribute("aria-label")).toBe("after.png");
  });

  it("follow their link when they are one", () => {
    const c = render(`[![after.png](${src})](https://github.com/acme/web/pull/7)`);
    expect(c.querySelector("button")).toBeNull();
    expect(c.querySelector("a img")?.getAttribute("src")).toBe(src);
  });

  it("take the room of a published screenshot before it loads, and load as the reader nears it", () => {
    const sizes = new Map([[src, { width: 1440, height: 900 }]]);
    for (const text of [
      `![after.png](${src})`,
      `[![after.png](${src})](https://github.com/acme/web/pull/7)`,
    ]) {
      const img = render(text, sizes).querySelector("img");
      expect([img?.getAttribute("width"), img?.getAttribute("height")]).toEqual(["1440", "900"]);
      expect(img?.getAttribute("loading")).toBe("lazy");
    }
  });

  it("load with the page when their size isn't known, so that they can't push it as the reader scrolls up", () => {
    const other = render("![a diagram](https://app.fctl.test/diagram.png)").querySelector("img");
    expect(other?.getAttribute("loading")).toBeNull();
    expect(other?.hasAttribute("width")).toBe(false);
    // Without both sides, the browser can't keep the room.
    const half = render(`![after.png](${src})`, new Map([[src, { width: 1440 }]])).querySelector("img");
    expect(half?.getAttribute("loading")).toBeNull();
    expect(half?.hasAttribute("width")).toBe(false);
  });

  it("show only their text from anywhere but https", () => {
    const c = render("![after.png](http://example.com/after.png)");
    expect(c.querySelector("img")).toBeNull();
    expect(c.textContent).toBe("after.png");
  });

  it("load from the app's own address only as published screenshots", () => {
    // jsdom's page is on http, where published links of the app's own would be http too.
    const app = "https://app.fctl.test";
    vi.stubGlobal("location", new URL(`${app}/s/ses_1`));
    try {
      const export_ = render(`![x](${app}/api/v1/me/export)`);
      expect(export_.querySelector("img")).toBeNull();
      expect(export_.textContent).toBe("x");
      const published = `${app}/api/v1/published/${"A".repeat(43)}/after.png`;
      expect(render(`![after.png](${published})`).querySelector("img")?.getAttribute("src")).toBe(published);
    } finally {
      vi.unstubAllGlobals();
    }
  });
});

describe("images in text from elsewhere", () => {
  const elsewhere = (text: string) =>
    render(text, undefined, (answer) => <Pictures value={false}>{answer}</Pictures>);

  it("are links to them, like any other link", () => {
    const a = elsewhere("![diagram](https://example.com/diagram.png)").querySelector("a");
    expect(a?.getAttribute("href")).toBe("https://example.com/diagram.png");
    expect(a?.textContent).toBe("diagram");
  });

  it("never link to the app's sign-in pages or API", () => {
    const origin = globalThis.location.origin;
    for (const page of [`${origin}/device?code=ABCD-EFGH`, `${origin}/api/v1/me/export`]) {
      const c = elsewhere(`![screenshot](${page})`);
      expect(c.querySelector("a")).toBeNull();
      expect(c.textContent).toBe("screenshot");
    }
  });
});

describe("tables", () => {
  const table = [
    "| File | Link |",
    "| --- | --- |",
    "| `crates/fctl/src/main.rs` | https://github.com/hyperstructure/fctl/pull/1 |",
  ].join("\n");

  it("break paths and URLs in their cells after a slash, but within //", () => {
    const [code, link] = [...render(table).querySelectorAll("td")];
    expect(code?.innerHTML).toBe("<code>crates/<wbr>fctl/<wbr>src/<wbr>main.rs</code>");
    expect(link?.querySelectorAll("wbr")).toHaveLength(5);
    expect(link?.innerHTML).toContain("https://<wbr>github.com/<wbr>");
    expect(link?.textContent).toBe("https://github.com/hyperstructure/fctl/pull/1");
    expect(link?.querySelector("a")?.getAttribute("href")).toBe(
      "https://github.com/hyperstructure/fctl/pull/1",
    );
  });

  it("leave text outside them as it is", () => {
    const c = render("See `crates/fctl/src/main.rs` and docs/README.md.");
    expect(c.querySelector("wbr")).toBeNull();
  });
});

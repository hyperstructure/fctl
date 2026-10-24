import { QueryClientProvider } from "@tanstack/react-query";
import { act, type ReactNode, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { queryClient } from "@/lib/queries";
import { ReviewBots } from "./review-bots";

// GitHub, as the control plane answers for it: Renovate and an app of the project's own opened
// pull requests lately, Dependabot too; it knows acme-translations by name, and nothing else.
vi.mock("@/lib/api", () => {
  class ApiFailure extends Error {
    code: string;
    constructor(_status: number, body: { code: string; message: string }) {
      super(body.message);
      this.code = body.code;
    }
  }
  const seen = (login: string, pull_requests: number) => ({
    login,
    url: `https://github.com/apps/${login.replace("[bot]", "")}`,
    pull_requests,
    latest: {
      number: 318,
      url: "https://github.com/acme/web/pull/318",
      title: "chore: update",
      repo: { owner: "acme", name: "web" },
    },
    latest_at: Date.now() - 3_600_000,
  });
  return {
    ApiFailure,
    api: {
      reviewBots: async () => ({
        bots: [seen("renovate[bot]", 14), seen("acme-releaser[bot]", 2), seen("dependabot[bot]", 3)],
      }),
      reviewBot: async (_project: string, login: string) => {
        if (login === "acme-translations[bot]")
          return {
            login,
            name: "Acme Translations",
            owner: "acme",
            url: "https://github.com/apps/acme-translations",
            pull_requests: 0,
          };
        throw new ApiFailure(404, { code: "not_found", message: "GitHub shows no app by that name." });
      },
    },
  };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  queryClient.clear();
});

const project = { id: "prj_1", slug: "web", name: "Web" } as G.Project;

/** The field in a form, as the review agents' settings have it, which keeps what it changes. */
function Form({
  saved,
  onChange,
  onSubmit,
}: {
  saved: string[];
  onChange?: (b: string[]) => void;
  onSubmit?: () => void;
}) {
  const [value, setValue] = useState(saved);
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit?.();
      }}
    >
      <ReviewBots
        project={project}
        value={value}
        saved={saved}
        onChange={(b) => {
          setValue(b);
          onChange?.(b);
        }}
      />
    </form>
  );
}

async function render(node: ReactNode) {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={queryClient}>{node}</QueryClientProvider>));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  // The list of apps GitHub saw answers.
  await settle();
  await settle();
  return container;
}

/** Lets queries answer, and the field's pause before it asks GitHub pass. */
const settle = (ms = 0) => act(() => new Promise<void>((r) => setTimeout(r, ms)));

const listed = (c: HTMLElement) =>
  [...c.querySelectorAll('ul[aria-label="Apps that start reviews"] > li')].map((li) => li.textContent ?? "");
const offered = (c: HTMLElement) =>
  [...c.querySelectorAll('[role="option"]')].map((o) => o.textContent ?? "");
const field = (c: HTMLElement) =>
  c.querySelector<HTMLInputElement>('input[role="combobox"]') as HTMLInputElement;

function type(c: HTMLElement, text: string) {
  const input = field(c);
  act(() => {
    input.focus();
    // React listens for the native setter's input event.
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

const press = (c: HTMLElement, key: string) =>
  act(() => {
    field(c).dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  });

describe("the apps that start reviews", () => {
  it("lists the project's by name, and offers those that opened pull requests lately", async () => {
    const c = await render(<Form saved={["dependabot[bot]"]} />);
    expect(listed(c)).toHaveLength(1);
    expect(listed(c)[0]).toContain("Dependabot");
    expect(listed(c)[0]).toContain("dependabot[bot]");
    expect(listed(c)[0]).toContain("3 pull requests here lately");
    const options = offered(c);
    expect(options).toHaveLength(2);
    expect(options[0]).toContain("Renovate");
    expect(options[0]).toContain("14 pull requests here lately");
    expect(c.querySelector('[role="option"] [title]')?.getAttribute("title")).toBe(
      "The newest: chore: update (web#318)",
    );
    expect(options[1]).toContain("acme-releaser[bot]");
    expect(c.textContent).toContain("Opened pull requests here lately");
  });

  it("adds one with a click, marked new until the form saves", async () => {
    const changed = vi.fn();
    const c = await render(<Form saved={["dependabot[bot]"]} onChange={changed} />);
    act(() => (c.querySelector('[role="option"]') as HTMLElement).click());
    expect(changed).toHaveBeenLastCalledWith(["dependabot[bot]", "renovate[bot]"]);
    expect(listed(c)[1]).toContain("Renovate");
    expect(listed(c)[1]).toContain("New");
    expect(offered(c).some((o) => o.includes("Renovate"))).toBe(false);
  });

  it("finds them as people type, and adds the one picked with the keys without saving the form", async () => {
    const submit = vi.fn();
    const changed = vi.fn();
    const c = await render(<Form saved={[]} onChange={changed} onSubmit={submit} />);
    expect(listed(c)[0]).toContain("None");
    type(c, "GitHub Act");
    expect(offered(c)).toHaveLength(1);
    expect(offered(c)[0]).toContain("GitHub Actions");
    type(c, "re");
    // Those with a word that starts with it first: those that opened pull requests here before
    // those many repositories use.
    const found = offered(c);
    expect(found).toHaveLength(3);
    expect(found[0]).toContain("Renovate");
    expect(found[1]).toContain("acme-releaser[bot]");
    expect(found[2]).toContain("pre-commit.ci");
    press(c, "ArrowDown");
    press(c, "Enter");
    expect(submit).not.toHaveBeenCalled();
    expect(changed).toHaveBeenLastCalledWith(["acme-releaser[bot]"]);
    expect(field(c).value).toBe("");
  });

  it("takes one off until the form saves, and puts it back", async () => {
    const changed = vi.fn();
    const submit = vi.fn();
    const c = await render(
      <Form saved={["dependabot[bot]", "renovate[bot]"]} onChange={changed} onSubmit={submit} />,
    );
    act(() => (c.querySelector('button[aria-label="Take Dependabot off the list"]') as HTMLElement).click());
    expect(changed).toHaveBeenLastCalledWith(["renovate[bot]"]);
    expect(listed(c)[0]).toContain("Taken off the list when you save");
    // It isn't offered again beside its Undo.
    expect(offered(c).some((o) => o.includes("Dependabot"))).toBe(false);
    const undo = [...c.querySelectorAll("button")].find((b) => b.textContent === "Undo");
    act(() => undo?.click());
    expect(changed).toHaveBeenLastCalledWith(["renovate[bot]", "dependabot[bot]"]);
    expect(listed(c)[0]).not.toContain("Taken off");
    // Neither saves the form: Save does.
    expect(submit).not.toHaveBeenCalled();
  });

  it("asks GitHub about an app nobody here knows", async () => {
    const changed = vi.fn();
    const c = await render(<Form saved={[]} onChange={changed} />);
    type(c, "https://github.com/apps/acme-translations");
    expect(c.textContent).toContain("Asking GitHub about acme-translations[bot]…");
    await settle(350);
    await settle();
    expect(offered(c)).toHaveLength(1);
    expect(offered(c)[0]).toContain("Acme Translations");
    expect(offered(c)[0]).toContain("By acme");
    press(c, "Enter");
    expect(changed).toHaveBeenLastCalledWith(["acme-translations[bot]"]);
  });

  it("offers an app GitHub doesn't show, as a private one, and guesses at a slip", async () => {
    const c = await render(<Form saved={[]} />);
    type(c, "acme-release-bot");
    await settle(350);
    await settle();
    expect(offered(c)).toHaveLength(1);
    expect(offered(c)[0]).toContain("acme-release-bot[bot]");
    expect(offered(c)[0]).toContain("GitHub shows no public app by this name");
    type(c, "renovat[bot]");
    await settle(350);
    await settle();
    // "renovat" starts Renovate's name: it is offered, and GitHub's answer stays out of the way.
    expect(offered(c)).toHaveLength(1);
    expect(offered(c)[0]).toContain("Renovate");
    type(c, "dependabto");
    await settle(350);
    await settle();
    expect(offered(c)[0]).toContain("Did you mean this one?");
    expect(offered(c)[0]).toContain("Dependabot");
    expect(offered(c)[1]).toContain("dependabto[bot]");
  });

  it("says how to name an app when what's typed can't be one", async () => {
    const c = await render(<Form saved={[]} />);
    type(c, "acme/web");
    expect(offered(c)).toHaveLength(0);
    expect(c.textContent).toContain("No app here goes by that.");
    expect(field(c).getAttribute("aria-invalid")).toBe("true");
  });

  it("says when one is on the list already", async () => {
    const c = await render(<Form saved={["dependabot[bot]"]} />);
    type(c, "dependabot");
    expect(offered(c)[0]).toContain("On the list already");
    expect(c.querySelector('[role="option"]')?.getAttribute("aria-disabled")).toBe("true");
  });
});

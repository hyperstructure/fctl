import { act, useState } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { repoChoices } from "@/lib/repos";
import { RepoPicker } from "./repo-picker";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const repo = (name: string, more: Partial<G.GithubRepo> = {}): G.GithubRepo => ({
  owner: "hyperstructure",
  name,
  full_name: `hyperstructure/${name}`,
  private: false,
  default_branch: "main",
  ...more,
});
// The workspace's repositories, newest push first, and the project's: fctl, and design added.
const available = [repo("fctl"), repo("homebrew-tap"), repo("design", { private: true }), repo("fctl-site")];
const project: G.ProjectRepo[] = [
  { owner: "hyperstructure", name: "fctl", access: "write", primary: true },
  { owner: "hyperstructure", name: "design", access: "write", primary: false },
];

/** The field as the Repositories tab has it, in a form whose submit adds what's chosen. */
function Tab({
  listed = project,
  repos = available,
  accounts = ["hyperstructure"],
  onChange,
  onSubmit,
}: {
  listed?: G.ProjectRepo[];
  repos?: G.GithubRepo[];
  accounts?: string[];
  onChange?: (c: string[]) => void;
  onSubmit?: (c: string[]) => void;
}) {
  const [chosen, setChosen] = useState<string[]>([]);
  const [query, setQuery] = useState("");
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        onSubmit?.(chosen);
      }}
    >
      <RepoPicker
        id="repos"
        choices={repoChoices({ listed, available: repos, accounts, chosen, query })}
        chosen={chosen}
        onChange={(c) => {
          setChosen(c);
          onChange?.(c);
        }}
        query={query}
        onQuery={setQuery}
        loaded
        placeholder="Choose from the list, or type hyperstructure/name"
      />
    </form>
  );
}

function render(node: React.ReactNode) {
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

const field = (c: HTMLElement) =>
  c.querySelector<HTMLInputElement>('input[role="combobox"]') as HTMLInputElement;
const options = (c: HTMLElement) => [...c.querySelectorAll<HTMLElement>('[role="option"]')];
const offered = (c: HTMLElement) => options(c).map((o) => o.textContent ?? "");
const option = (c: HTMLElement, text: string) =>
  options(c).find((o) => o.textContent?.startsWith(text)) as HTMLElement;
const chips = (c: HTMLElement) =>
  [...c.querySelectorAll<HTMLButtonElement>('button[aria-label^="Remove "]')].map((b) =>
    (b.getAttribute("aria-label") ?? "").replace(/^Remove (.*) from the choice$/, "$1"),
  );
const expanded = (c: HTMLElement) => field(c).getAttribute("aria-expanded") === "true";

function type(c: HTMLElement, text: string) {
  const input = field(c);
  act(() => {
    input.focus();
    // React listens for the native setter's input event.
    Object.getOwnPropertyDescriptor(HTMLInputElement.prototype, "value")?.set?.call(input, text);
    input.dispatchEvent(new Event("input", { bubbles: true }));
  });
}

/** Presses a key in the field; whether nothing cancelled it, as a form's Enter needs. */
function press(c: HTMLElement, key: string): boolean {
  let free = true;
  act(() => {
    free = field(c).dispatchEvent(new KeyboardEvent("keydown", { key, bubbles: true, cancelable: true }));
  });
  return free;
}

/** A click on the field, away from its text: it opens and closes the list. */
const open = (c: HTMLElement) => act(() => field(c).parentElement?.click());

describe("the field that adds repositories to a project", () => {
  it("lists every repository of the account, then the others, then those on the project", () => {
    const c = render(<Tab />);
    expect(options(c)).toHaveLength(0);
    open(c);
    expect(expanded(c)).toBe(true);
    const list = offered(c);
    expect(list).toHaveLength(5);
    expect(list[0]).toContain("hyperstructure/*");
    expect(list[0]).toContain("Every repository of hyperstructure the GitHub App can reach, read only");
    expect(list.slice(1, 3)).toEqual(["hyperstructure/homebrew-tap", "hyperstructure/fctl-site"]);
    expect(list[3]).toBe("hyperstructure/fctlPrimary");
    expect(list[4]).toBe("hyperstructure/designAdded");
    // Those on the project are ticked, and can't be chosen.
    expect(options(c).map((o) => o.getAttribute("aria-selected"))).toEqual([
      "false",
      "false",
      "false",
      "true",
      "true",
    ]);
    expect(options(c).map((o) => o.getAttribute("aria-disabled"))).toEqual([
      null,
      null,
      null,
      "true",
      "true",
    ]);
    expect(c.querySelector('[role="listbox"]')?.getAttribute("aria-multiselectable")).toBe("true");
  });

  it("ticks several with clicks, and shows them in the field", () => {
    const changed = vi.fn();
    const c = render(<Tab onChange={changed} />);
    open(c);
    act(() => option(c, "hyperstructure/fctl-site").click());
    act(() => option(c, "hyperstructure/*").click());
    expect(changed).toHaveBeenLastCalledWith(["hyperstructure/fctl-site", "hyperstructure/*"]);
    expect(chips(c)).toEqual(["hyperstructure/fctl-site", "hyperstructure/*"]);
    // The list stays open for more.
    expect(expanded(c)).toBe(true);
    expect(option(c, "hyperstructure/fctl-site").getAttribute("aria-selected")).toBe("true");
    // A click again takes one back.
    act(() => option(c, "hyperstructure/fctl-site").click());
    expect(chips(c)).toEqual(["hyperstructure/*"]);
    // What's on the project already doesn't change.
    act(() => option(c, "hyperstructure/design").click());
    expect(changed).toHaveBeenLastCalledWith(["hyperstructure/*"]);
  });

  it("finds one as people type, chooses it with Enter, and leaves the next Enter to the form", () => {
    const c = render(<Tab />);
    type(c, "tap");
    expect(expanded(c)).toBe(true);
    expect(offered(c)).toEqual(["hyperstructure/homebrew-tap"]);
    expect(field(c).getAttribute("aria-activedescendant")).toBe(options(c)[0]?.id);
    expect(press(c, "Enter")).toBe(false);
    expect(chips(c)).toEqual(["hyperstructure/homebrew-tap"]);
    expect(field(c).value).toBe("");
    expect(field(c).getAttribute("aria-activedescendant")).toBeNull();
    // Nothing is highlighted now, so the form adds what's chosen.
    expect(press(c, "Enter")).toBe(true);
    expect(expanded(c)).toBe(false);
  });

  it("chooses one typed whole that isn't on the list", () => {
    const c = render(<Tab />);
    type(c, "https://github.com/hyperstructure/sandbox");
    expect(offered(c)).toEqual([
      "hyperstructure/sandboxNot on the list: fctl checks it with GitHub as it adds it",
    ]);
    press(c, "Enter");
    expect(chips(c)).toEqual(["hyperstructure/sandbox"]);
    // It stays on the list, ticked, at its top.
    expect(offered(c)[0]).toContain("hyperstructure/sandbox");
    expect(options(c)[0]?.getAttribute("aria-selected")).toBe("true");
  });

  it("won't choose another account's", () => {
    const c = render(<Tab />);
    type(c, "romanesco/web");
    expect(offered(c)).toEqual(["romanesco/webAnother account"]);
    expect(options(c)[0]?.getAttribute("aria-disabled")).toBe("true");
    expect(press(c, "Enter")).toBe(true);
    act(() => options(c)[0]?.click());
    expect(chips(c)).toEqual([]);
  });

  it("moves through the choices with the arrow keys, past those it can't choose", () => {
    const c = render(<Tab />);
    act(() => field(c).focus());
    press(c, "ArrowDown");
    expect(expanded(c)).toBe(true);
    const active = () => document.getElementById(field(c).getAttribute("aria-activedescendant") ?? "");
    expect(active()?.textContent).toContain("hyperstructure/*");
    press(c, "ArrowDown");
    press(c, "ArrowDown");
    expect(active()?.textContent).toBe("hyperstructure/fctl-site");
    // The rest are on the project already.
    press(c, "ArrowDown");
    expect(active()?.textContent).toBe("hyperstructure/fctl-site");
    press(c, "Enter");
    press(c, "ArrowUp");
    press(c, "Enter");
    expect(chips(c)).toEqual(["hyperstructure/fctl-site", "hyperstructure/homebrew-tap"]);
    press(c, "Escape");
    expect(expanded(c)).toBe(false);
  });

  it("takes back the last choice with Backspace, and any with its button", () => {
    const changed = vi.fn();
    const c = render(<Tab onChange={changed} />);
    open(c);
    for (const name of ["hyperstructure/homebrew-tap", "hyperstructure/fctl-site", "hyperstructure/*"])
      act(() => option(c, name).click());
    act(() =>
      c
        .querySelector<HTMLButtonElement>(
          'button[aria-label="Remove hyperstructure/homebrew-tap from the choice"]',
        )
        ?.click(),
    );
    expect(chips(c)).toEqual(["hyperstructure/fctl-site", "hyperstructure/*"]);
    expect(press(c, "Backspace")).toBe(false);
    expect(chips(c)).toEqual(["hyperstructure/fctl-site"]);
    // Not while there's text to delete.
    type(c, "x");
    expect(press(c, "Backspace")).toBe(true);
    expect(chips(c)).toEqual(["hyperstructure/fctl-site"]);
    expect(c.textContent).toContain("hyperstructure/* no longer chosen");
  });

  it("closes on a press or a tap elsewhere, but not on a swipe that scrolls the page", () => {
    const c = render(<Tab />);
    const touch = (type: string, x: number, y: number) =>
      act(() => {
        document.body.dispatchEvent(
          new PointerEvent(type, {
            bubbles: true,
            pointerType: "touch",
            pointerId: 2,
            clientX: x,
            clientY: y,
          }),
        );
      });
    open(c);
    // The browser cancels a touch that scrolls; iOS at times lets it lift instead, far from where it
    // went down.
    touch("pointerdown", 100, 600);
    touch("pointercancel", 100, 580);
    expect(expanded(c)).toBe(true);
    touch("pointerdown", 100, 600);
    touch("pointerup", 100, 300);
    expect(expanded(c)).toBe(true);
    touch("pointerdown", 100, 600);
    expect(expanded(c)).toBe(true);
    touch("pointerup", 102, 601);
    expect(expanded(c)).toBe(false);
    open(c);
    act(() => {
      document.body.dispatchEvent(new PointerEvent("pointerdown", { bubbles: true, pointerType: "mouse" }));
    });
    expect(expanded(c)).toBe(false);
  });

  it("heads each account's choices for a project without repositories, and keeps to the one chosen", () => {
    const shop = { ...repo("shop", { private: true }), owner: "romanescohq", full_name: "romanescohq/shop" };
    const c = render(
      <Tab listed={[]} repos={[...available, shop]} accounts={["hyperstructure", "romanescohq"]} />,
    );
    open(c);
    const headings = () =>
      [...c.querySelectorAll('[role="listbox"] > [role="presentation"]')].map((h) => h.textContent);
    expect(headings()).toEqual(["hyperstructure", "romanescohq"]);
    act(() => option(c, "romanescohq/shop").click());
    expect(chips(c)).toEqual(["romanescohq/shop"]);
    const fctl = option(c, "hyperstructure/fctl");
    expect(fctl.getAttribute("aria-disabled")).toBe("true");
    expect(fctl.textContent).toContain("Another account");
    expect(option(c, "hyperstructure/*").textContent).not.toContain("Every repository");
    expect(option(c, "romanescohq/*").getAttribute("aria-disabled")).toBeNull();
    // Typing finds them across accounts, without the headings.
    type(c, "fctl");
    expect(headings()).toEqual([]);
  });
});

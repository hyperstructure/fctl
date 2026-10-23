import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { RunnerUpdate } from "./runner-update";

// No request is answered: the notice shows what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = new Proxy({}, { get: () => () => new Promise(() => {}) });
  return { api: pending, workspaces: pending, ApiFailure: class extends Error {} };
});

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  localStorage.clear();
});

const instance = (version: string): G.InstanceInfo => ({
  name: "fctl",
  version,
  setup_required: false,
  features: {} as G.Features,
});

/** The notice for `user`, with the control plane at `version` and these runners in the cache. */
function notice(runners?: G.Runner[], version: string | null = "0.16.0", user = "usr_ada") {
  const client = new QueryClient();
  if (runners) client.setQueryData<G.RunnerList>(keys.runners, { runners });
  if (version) client.setQueryData<G.InstanceInfo>(keys.instance, instance(version));
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() =>
    root.render(
      <QueryClientProvider client={client}>
        <RunnerUpdate user={user} />
      </QueryClientProvider>,
    ),
  );
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container;
}

const runner = (r: Partial<G.Runner> & { id: string }): G.Runner => ({
  name: r.id,
  status: "online",
  version: "0.15.0",
  drivers: [],
  capacity: 4,
  active_sessions: 0,
  labels: [],
  created_at: 0,
  yours: true,
  ...r,
});

const button = (container: HTMLElement, label: string) => {
  const b = [...container.querySelectorAll("button")].find((x) => x.textContent === label);
  if (!b) throw new Error(`no button ${label}`);
  return b;
};

describe("the notice to update runners", () => {
  it("asks you to update your runner, and says how when asked", () => {
    const text = notice([runner({ id: "studio", name: "studio mac" })]);
    expect(text.querySelector("section > div")?.textContent).toBe(
      "Update your runner to fctl 0.16.0studio mac runs 0.15.0Skip this versionHow to update",
    );
    const how = button(text, "How to update");
    const details = text.querySelector<HTMLElement>(
      `#${CSS.escape(how.getAttribute("aria-controls") ?? "")}`,
    );
    expect(how.getAttribute("aria-expanded")).toBe("false");
    expect(details?.hidden).toBe(true);
    act(() => how.click());
    expect(how.getAttribute("aria-expanded")).toBe("true");
    expect(details?.hidden).toBe(false);
    expect(details?.textContent).toContain("Run this on studio mac:fctl self update");
    expect(details?.textContent).toContain("This notice goes away once it reconnects with the new version.");
    expect(details?.textContent).toContain(
      "From then on, it updates itself whenever the app runs a newer fctl; fctl host auto-update off turns that off.",
    );
    expect(details?.textContent).toContain(
      "Installed fctl with Homebrew? Run brew upgrade fctl instead, then fctl host restart: Homebrew's fctl doesn't update itself.",
    );
  });

  it("names each of your runners that runs an older version", () => {
    const text = notice([
      runner({ id: "studio", name: "studio mac" }),
      runner({ id: "mini", version: "0.14.2" }),
      runner({ id: "current", version: "0.16.0" }),
    ]);
    expect(text.textContent).toContain("Update your runners to fctl 0.16.0");
    expect(text.textContent).toContain("studio mac runs 0.15.0 and mini runs 0.14.2");
    expect(text.textContent).toContain("Run this on studio mac and mini:");
    expect(text.textContent).toContain("This notice goes away once they reconnect with the new version.");
  });

  it("says nothing of runners up to date, offline, someone else's or that update themselves", () => {
    expect(
      notice([
        runner({ id: "current", version: "0.16.0" }),
        runner({ id: "ahead", version: "0.17.0" }),
        runner({ id: "asleep", status: "offline" }),
        runner({ id: "off", status: "disabled" }),
        runner({ id: "theirs", yours: false }),
        runner({ id: "updating", auto_update: true }),
      ]).textContent,
    ).toBe("");
  });

  it("asks you to update runners that don't update themselves, without saying they will", () => {
    const text = notice([
      runner({ id: "manual", name: "manual mac", auto_update: false }),
      runner({ id: "updating", auto_update: true }),
    ]);
    expect(text.textContent).toContain("Update your runner to fctl 0.16.0manual mac runs 0.15.0");
    expect(text.textContent).toContain("This notice goes away once it reconnects with the new version.");
    expect(text.textContent).not.toContain("From then on");
  });

  it("says runners from before update themselves once updated", () => {
    const text = notice([runner({ id: "studio" }), runner({ id: "mini" })]);
    expect(text.textContent).toContain(
      "From then on, they update themselves whenever the app runs a newer fctl; fctl host auto-update off turns that off.",
    );
  });

  it("says nothing before the runners and the control plane's version load", () => {
    expect(notice(undefined).textContent).toBe("");
    expect(notice([runner({ id: "studio" })], null).textContent).toBe("");
  });

  it("stays away once you skip the version, until a newer one", () => {
    const text = notice([runner({ id: "studio" })]);
    act(() => button(text, "Skip this version").click());
    expect(text.textContent).toBe("");
    // In this browser, for you: not for someone else who signs in here.
    expect(notice([runner({ id: "studio" })]).textContent).toBe("");
    expect(notice([runner({ id: "studio" })], "0.16.0", "usr_grace").textContent).toContain(
      "Update your runner to fctl 0.16.0",
    );
    expect(notice([runner({ id: "studio" })], "0.17.0").textContent).toContain(
      "Update your runner to fctl 0.17.0",
    );
  });
});

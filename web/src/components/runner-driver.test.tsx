import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { DriverLine } from "./runner-driver";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const runner = (r: Partial<G.Runner>): G.Runner => ({
  id: "rnr_01STUDIO",
  name: "studio",
  status: "online",
  driver: "docker",
  drivers: [
    {
      name: "docker",
      available: false,
      isolation: "container",
      detail: "the Docker daemon isn't running: `sudo systemctl enable --now docker`",
    },
  ],
  capacity: 4,
  active_sessions: 0,
  labels: [],
  created_at: 0,
  workspaces: [],
  yours: true,
  personal: [],
  ...r,
});

/** The line under `r`, as text. */
function line(r: G.Runner): string {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<DriverLine runner={r} />));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return container.textContent ?? "";
}

describe("DriverLine", () => {
  it("says that sessions wait while a runner's driver isn't ready", () => {
    expect(line(runner({ waits_for_driver: true }))).toBe(
      "Its docker driver isn't ready: the Docker daemon isn't running: sudo systemctl enable --now docker. No new sessions start on it until it is.",
    );
  });

  it("says that sessions fail on an older host's runner", () => {
    expect(line(runner({}))).toMatch(/Sessions that start on it fail until it is\.$/);
  });

  it("says nothing while its driver is ready or it is offline", () => {
    expect(line(runner({ status: "offline", waits_for_driver: true }))).toBe("");
    const ready = { name: "docker", available: true, isolation: "container", detail: "ready" };
    expect(line(runner({ drivers: [ready] }))).toBe("");
  });
});

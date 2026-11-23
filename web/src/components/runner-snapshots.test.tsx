import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { SnapshotStoreLine } from "./runner-snapshots";

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
});

const runner = (r: Partial<G.Runner>): G.Runner => ({
  id: "rnr_01AWS7",
  name: "aws-7",
  status: "online",
  drivers: [],
  capacity: 3,
  active_sessions: 0,
  labels: [],
  created_at: 0,
  workspaces: [],
  yours: true,
  personal: [],
  ...r,
});

/** The line under `r`: its text and whether it is amber. */
function line(r: G.Runner): { text: string; amber: boolean } {
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<SnapshotStoreLine runner={r} />));
  roots.push(() => {
    act(() => root.unmount());
    container.remove();
  });
  return {
    text: container.textContent ?? "",
    amber: container.querySelector(".text-amber") !== null,
  };
}

describe("SnapshotStoreLine", () => {
  it("says nothing of a runner that keeps snapshots in the control plane", () => {
    expect(line(runner({})).text).toBe("");
  });

  it("names the bucket a runner keeps snapshots in", () => {
    const l = line(runner({ snapshot_store: { url: "s3://acme-fctl/sessions" } }));
    expect(l.text).toBe("Keeps sessions' snapshots in s3://acme-fctl/sessions");
    expect(l.amber).toBe(false);
  });

  it("says in amber why the bucket refused the last one, and what follows", () => {
    const l = line(
      runner({
        snapshot_store: {
          url: "s3://acme-fctl/sessions",
          error: "putting it in the bucket failed: AccessDenied",
          failed_at: Date.now() - 120_000,
        },
      }),
    );
    expect(l.amber).toBe(true);
    expect(l.text).toContain("Its bucket s3://acme-fctl/sessions refused the last snapshot");
    expect(l.text).toContain(": putting it in the bucket failed: AccessDenied.");
    expect(l.text).toContain(
      "Sessions it stops resume elsewhere from their branch until it takes them again.",
    );
  });
});

import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { ReactNode } from "react";
import { act } from "react";
import { createRoot } from "react-dom/client";
import { afterEach, describe, expect, it, vi } from "vitest";
import type * as G from "@/gen";
import { keys } from "@/lib/queries";
import { SnapshotSettings, SnapshotsSection } from "./snapshots";

const update = vi.fn((_id: string, _b: G.UpdateWorkspaceRequest) => Promise.resolve({}));

// Only an update is answered: the sections show what the cache holds.
vi.mock("@/lib/api", () => {
  const pending = () => new Promise(() => {});
  return {
    api: new Proxy({}, { get: () => pending }),
    workspaces: new Proxy(
      {},
      {
        get: (_, k) =>
          k === "update" ? (id: string, b: G.UpdateWorkspaceRequest) => update(id, b) : pending,
      },
    ),
    ApiFailure: class extends Error {},
  };
});

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a href="/">{children}</a>,
}));

(globalThis as { IS_REACT_ACT_ENVIRONMENT?: boolean }).IS_REACT_ACT_ENVIRONMENT = true;

const roots: (() => void)[] = [];
afterEach(() => {
  for (const unmount of roots.splice(0)) unmount();
  document.body.innerHTML = "";
  update.mockClear();
});

const acme = {
  id: "wsp_acme",
  slug: "acme",
  name: "Acme",
  role: "admin",
  snapshots: true,
  snapshot_days: 30,
  primary: { id: "usr_ada", name: "Ada" },
} as G.Workspace;

const usage = (u: Partial<G.WorkspaceSnapshots> = {}): G.WorkspaceSnapshots => ({
  instance: true,
  max_mb: 200,
  kept: { count: 14, bytes: 210_000_000 },
  buckets: [{ bucket: "s3://acme-fctl", count: 3, bytes: 1_200_000_000 }],
  ...u,
});

function show(node: ReactNode, u?: G.WorkspaceSnapshots) {
  const client = new QueryClient();
  if (u) client.setQueryData(keys.workspaceSnapshots(acme.id), u);
  const container = document.createElement("div");
  document.body.append(container);
  const root = createRoot(container);
  act(() => root.render(<QueryClientProvider client={client}>{node}</QueryClientProvider>));
  roots.push(() => act(() => root.unmount()));
  return container;
}

const button = (label: string) => {
  const b = [...document.querySelectorAll("button")].find((x) => x.textContent === label);
  if (!b) throw new Error(`no button ${label}`);
  return b;
};

describe("the Runners tab's snapshots", () => {
  it("say where they are now, and how long each is kept", () => {
    const text = show(<SnapshotsSection w={acme} admin />, usage()).textContent ?? "";
    expect(text).toContain("Sessions that resume anywhere");
    expect(text).toContain(
      "On, each kept 30 days after it was saved: 14 snapshots with fctl (210 MB) and 3 in s3://acme-fctl (1.2 GB).",
    );
    expect(text).toContain("Change in Settings");
    expect(text).toContain("fctl host install --snapshot-bucket s3://your-bucket/fctl");
    expect(text).toContain("a lifecycle rule that deletes snapshots after 30 days");
  });

  it("say in amber that the instance keeps none, and to members who chooses", () => {
    const off = show(<SnapshotsSection w={acme} admin />, usage({ instance: false }));
    expect(off.querySelector(".text-amber")?.textContent).toContain(
      "This instance keeps no snapshots: its admins turned them off.",
    );
    const member = show(<SnapshotsSection w={{ ...acme, role: "member" }} admin={false} />, usage());
    expect(member.textContent).toContain(
      "Its admins choose this in its settings (Ada is its primary admin).",
    );
    expect(member.textContent).not.toContain("Change in Settings");
  });

  it("say what follows when the workspace turned them off", () => {
    const text = show(<SnapshotsSection w={{ ...acme, snapshots: false }} admin />, usage()).textContent;
    expect(text).toContain("Off: a session that resumes on another runner starts from its task and branch.");
  });
});

describe("the workspace's snapshot settings", () => {
  it("ask before turning them off, counting what goes", async () => {
    show(<SnapshotSettings w={acme} />, usage());
    act(() => button("").click());
    const dialog = document.querySelector("dialog")?.textContent ?? "";
    expect(dialog).toContain("Turn session snapshots off?");
    expect(dialog).toContain("The 14 snapshots fctl keeps for Acme's sessions are deleted within the hour");
    expect(update).not.toHaveBeenCalled();
    await act(async () => button("Turn them off").click());
    expect(update).toHaveBeenCalledWith("wsp_acme", { snapshots: false });
  });

  it("turn them on at once, and show the days only while on", async () => {
    const off = show(<SnapshotSettings w={{ ...acme, snapshots: false }} />, usage());
    expect(off.textContent).not.toContain("Keep each one");
    await act(async () => button("").click());
    expect(update).toHaveBeenCalledWith("wsp_acme", { snapshots: true });
    const on = show(<SnapshotSettings w={acme} />, usage());
    expect(on.textContent).toContain("Keep each one");
    expect(on.textContent).toContain(
      "Now: 14 snapshots with fctl (210 MB) and 3 in s3://acme-fctl (1.2 GB).",
    );
  });
});

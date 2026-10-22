import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { older, outdated, parseVersion } from "./updates";

describe("versions", () => {
  it("read as fctl self update reads them", () => {
    expect(parseVersion("0.16.0")).toEqual([0, 16, 0]);
    expect(parseVersion("v1.10.2\n")).toEqual([1, 10, 2]);
    expect(parseVersion("1.2")).toBeUndefined();
    expect(parseVersion("0.16.0-dev")).toBeUndefined();
    expect(parseVersion("latest")).toBeUndefined();
    expect(parseVersion(undefined)).toBeUndefined();
  });

  it("compare by their numbers", () => {
    expect(older("0.15.3", "0.16.0")).toBe(true);
    expect(older("0.9.9", "0.10.0")).toBe(true);
    expect(older("0.16.0", "0.16.1")).toBe(true);
    expect(older("0.16.0", "1.0.0")).toBe(true);
    expect(older("0.16.0", "0.16.0")).toBe(false);
    expect(older("0.17.0", "0.16.0")).toBe(false);
    expect(older("1.0.0", "0.99.99")).toBe(false);
  });

  it("are never older than something that isn't one", () => {
    expect(older(undefined, "0.16.0")).toBe(false);
    expect(older("0.15.0", undefined)).toBe(false);
    expect(older("dev", "0.16.0")).toBe(false);
  });
});

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

describe("the runners to update", () => {
  it("are yours, online, and older than the control plane", () => {
    const list = [
      runner({ id: "studio" }),
      runner({ id: "mini", version: "0.16.0" }),
      runner({ id: "ahead", version: "0.17.0" }),
      runner({ id: "theirs", yours: false }),
      runner({ id: "asleep", status: "offline" }),
      runner({ id: "off", status: "disabled" }),
      runner({ id: "new", version: undefined }),
      runner({ id: "draining", drain: { since: 1 } }),
    ];
    expect(outdated(list, "0.16.0").map((r) => r.id)).toEqual(["studio", "draining"]);
  });

  it("are none while the control plane's version is unknown", () => {
    expect(outdated([runner({ id: "studio" })], undefined)).toEqual([]);
  });
});

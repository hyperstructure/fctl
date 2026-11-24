import { describe, expect, it } from "vitest";
import { bucketOf, holds, snapshotDays, usageWords } from "./snapshots";

describe("holds", () => {
  it("says what a snapshot holds as fctl show does", () => {
    expect(holds({ conversation: true, changes: true })).toBe("its conversation and uncommitted work");
    expect(holds({ conversation: true, changes: false, left_out: ["images"] })).toBe(
      "its conversation, without its images",
    );
    expect(holds({ conversation: false, changes: true })).toBe("its uncommitted work");
    expect(holds({ conversation: true, changes: false, left_out: ["images", "changes"] })).toBe(
      "its conversation, without its images or changes",
    );
  });
});

describe("bucketOf", () => {
  it("takes the bucket of a location in one", () => {
    expect(bucketOf("s3://acme-fctl/sessions/ses_1/snp_2")).toBe("s3://acme-fctl");
    expect(bucketOf("s3://acme-fctl")).toBeUndefined();
    expect(bucketOf("https://acme-fctl/k")).toBeUndefined();
  });
});

describe("usageWords", () => {
  const none = { count: 0, bytes: 0 };
  it("says where a workspace's snapshots are, and what they take", () => {
    expect(usageWords({ kept: none, buckets: [] })).toBe("none yet");
    expect(usageWords({ kept: { count: 1, bytes: 1_500 }, buckets: [] })).toBe("1 snapshot with fctl (2 KB)");
    expect(
      usageWords({
        kept: { count: 14, bytes: 210_000_000 },
        buckets: [{ bucket: "s3://acme-fctl", count: 3, bytes: 1_200_000_000 }],
      }),
    ).toBe("14 snapshots with fctl (210 MB) and 3 in s3://acme-fctl (1.2 GB)");
    expect(
      usageWords({
        kept: none,
        buckets: [
          { bucket: "s3://acme-fctl", count: 3, bytes: 3_000_000 },
          { bucket: "s3://acme-eu", count: 1, bytes: 500_000 },
        ],
      }),
    ).toBe("3 snapshots in s3://acme-fctl (3.0 MB) and 1 in s3://acme-eu (500 KB)");
  });
});

describe("snapshotDays", () => {
  it("offers the usual days, the default said, and the workspace's own", () => {
    const days = snapshotDays(45);
    expect(days.map((d) => d.value)).toEqual(["1", "3", "7", "14", "30", "45", "60", "90", "180", "365"]);
    expect(days.find((d) => d.value === "30")?.description).toBe("The default");
    expect(days.find((d) => d.value === "1")?.label).toBe("1 day");
  });
});

import { describe, expect, it } from "vitest";
import {
  agoShort,
  browserName,
  cpus,
  diskBelow,
  duration,
  initials,
  lowOnDisk,
  memory,
  sandboxLimits,
  until,
} from "./format";

describe("format", () => {
  const now = Date.UTC(2026, 9, 17, 12, 0, 0);

  it("formats compact relative times", () => {
    expect(agoShort(undefined, now)).toBe("");
    expect(agoShort(now - 10_000, now)).toBe("now");
    expect(agoShort(now - 4 * 60_000, now)).toBe("4m");
    expect(agoShort(now - 3 * 3_600_000, now)).toBe("3h");
    expect(agoShort(now - 2 * 86_400_000, now)).toBe("2d");
    // Clock skew never shows a negative time.
    expect(agoShort(now + 60_000, now)).toBe("now");
  });

  it("formats durations", () => {
    expect(duration(420)).toBe("420 ms");
    expect(duration(7_100)).toBe("7.1 s");
    expect(duration(42_000)).toBe("42 s");
    expect(duration(125_000)).toBe("2m 5s");
    expect(duration(3_900_000)).toBe("1h 5m");
  });

  it("formats countdowns", () => {
    expect(until(now + 30_000, now)).toBe("30s");
    expect(until(now + 10 * 60_000, now)).toBe("10 min");
    expect(until(now - 1, now)).toBe("0s");
  });

  it("formats sandbox sizes", () => {
    expect(memory(512)).toBe("512 MiB");
    expect(memory(1024)).toBe("1 GiB");
    expect(memory(1536)).toBe("1.5 GiB");
    expect(memory(8192)).toBe("8 GiB");
    expect(memory(15_990)).toBe("15.6 GiB");
    expect(cpus(1)).toBe("1 CPU");
    expect(cpus(10)).toBe("10 CPUs");
    expect(
      sandboxLimits({ default: { cpus: 4, memory_mb: 8192 }, max: { cpus: 10, memory_mb: 15_990 } }),
    ).toBe("4 CPUs and 8 GiB by default, at most 10 CPUs and 15.6 GiB");
  });

  it("says when a runner is low on disk", () => {
    expect(lowOnDisk({ disk_available_mb: 12_595, min_free_disk_mb: 20_480 })).toBe(true);
    expect(lowOnDisk({ disk_available_mb: 20_480, min_free_disk_mb: 20_480 })).toBe(false);
    // Offline runners report no free disk, and older hosts keep no floor.
    expect(lowOnDisk({ min_free_disk_mb: 20_480 })).toBe(false);
    expect(lowOnDisk({ disk_available_mb: 0 })).toBe(false);
    expect(diskBelow(12_595, 20_480)).toBe("12.3 GiB of disk free, below the 20 GiB it keeps free");
  });

  it("derives initials", () => {
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("  grace  ")).toBe("G");
  });

  it("names browsers", () => {
    const mac =
      "Mozilla/5.0 (Macintosh; Intel Mac OS X 10_15_7) AppleWebKit/537.36 (KHTML, like Gecko) Chrome/141.0.0.0 Safari/537.36";
    const iphone =
      "Mozilla/5.0 (iPhone; CPU iPhone OS 26_0 like Mac OS X) AppleWebKit/605.1.15 (KHTML, like Gecko) Version/26.0 Mobile/15E148 Safari/604.1";
    const firefox = "Mozilla/5.0 (X11; Linux x86_64; rv:144.0) Gecko/20100101 Firefox/144.0";
    expect(browserName(mac)).toBe("Chrome on macOS");
    expect(browserName(iphone)).toBe("Safari on iOS");
    expect(browserName(firefox)).toBe("Firefox on Linux");
    expect(browserName(undefined)).toBe("Browser");
  });
});

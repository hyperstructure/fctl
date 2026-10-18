import { describe, expect, it } from "vitest";
import { agoShort, cpus, duration, initials, memory, sandboxLimits, until } from "./format";

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

  it("derives initials", () => {
    expect(initials("Ada Lovelace")).toBe("AL");
    expect(initials("  grace  ")).toBe("G");
  });
});

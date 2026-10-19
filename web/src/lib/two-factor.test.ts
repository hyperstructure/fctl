import { describe, expect, it } from "vitest";
import {
  clock,
  digits,
  formatRecoveryCode,
  groupKey,
  isRecoveryCode,
  qrRings,
  recoveryCodesFile,
  recoveryCodesText,
} from "./two-factor";

describe("two-factor helpers", () => {
  it("groups keys in fours", () => {
    expect(groupKey("JBSWY3DPEHPK3PXP")).toBe("JBSW Y3DP EHPK 3PXP");
    expect(groupKey("ABCDEF")).toBe("ABCD EF");
    expect(groupKey("")).toBe("");
  });

  it("keeps the digits of a code", () => {
    expect(digits("123 456")).toBe("123456");
    expect(digits("12-34-56-78")).toBe("123456");
    expect(digits("abc")).toBe("");
  });

  it("formats recovery codes as they are typed", () => {
    expect(formatRecoveryCode("kftwq")).toBe("KFTWQ");
    expect(formatRecoveryCode("kftwqx")).toBe("KFTWQ-X");
    expect(formatRecoveryCode("KFTWQ-XPDMB")).toBe("KFTWQ-XPDMB");
    expect(formatRecoveryCode(" kftwq xpdmb extra")).toBe("KFTWQ-XPDMB");
    // Vowels and look-alikes are never in a code.
    expect(formatRecoveryCode("AEIOU01")).toBe("");
    expect(isRecoveryCode("kftwq-xpdmb")).toBe(true);
    expect(isRecoveryCode("kftwq-xplm")).toBe(false);
  });

  it("writes the codes to a file to keep", () => {
    const text = recoveryCodesText(["BCDFG-HJKMN", "PQRST-VWXZ2"], "fctl dev", "dev@fctl.test", new Date(0));
    expect(text).toContain("fctl dev recovery codes for dev@fctl.test");
    expect(text).toContain("1970-01-01");
    expect(text.trim().split("\n").slice(-2)).toEqual(["BCDFG-HJKMN", "PQRST-VWXZ2"]);
    expect(recoveryCodesFile("fctl dev")).toBe("fctl-dev-recovery-codes.txt");
    expect(recoveryCodesFile("Acme's factory!")).toBe("acme-s-factory-recovery-codes.txt");
    expect(recoveryCodesFile("???")).toBe("fctl-recovery-codes.txt");
  });

  it("counts down like a clock", () => {
    expect(clock(47_000)).toBe("0:47");
    expect(clock(46_100)).toBe("0:47");
    expect(clock(125_000)).toBe("2:05");
    expect(clock(3_600_000)).toBe("1:00:00");
    expect(clock(-5)).toBe("0:00");
  });

  it("splits a QR code into rings from the middle out", () => {
    const rows = ["101", "010", "101"];
    const rings = qrRings(rows, 0);
    expect(rings.map((r) => r.ring)).toEqual([0, 1]);
    expect(rings[0]?.d).toBe("M1 1h1v1h-1z");
    expect(rings[1]?.d.match(/M/g)).toHaveLength(4);
    expect(qrRings(rows)[0]?.d).toBe("M5 5h1v1h-1z");
  });
});

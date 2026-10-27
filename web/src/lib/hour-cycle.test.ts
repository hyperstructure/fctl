import { describe, expect, it } from "vitest";
import {
  clockOptions,
  type HourCycle,
  hourCycle,
  localeHourCycle,
  pickedHourCycle,
  setHourCycle,
} from "./hour-cycle";

/** A time of day in `locale` on `cycle`, with plain spaces (CLDR puts a narrow one before "PM"). */
const at = (locale: string, cycle: HourCycle, h: number, m: number) =>
  new Intl.DateTimeFormat(locale, { ...clockOptions(cycle, localeHourCycle(locale)), timeZone: "UTC" })
    .format(Date.UTC(2026, 9, 27, h, m))
    .replace(/\s/g, " ");

describe("hour cycles", () => {
  it("are the locale's", () => {
    expect(localeHourCycle("en-US")).toBe(12);
    expect(localeHourCycle("ko-KR")).toBe(12);
    expect(localeHourCycle("en-GB")).toBe(24);
    expect(localeHourCycle("sv-SE")).toBe(24);
    expect(localeHourCycle("ja-JP")).toBe(24);
  });

  it("write times as the locale does on its own clock", () => {
    // No leading zero on 12 hours, as in "07:40 PM".
    expect(at("en-US", 12, 19, 40)).toBe("7:40 PM");
    expect(at("en-US", 12, 9, 5)).toBe("9:05 AM");
    expect(at("en-GB", 24, 19, 40)).toBe("19:40");
    expect(at("en-GB", 24, 9, 5)).toBe("09:05");
    expect(at("sv-SE", 24, 9, 5)).toBe("09:05");
  });

  it("write times on a clock the locale doesn't use", () => {
    expect(at("en-US", 24, 19, 40)).toBe("19:40");
    expect(at("en-US", 24, 9, 5)).toBe("09:05");
    expect(at("en-US", 24, 0, 5)).toBe("00:05");
    expect(at("en-GB", 12, 19, 40)).toBe("7:40 pm");
    expect(at("en-GB", 12, 0, 5)).toBe("12:05 am");
  });

  it("keep the clock picked in this browser", () => {
    expect(pickedHourCycle()).toBeUndefined();
    expect(hourCycle()).toBe(localeHourCycle());
    setHourCycle(24);
    expect(hourCycle()).toBe(24);
    expect(localStorage.getItem("fctl-clock")).toBe("24");
    setHourCycle(12);
    expect(pickedHourCycle()).toBe(12);
    expect(localStorage.getItem("fctl-clock")).toBe("12");
  });
});

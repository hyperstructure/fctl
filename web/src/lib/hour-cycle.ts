// 12- or 24-hour times. They follow the browser's language until someone picks a clock in
// Settings, which this browser keeps, as it does the theme: browsers keep the system's own
// choice from pages, so Safari on an iPhone set to English (US) writes 7:40 PM even with
// 24-Hour Time on (https://bugs.webkit.org/show_bug.cgi?id=240047).
import { useSyncExternalStore } from "react";

export type HourCycle = 12 | 24;

const KEY = "fctl-clock";
const listeners = new Set<() => void>();

/** The clock `locale` writes times on; without one, the browser's language's. */
export function localeHourCycle(locale?: string): HourCycle {
  const o = new Intl.DateTimeFormat(locale, { hour: "numeric" }).resolvedOptions();
  const cycle = o.hourCycle ?? (o.hour12 === false ? "h23" : "h12");
  return cycle === "h23" || cycle === "h24" ? 24 : 12;
}

let own: HourCycle | undefined;
const browserHourCycle = (): HourCycle => {
  own ??= localeHourCycle();
  return own;
};

// Read once: this browser's pick changes only here.
let picked: HourCycle | undefined | null = null;

/** The clock picked in Settings, if one is. */
export function pickedHourCycle(): HourCycle | undefined {
  if (picked === null) {
    let saved: string | null = null;
    try {
      saved = localStorage.getItem(KEY);
    } catch {}
    picked = saved === "12" ? 12 : saved === "24" ? 24 : undefined;
  }
  return picked;
}

/** The clock times show on. */
export const hourCycle = (): HourCycle => pickedHourCycle() ?? browserHourCycle();

export function setHourCycle(h: HourCycle) {
  picked = h;
  try {
    localStorage.setItem(KEY, String(h));
  } catch {}
  for (const l of listeners) l();
}

export function useHourCycle(): [HourCycle, (h: HourCycle) => void] {
  const h = useSyncExternalStore((fn) => {
    listeners.add(fn);
    return () => listeners.delete(fn);
  }, hourCycle);
  return [h, setHourCycle];
}

/**
 * Intl.DateTimeFormat's options for a time of day on `cycle` where the locale's clock is `own`:
 * "7:40 PM" on 12 hours, "07:40" and "19:40" on 24. Only a clock the locale doesn't use sets
 * `hourCycle`, so that the locale keeps its own way of writing it.
 */
export function clockOptions(cycle: HourCycle, own: HourCycle): Intl.DateTimeFormatOptions {
  return {
    hour: cycle === 24 ? "2-digit" : "numeric",
    minute: "2-digit",
    ...(cycle !== own && { hourCycle: cycle === 24 ? "h23" : "h12" }),
  };
}

/** The options for a time of day on the clock times show on. */
export const timeOptions = (): Intl.DateTimeFormatOptions => clockOptions(hourCycle(), browserHourCycle());

/** 19:40 on `cycle`, as the browser's language writes it: "7:40 PM", "19:40". */
export const sampleTime = (cycle: HourCycle): string =>
  dateTimeFormat({ ...clockOptions(cycle, browserHourCycle()), timeZone: "UTC" }).format(
    Date.UTC(2000, 0, 1, 19, 40),
  );

const formats = new Map<string, Intl.DateTimeFormat>();

/** An Intl.DateTimeFormat in the browser's language, made once for each set of options. */
export function dateTimeFormat(options: Intl.DateTimeFormatOptions): Intl.DateTimeFormat {
  const key = JSON.stringify(options);
  let f = formats.get(key);
  if (!f) {
    f = new Intl.DateTimeFormat(undefined, options);
    formats.set(key, f);
  }
  return f;
}

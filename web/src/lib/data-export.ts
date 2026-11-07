// Exporting your data (Settings → Your data), which the instance allows once
// in each of its `export_interval_hours` (`G.DataExport`).

import type * as G from "@/gen";
import { ago, clock, day } from "./format";

const HOUR_MS = 3_600_000;

/** How often people can export their data: "once a week", "once every 3 days", as the CLI says it. */
export function exportEvery(hours: number): string {
  if (hours === 0) return "at any time";
  if (hours === 1) return "once an hour";
  if (hours === 24) return "once a day";
  if (hours === 168) return "once a week";
  if (hours === 8760) return "once a year";
  if (hours % 168 === 0) return `once every ${hours / 168} weeks`;
  if (hours % 24 === 0) return `once every ${hours / 24} days`;
  return `once every ${hours} hours`;
}

/** When you can export your data next, while you have to wait: undefined when you can now. */
export function nextExport(e: G.DataExport | undefined, now = Date.now()): number | undefined {
  return e?.next_at !== undefined && e.next_at > now ? e.next_at : undefined;
}

/** What the export button says under it: how often, and while you wait, until when. */
export function exportHint(e: G.DataExport | undefined, now = Date.now()): string | undefined {
  if (!e || e.interval_hours === 0) return undefined;
  const every = exportEvery(e.interval_hours);
  const next = nextExport(e, now);
  if (next === undefined) return `You can export it ${every}.`;
  const last =
    e.last_at === undefined
      ? ""
      : `You exported it ${now - e.last_at < 7 * 24 * HOUR_MS ? ago(e.last_at, now) : `on ${day(e.last_at)}`}. `;
  return `${last}You can export it ${every}: again from ${day(next)}, ${clock(next)}.`;
}

/** You, once an export started `now`: the control plane counts it from its start. */
export function exported(me: G.Me, now = Date.now()): G.Me {
  if (!me.export) return me;
  const hours = me.export.interval_hours;
  return {
    ...me,
    export: { ...me.export, last_at: now, next_at: hours > 0 ? now + hours * HOUR_MS : undefined },
  };
}

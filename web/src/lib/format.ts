// Formatting helpers: relative times, durations, numbers, sizes.

import type * as G from "@/gen";

const rtf = new Intl.RelativeTimeFormat(undefined, { numeric: "auto" });

export function ago(ms: number | undefined | null, now = Date.now()): string {
  if (!ms) return "";
  const s = Math.round((ms - now) / 1000);
  const a = Math.abs(s);
  if (a < 45) return "just now";
  if (a < 3600) return rtf.format(Math.round(s / 60), "minute");
  if (a < 86_400) return rtf.format(Math.round(s / 3600), "hour");
  if (a < 7 * 86_400) return rtf.format(Math.round(s / 86_400), "day");
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** Compact relative time for dense lists: "now", "4m", "3h", "2d", "Oct 3". */
export function agoShort(ms: number | undefined | null, now = Date.now()): string {
  if (!ms) return "";
  const a = Math.max(0, Math.round((now - ms) / 1000));
  if (a < 45) return "now";
  if (a < 3600) return `${Math.max(1, Math.round(a / 60))}m`;
  if (a < 86_400) return `${Math.round(a / 3600)}h`;
  if (a < 7 * 86_400) return `${Math.round(a / 86_400)}d`;
  return new Date(ms).toLocaleDateString(undefined, { month: "short", day: "numeric" });
}

/** A full timestamp for tooltips. */
export function stamp(ms: number): string {
  return new Date(ms).toLocaleString(undefined, { dateStyle: "medium", timeStyle: "short" });
}

export function duration(ms: number): string {
  if (ms < 1000) return `${Math.max(0, Math.round(ms))} ms`;
  const s = ms / 1000;
  if (s < 60) return `${s < 10 ? s.toFixed(1) : Math.round(s)} s`;
  const m = Math.floor(s / 60);
  if (m < 60) return `${m}m ${Math.round(s % 60)}s`;
  return `${Math.floor(m / 60)}h ${m % 60}m`;
}

const compact = new Intl.NumberFormat(undefined, { notation: "compact", maximumFractionDigits: 1 });
export const num = (n: number) => compact.format(n);

/** `912 B`, `245 KB`, `1.2 MB`: decimal units, as file managers (and fctl's Rust side) show them. */
export function bytes(n: number): string {
  if (n < 1000) return `${n} B`;
  if (n < 1e6) return `${Math.round(n / 1000)} KB`;
  if (n < 1e7) return `${(n / 1e6).toFixed(1)} MB`;
  if (n < 1e9) return `${Math.round(n / 1e6)} MB`;
  return `${(n / 1e9).toFixed(1)} GB`;
}

/** An amount of memory in MiB: "512 MiB", "8 GiB", "15.6 GiB" (GiB to a tenth from 1 GiB up). */
export function memory(mb: number): string {
  if (mb < 1024) return `${Math.round(mb)} MiB`;
  const tenths = Math.round((mb * 10) / 1024);
  return `${tenths % 10 === 0 ? tenths / 10 : (tenths / 10).toFixed(1)} GiB`;
}

/** "1 CPU", "4 CPUs". */
export const cpus = (n: number) => (n === 1 ? "1 CPU" : `${n} CPUs`);

/** What a runner gives sandboxes: "4 CPUs and 8 GiB by default, at most 10 CPUs and 15.6 GiB". */
export function sandboxLimits(l: G.SandboxLimits): string {
  // Hosts that cannot measure their memory and set no maximum send u32::MAX.
  const mem = (mb: number) => (mb >= 4_294_967_295 ? "unlimited memory" : memory(mb));
  return `${cpus(l.default.cpus)} and ${mem(l.default.memory_mb)} by default, at most ${cpus(l.max.cpus)} and ${mem(l.max.memory_mb)}`;
}

export function clock(ms: number): string {
  return new Date(ms).toLocaleTimeString(undefined, { hour: "2-digit", minute: "2-digit" });
}

export function until(ms: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((ms - now) / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.ceil(s / 60)} min`;
  return `${Math.floor(s / 3600)}h ${Math.round((s % 3600) / 60)}m`;
}

export function initials(name: string): string {
  const parts = name.trim().split(/\s+/);
  return (
    ((parts[0]?.[0] ?? "") + (parts.length > 1 ? (parts[parts.length - 1]?.[0] ?? "") : "")).toUpperCase() ||
    "?"
  );
}

/** A stable hue for a person, from their id. */
export function hue(id: string): number {
  let h = 0;
  for (let i = 0; i < id.length; i++) h = (h * 31 + id.charCodeAt(i)) >>> 0;
  return h % 360;
}

/** "Chrome on macOS", from a user agent. */
export function browserName(ua?: string): string {
  if (!ua) return "Browser";
  const name = /Edg\//.test(ua)
    ? "Edge"
    : /Chrome\//.test(ua)
      ? "Chrome"
      : /Firefox\//.test(ua)
        ? "Firefox"
        : /Safari\//.test(ua)
          ? "Safari"
          : "Browser";
  const os = /iPhone|iPad/.test(ua)
    ? "iOS"
    : /Mac OS X/.test(ua)
      ? "macOS"
      : /Windows/.test(ua)
        ? "Windows"
        : /Android/.test(ua)
          ? "Android"
          : /Linux/.test(ua)
            ? "Linux"
            : "";
  return os ? `${name} on ${os}` : name;
}

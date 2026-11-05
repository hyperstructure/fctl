// A workspace's enrollment tokens (G.EnrollmentToken): machines that autoscaling creates, or that
// someone sets up without signing in, enroll with one as the workspace's runners. Its admins make
// them on the Runners tab, which says what each does in a sentence, as `fctl runners
// enrollment-tokens` does (`fctl_domain::enrollment::describe`), and approve the runners that wait.

import type * as G from "@/gen";
import { cpus, memory } from "./format";

/** How long a new token works. */
export const ENROLLMENT_EXPIRIES: readonly { value: string; label: string; hours?: number }[] = [
  { value: "1h", label: "An hour", hours: 1 },
  { value: "1d", label: "A day", hours: 24 },
  { value: "7d", label: "7 days", hours: 7 * 24 },
  { value: "30d", label: "30 days", hours: 30 * 24 },
  { value: "90d", label: "90 days", hours: 90 * 24 },
  { value: "1y", label: "A year", hours: 365 * 24 },
  { value: "never", label: "Never" },
];

/**
 * What happens to the runners it enrolls once they go offline, in minutes of being offline while the
 * control plane runs: a deploy or an outage of its own doesn't count.
 */
export const OFFLINE_REMOVAL: readonly {
  value: string;
  label: string;
  description: string;
  minutes?: number;
}[] = [
  { value: "keep", label: "Keep them", description: "They stay until someone removes them." },
  {
    value: "1m",
    label: "Remove after a minute",
    description: "For machines that never come back: one that loses its network for longer is gone for good.",
    minutes: 1,
  },
  {
    value: "5m",
    label: "Remove after 5 minutes",
    description: "For machines autoscaling destroys without uninstalling.",
    minutes: 5,
  },
  {
    value: "15m",
    label: "Remove after 15 minutes",
    description: "The same, through a short network outage.",
    minutes: 15,
  },
  { value: "1h", label: "Remove after an hour", description: "Through a longer one.", minutes: 60 },
  {
    value: "1d",
    label: "Remove after a day",
    description: "For machines that are off overnight.",
    minutes: 1440,
  },
  { value: "7d", label: "Remove after a week", description: "Through a weekend.", minutes: 7 * 1440 },
];

/** A new enrollment token, as the form holds it. */
export interface EnrollmentForm {
  name: string;
  /** One of `ENROLLMENT_EXPIRIES`' values. */
  expires: string;
  /** The most runners at once, as typed: empty for no limit. */
  limit: string;
  approval: boolean;
  /** One of `OFFLINE_REMOVAL`'s values. */
  offline: string;
}

export const NEW_ENROLLMENT: EnrollmentForm = {
  name: "",
  expires: "90d",
  limit: "",
  approval: false,
  offline: "keep",
};

/** The most runners one token keeps at once (`fctl_domain::enrollment::MAX_RUNNERS`). */
export const MAX_RUNNERS = 1000;

/** The limit the form asks for: none (empty), a whole number in range, or `NaN` for anything else. */
export function limitOf(typed: string): number | undefined {
  const t = typed.trim();
  if (!t) return undefined;
  const n = Number(t);
  return Number.isInteger(n) && n >= 1 && n <= MAX_RUNNERS ? n : Number.NaN;
}

/** Whether the form makes a token: it has a name, and its limit, if any, is one. */
export const enrollmentReady = (f: EnrollmentForm) => !!f.name.trim() && !Number.isNaN(limitOf(f.limit));

/** The request that makes the token `f` describes. */
export function enrollmentRequest(f: EnrollmentForm): G.CreateEnrollmentTokenRequest {
  const req: G.CreateEnrollmentTokenRequest = { name: f.name.trim(), approval: f.approval };
  const expires = ENROLLMENT_EXPIRIES.find((e) => e.value === f.expires)?.hours;
  if (expires) req.expires_in_hours = expires;
  const limit = limitOf(f.limit);
  if (limit !== undefined && !Number.isNaN(limit)) req.max_runners = limit;
  const offline = OFFLINE_REMOVAL.find((o) => o.value === f.offline)?.minutes;
  if (offline) req.remove_offline_after_minutes = offline;
  return req;
}

/** `a minute`, `5 minutes`, `an hour`, `6 hours`, `a day`, `3 days`, `a week`, as the CLI says them. */
export function minutesText(m: number): string {
  if (m === 1) return "a minute";
  if (m === 60) return "an hour";
  if (m === 1440) return "a day";
  if (m === 10080) return "a week";
  if (m % 1440 === 0) return `${m / 1440} days`;
  if (m % 60 === 0) return `${m / 60} hours`;
  return `${m} minutes`;
}

/**
 * What a token does with the runners it enrolls, in a sentence, as the CLI says it: `Enrolls up to
 * 20 runners at a time, which wait for an admin's approval and are removed once they have been
 * offline for 5 minutes.` `removeOffline` is in minutes.
 */
export function describeEnrollment(approval: boolean, maxRunners?: number, removeOffline?: number): string {
  const one = maxRunners === 1;
  const howMany =
    maxRunners === undefined
      ? "Enrolls any number of runners"
      : one
        ? "Enrolls one runner at a time"
        : `Enrolls up to ${maxRunners} runners at a time`;
  const [runs, waits, is, has, they] = one
    ? ["runs", "waits", "is", "it has", "it runs"]
    : ["run", "wait", "are", "they have", "they run"];
  const then =
    removeOffline === undefined
      ? approval
        ? `, which ${waits} for an admin's approval before ${they} anything`
        : `, which ${runs} everyone's sessions at once`
      : approval
        ? `, which ${waits} for an admin's approval and ${is} removed once ${has} been offline for ${minutesText(removeOffline)}`
        : `, which ${runs} everyone's sessions at once and ${is} removed once ${has} been offline for ${minutesText(removeOffline)}`;
  return `${howMany}${then}.`;
}

/** Whether a token enrolls runners now, and why not. */
export function tokenState(t: G.EnrollmentToken, now: number): "active" | "revoked" | "expired" | "full" {
  if (t.revoked_at) return "revoked";
  if (t.expires_at && t.expires_at <= now) return "expired";
  if (t.max_runners !== undefined && t.runners >= t.max_runners) return "full";
  return "active";
}

/** The command that enrolls a machine with `secret`, from a shell or a boot script, a variable a line. */
export function enrollCommand(origin: string, secret: string): string {
  return `FCTL_SERVER=${origin} \\\nFCTL_ENROLL_TOKEN=${secret} \\\nfctl host install`;
}

/** `ip-10-0-3-17 · linux/x86_64 · docker · fctl 0.28.0`: what a runner said of its machine. */
export function machineFacts(r: G.Runner): string[] {
  return [
    r.hostname && r.hostname !== r.name ? r.hostname : undefined,
    r.os && r.arch ? `${r.os}/${r.arch}` : undefined,
    r.driver,
    r.cpus && r.memory_mb ? `${cpus(r.cpus)}, ${memory(r.memory_mb)}` : undefined,
    r.version ? `fctl ${r.version}` : undefined,
  ].filter((x): x is string => !!x);
}

/** `203.0.113.7 (Stockholm, SE)`: where a runner enrolled from, as far as the control plane knows. */
export function enrolledFrom(e: G.RunnerEnrollment | undefined): string | undefined {
  if (!e?.address) return undefined;
  return e.location ? `${e.address} (${e.location})` : e.address;
}

/** The runners that wait for approval in the workspaces you are an admin of, by workspace. */
export function waitingByWorkspace(runners: G.Runner[], workspaces: G.Workspace[]) {
  const admined = new Map(
    workspaces.filter((w) => !w.blocked && (w.role === "owner" || w.role === "admin")).map((w) => [w.id, w]),
  );
  const out = new Map<string, { workspace: G.Workspace; runners: G.Runner[] }>();
  for (const r of runners) {
    const w = r.pending_until && r.workspace ? admined.get(r.workspace.id) : undefined;
    if (!w) continue;
    const entry = out.get(w.id) ?? { workspace: w, runners: [] };
    entry.runners.push(r);
    out.set(w.id, entry);
  }
  return [...out.values()];
}

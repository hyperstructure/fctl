// Runner pools in the web app (docs/architecture.md#runner-pools): a workspace's enrollment token and
// the runners it enrolled, which carry its labels and follow its placement, and what it needs now
// (G.PoolDemand). Projects choose a pool by its labels. The words follow the CLI's
// (`fctl runners pools`) and `fctl_domain::pools`.

import type * as G from "@/gen";
import { memory } from "./format";

/** The most labels a pool or a project names (`fctl_domain::pools::MAX_LABELS`). */
export const MAX_LABELS = 16;
/** The longest label (`MAX_LABEL_CHARS`). */
export const MAX_LABEL_CHARS = 40;

/** What a placement does, as `fctl_domain::pools::placement_words` says it. */
export function placementWords(p: G.Placement | undefined): string {
  return p === "packed"
    ? "Packed: fills one runner before the next, so that idle ones can go"
    : "Spread: each session goes to the least busy runner";
}

/** The placements to choose from, each saying when to pick it. */
export const PLACEMENTS: readonly { value: G.Placement; label: string; description: string }[] = [
  {
    value: "spread",
    label: "Spread",
    description: "Each session goes to the least busy runner: for machines that stay on.",
  },
  {
    value: "packed",
    label: "Packed",
    description:
      "Fills one runner before the next, so that idle ones can go: for a pool that grows and shrinks.",
  },
];

/**
 * The placement a new token gets until someone chooses: packed for one that removes offline runners,
 * whose machines autoscaling makes and destroys (`fctl_domain::pools::default_placement`).
 */
export const defaultPlacement = (removeOfflineMinutes: number | undefined): G.Placement =>
  removeOfflineMinutes !== undefined ? "packed" : "spread";

/** Labels as typed: split at commas and spaces, trimmed, lowercase, each once, in order. */
export function parseLabels(text: string): string[] {
  const out: string[] = [];
  for (const l of text.split(/[\s,]+/)) {
    const label = l.trim().toLowerCase();
    if (label && !out.includes(label)) out.push(label);
  }
  return out;
}

/** Why labels aren't ones the server takes, in its words (`check_labels`); undefined when they are. */
export function labelProblem(labels: readonly string[]): string | undefined {
  for (const l of labels) {
    if (l.length > MAX_LABEL_CHARS || !/^[a-z0-9][a-z0-9_.:-]*$/.test(l))
      return `"${l}" isn't a label: use letters, digits, -, _, . and :, up to ${MAX_LABEL_CHARS} of them, starting with a letter or a digit`;
  }
  if (labels.length > MAX_LABELS) return `Name at most ${MAX_LABELS} labels`;
  return undefined;
}

/** `the label gpu`, `the labels gpu and large`, as `fctl_domain::pools::labels_words` says them. */
export function labelsWords(labels: readonly string[]): string {
  if (labels.length === 0) return "no labels";
  if (labels.length === 1) return `the label ${labels[0]}`;
  return `the labels ${labels.slice(0, -1).join(", ")} and ${labels[labels.length - 1]}`;
}

/**
 * A pool's labels and placement in two sentences, as `fctl runners enrollment-tokens` says them: "Its
 * runners carry the labels aws and large. Packed: fills one runner before the next, so that idle ones
 * can go."
 */
export function poolLine(labels: readonly string[], placement: G.Placement | undefined): string {
  const carry = labels.length
    ? `Its runners carry ${labelsWords(labels)}`
    : "No labels: projects that ask for none run on its runners";
  return `${carry}. ${placementWords(placement)}.`;
}

/** How long something has waited, as the CLI says it: `40s`, `2m`, `1h 5m`, `2d 3h`. */
export function waited(ms: number): string {
  const s = Math.max(0, Math.floor(ms / 1000));
  if (s < 60) return `${s}s`;
  if (s < 3600) return `${Math.floor(s / 60)}m`;
  if (s < 86_400) return `${Math.floor(s / 3600)}h ${Math.floor(s / 60) % 60}m`;
  return `${Math.floor(s / 86_400)}d ${Math.floor(s / 3600) % 24}h`;
}

const count = (n: number, one: string, many: string) => `${n} ${n === 1 ? one : many}`;

/** Its runners that are there, whatever they do now. */
export const enrolled = (r: G.PoolRunners) => r.online + r.draining + r.pending + r.starting + r.offline;

/**
 * The facts on a pool's card, each a short phrase: "6 of 8 runners busy", "3 starting", "11 of 32
 * sessions", "3 sessions waiting", "longest 2m".
 */
export function poolFacts(d: G.PoolDemand): { key: string; text: string; amber?: boolean }[] {
  const r = d.runners;
  const facts: { key: string; text: string; amber?: boolean }[] = [];
  const limit = d.pool.max_runners;
  facts.push({
    key: "runners",
    text:
      r.online > 0
        ? `${r.busy} of ${count(r.online, "runner", "runners")} busy`
        : limit !== undefined
          ? `${enrolled(r)} of ${limit} runners`
          : count(enrolled(r), "runner", "runners"),
  });
  if (r.starting) facts.push({ key: "starting", text: `${r.starting} starting` });
  if (r.draining) facts.push({ key: "draining", text: `${r.draining} draining` });
  if (r.pending) facts.push({ key: "pending", text: `${r.pending} waiting for approval`, amber: true });
  if (r.offline) facts.push({ key: "offline", text: `${r.offline} offline` });
  if (d.slots.capacity > 0)
    facts.push({ key: "slots", text: `${d.slots.busy} of ${d.slots.capacity} sessions` });
  if (d.waiting.sessions > 0) {
    facts.push({ key: "waiting", text: count(d.waiting.sessions, "session waiting", "sessions waiting") });
    if (d.waiting.since) facts.push({ key: "longest", text: `longest ${waited(d.at - d.waiting.since)}` });
  }
  return facts;
}

/**
 * What a pool is doing now, in a sentence, and its tone: amber where it needs someone (at its limit,
 * sessions waiting with nothing on its way), the accent while it grows.
 */
export function poolState(d: G.PoolDemand): { text: string; tone: "quiet" | "accent" | "amber" } {
  const r = d.runners;
  const n = d.waiting.sessions;
  const all = enrolled(r);
  const limit = d.pool.max_runners;
  if (d.pool.closed && all === 0) return { text: "Its token enrolls no more runners.", tone: "quiet" };
  if (n > 0) {
    const sessions = count(n, "session", "sessions");
    if (limit !== undefined && all >= limit)
      return {
        text: `At its limit of ${count(limit, "runner", "runners")}: ${sessions} ${n === 1 ? "waits" : "wait"} for room.`,
        tone: "amber",
      };
    if (r.starting > 0)
      return {
        text: `Growing: ${count(r.starting, "runner is", "runners are")} starting for ${sessions} that ${n === 1 ? "waits" : "wait"}.`,
        tone: "accent",
      };
    const longest = d.waiting.since ? `, the longest for ${waited(d.at - d.waiting.since)}` : "";
    return {
      text: `${sessions} ${n === 1 ? "waits" : "wait"}${longest}: it wants ${count(d.wanted, "runner", "runners")}, and has ${r.online} online.`,
      tone: "amber",
    };
  }
  if (all === 0)
    return { text: "No runners yet: machines that enroll with its token show here.", tone: "quiet" };
  if (r.online === 0) return { text: "No runner online, and no session waits for it.", tone: "quiet" };
  if (r.busy === 0)
    return {
      text: `Idle: ${r.online === 1 ? "its runner runs" : `its ${r.online} runners run`} nothing.`,
      tone: "quiet",
    };
  if (d.wanted < r.online)
    return {
      text: `Could shrink: it wants ${count(d.wanted, "runner", "runners")} of its ${r.online}.`,
      tone: "quiet",
    };
  return {
    text: `Busy: ${r.busy} of ${count(r.online, "runner", "runners")} ${r.busy === 1 ? "runs" : "run"} sessions.`,
    tone: "quiet",
  };
}

/**
 * A pool as a project chooses it: "4 runners with 16 CPUs and 64 GiB", from what its runners said
 * last; "no runners yet" before one connected.
 */
export function poolShapeWords(d: G.PoolDemand): string {
  const all = enrolled(d.runners);
  const runners = count(all, "runner", "runners");
  if (!d.shape) return all ? runners : "no runners yet";
  const each = `${count(d.shape.cpus, "CPU", "CPUs")} and ${memory(d.shape.memory_mb)}`;
  return all ? `${runners} with ${each}` : `no runners now, ${each} each`;
}

/**
 * Of `pools`, the one a project with `labels` waits for and counts towards first: the one whose
 * labels and its runners' include them all with the fewest others, then the oldest
 * (`fctl_domain::pools::first_with`). Undefined for no labels, or when no pool has them. A member's
 * own pool runs only their sessions (`do_fleet.rs`'s `pool_for`), so it is no project's.
 */
export function poolFor(labels: readonly string[], pools: readonly G.PoolDemand[]): G.PoolDemand | undefined {
  if (labels.length === 0) return undefined;
  let best: { p: G.PoolDemand; n: number; i: number } | undefined;
  pools.forEach((p, i) => {
    if (p.pool.owner) return;
    const have = [
      ...(p.pool.labels ?? []),
      ...(p.shape?.labels ?? []).filter((l) => !(p.pool.labels ?? []).includes(l)),
    ];
    if (!labels.every((l) => have.includes(l))) return;
    if (!best || have.length < best.n) best = { p, n: have.length, i };
  });
  return best?.p;
}

/** A pool a project may choose: by name, for its labels. */
export interface PoolChoice {
  id: string;
  name: string;
  labels: string[];
  /** "4 runners with 16 CPUs and 64 GiB". */
  facts: string;
}

/**
 * The pools a project of workspace `wid` may choose, from what the viewer reads: what each needs
 * now, for those who manage the pools; and the pools of the workspace's runners, for the rest, which
 * carry their pool's labels and their hosts' (the labels all of a pool's runners share stand for its
 * own). Not a member's own pool, whose runners run only their sessions.
 */
export function poolChoices(
  pools: readonly G.PoolDemand[] | undefined,
  runners: readonly G.Runner[],
  wid: string,
): PoolChoice[] {
  const out: PoolChoice[] = (pools ?? [])
    .filter((d) => !d.pool.owner && (!d.pool.closed || enrolled(d.runners) > 0))
    .map((d) => ({
      id: d.pool.id,
      name: d.pool.name,
      labels: d.pool.labels ?? [],
      facts: poolShapeWords(d),
    }));
  const seen = new Set(out.map((c) => c.id));
  const by = new Map<string, { name: string; runners: G.Runner[] }>();
  for (const r of runners) {
    if (!r.pool || seen.has(r.pool.id) || r.personal?.includes(wid)) continue;
    const g = by.get(r.pool.id) ?? { name: r.pool.name, runners: [] };
    g.runners.push(r);
    by.set(r.pool.id, g);
  }
  for (const [id, g] of by) {
    const [first, ...rest] = g.runners;
    const labels = (first?.labels ?? []).filter((l) => rest.every((r) => r.labels.includes(l)));
    out.push({ id, name: g.name, labels, facts: count(g.runners.length, "runner", "runners") });
  }
  return out;
}

/** Whether two sets of labels are the same, whatever their order. */
export const sameLabels = (a: readonly string[], b: readonly string[]) =>
  a.length === b.length && a.every((l) => b.includes(l));

/** A wait a project's sessions may give up after, for any runner: none (however long), or minutes. */
export interface Fallback {
  value: string;
  label: string;
  description: string;
  minutes: number | null;
}

const WAIT: Fallback = {
  value: "wait",
  label: "Wait for one",
  description: "However long that takes: for work that needs those runners.",
  minutes: null,
};

/** The waits to choose from. */
export const FALLBACKS: readonly Fallback[] = [
  WAIT,
  {
    value: "5",
    label: "After 5 minutes, any runner",
    description: "The labels are a preference.",
    minutes: 5,
  },
  {
    value: "15",
    label: "After 15 minutes, any runner",
    description: "Time for a pool to start a machine.",
    minutes: 15,
  },
  { value: "60", label: "After an hour, any runner", description: "Through a slow scale-up.", minutes: 60 },
  { value: "1440", label: "After a day, any runner", description: "Overnight.", minutes: 1440 },
];

/** The wait a project's setting stands for: one to choose from, with its own where it set another. */
export function fallbackOf(minutes: number | null | undefined): Fallback {
  if (minutes === null || minutes === undefined) return WAIT;
  return (
    FALLBACKS.find((f) => f.minutes === minutes) ?? {
      value: String(minutes),
      label: `After ${minutes} minutes, any runner`,
      description: "As the project set it.",
      minutes,
    }
  );
}

/** A project's runners in a few words, for its settings' list: "Any runner", "Runners with aws, large". */
export function runnersGist(s: Pick<G.ProjectSettings, "runner_labels">): string {
  const labels = s.runner_labels ?? [];
  return labels.length ? `Runners with ${labels.join(", ")}` : "Any runner";
}

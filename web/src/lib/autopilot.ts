// Autopilots (experimental): an agent that runs a project, or its whole workspace, on its own
// (docs/architecture.md#autopilots). What the app says of one: where it stands, what it may do,
// how often it wakes, and the directions to start from.

import type * as G from "@/gen";
import { ago, clock, day, num } from "./format";

/** How long it goes without waking, at most, as people choose it (the control plane's CHECK_INS). */
export const CHECK_INS: { minutes: number; label: string }[] = [
  { minutes: 15, label: "Every 15 minutes" },
  { minutes: 30, label: "Every 30 minutes" },
  { minutes: 60, label: "Every hour" },
  { minutes: 120, label: "Every 2 hours" },
  { minutes: 240, label: "Every 4 hours" },
  { minutes: 480, label: "Every 8 hours" },
  { minutes: 720, label: "Every 12 hours" },
  { minutes: 1440, label: "Once a day" },
];
export const DEFAULT_CHECK_IN = 60;
export const DEFAULT_CREW = 3;
export const CREW_MAX = 8;

/** "every hour", "every 30 minutes", "once a day". */
export function checkInWords(minutes: number): string {
  const c = CHECK_INS.find((x) => x.minutes === minutes);
  if (c) return c.label.charAt(0).toLowerCase() + c.label.slice(1);
  return minutes % 60 === 0 ? `every ${minutes / 60} hours` : `every ${minutes} minutes`;
}

/**
 * Where a fresh session of its own takes over from the one it works in, as people choose it (the
 * control plane's HANDOFF_CHOICES), besides only after 12 hours and a size of their own.
 */
export const HANDOFF_CHOICES = [150_000, 250_000, 400_000] as const;

/** What the handoff's list shows: "" for only after 12 hours, a size offered, or "other" for one typed. */
export type HandoffChoice = "" | `${(typeof HANDOFF_CHOICES)[number]}` | "other";

/** The choice that shows `tokens`. */
export function handoffChoice(tokens?: number | null): HandoffChoice {
  if (tokens == null) return "";
  return HANDOFF_CHOICES.find((c) => c === tokens) != null ? (`${tokens}` as HandoffChoice) : "other";
}

/** When a fresh session takes over: "at 250K tokens of context, or after 12 hours". */
export function handoffWords(tokens?: number | null): string {
  return tokens == null ? "after 12 hours" : `at ${num(tokens)} tokens of context, or after 12 hours`;
}

/** What each power lets it do, in the order people see them. */
export const POWERS: { power: G.AutopilotPower; label: string; line: string; workspace?: boolean }[] = [
  {
    power: "merge",
    label: "Merge pull requests",
    line: "Its sessions merge their own once checks pass and the work is done, so the next builds on it. Repositories' rules still apply.",
  },
  {
    power: "settings",
    label: "Change settings",
    line: "The agents' instructions, setup script, image, model, follow-ups and review agents.",
  },
  {
    power: "secrets",
    label: "Manage secrets",
    line: "Sets secrets it makes itself, such as a signing key. It never reads their values, nor replaces those people set.",
  },
  {
    power: "repos",
    label: "Add repositories",
    line: "Among those the workspace's GitHub accounts reach, to read or to work in.",
  },
  {
    power: "schedules",
    label: "Set up schedules",
    line: "For work that comes back: dependency updates, triage, a nightly check.",
  },
  {
    power: "projects",
    label: "Create projects",
    line: "In the workspace, with new repositories in an organization it works in whole.",
    workspace: true,
  },
];

export const powerLabel = (p: G.AutopilotPower) =>
  POWERS.find((x) => x.power === p)?.label.toLowerCase() ?? p;

/** The powers a new autopilot has unless people say otherwise: all its scope has. */
export const defaultPowers = (scope: G.AutopilotScope): G.AutopilotPower[] =>
  POWERS.filter((p) => scope === "workspace" || !p.workspace).map((p) => p.power);

/** Its own session is at work. */
export function atWork(a: Pick<G.Autopilot, "state" | "session">): boolean {
  const s = a.session?.status;
  return (
    a.state === "on" &&
    (s === "queued" || s === "provisioning" || s === "preparing" || s === "running" || s === "stopping")
  );
}

/** How far its plan's checklist is ticked off ("9 of 15"); none for a plan without one. */
export function planDone(plan?: string): { checked: number; of: number } | undefined {
  const items = plan?.match(/^\s*[-*+] \[( |x|X)\]/gm) ?? [];
  if (!items.length) return;
  return { checked: items.filter((i) => /\[(x|X)\]/.test(i)).length, of: items.length };
}

/** What it asks that is still open. */
export const openAsks = (a: Pick<G.Autopilot, "asks">) => a.asks.filter((x) => x.status === "open");

/** Where it stands, as its header says it: a tone, a word, and a sentence of what follows. */
export interface Phase {
  tone: "accent" | "blue" | "pink" | "ink";
  word: string;
  line: string;
}

/**
 * Where `a` stands. `here` is the session the reader is in: its own session, said as "this session"
 * when it is the one the autopilot works in now.
 */
export function phase(a: G.Autopilot, now = Date.now(), here?: G.SessionId): Phase {
  const asks = openAsks(a).length;
  const waiting =
    asks > 0
      ? ` It asks you ${asks === 1 ? "one thing" : `${asks} things`}, and goes on with what it can meanwhile.`
      : "";
  if (a.state === "off") return { tone: "ink", word: "Off", line: "The project is off autopilot." };
  if (a.state === "paused") {
    const who = a.paused_by ? `${a.paused_by.name} paused it` : "It paused";
    const why = a.paused_reason ? `: ${a.paused_reason}` : ".";
    return {
      tone: "pink",
      word: "Paused",
      line: `${who}${a.paused_at ? ` ${ago(a.paused_at, now)}` : ""}${why} It wakes again once someone resumes it.`,
    };
  }
  if (atWork(a)) {
    const starting =
      a.session?.status === "queued" ||
      a.session?.status === "provisioning" ||
      a.session?.status === "preparing";
    const inIt = !!here && a.session?.id === here;
    const where = inIt ? "this session" : `its session #${a.session?.number}`;
    const fresh = a.fresh_asked ? " A fresh session takes over once this turn ends." : "";
    return {
      tone: "accent",
      word: starting ? "Waking" : "At work",
      line: starting
        ? `${inIt ? "This session" : `Its session #${a.session?.number}`} is starting.${fresh}${waiting}`
        : `It works in ${where} now.${fresh}${waiting}`,
    };
  }
  if (asks > 0)
    return {
      tone: "blue",
      word: "Waiting for you",
      line: `It asks you ${asks === 1 ? "one thing" : `${asks} things`}, and goes on with what it can meanwhile.${
        a.wake_at ? ` ${wakeWords(a, now)}` : ""
      }`,
    };
  return {
    tone: "blue",
    word: "On",
    line: a.wake_at ? wakeWords(a, now) : "It wakes when something needs it.",
  };
}

/**
 * A project's autopilot as the sidebar shows it: the project list's summary (`G.ProjectAutopilot`),
 * or what the sidebar kept of one, which has its state and session alone.
 */
export type AutopilotGlance = Pick<G.ProjectAutopilot, "state"> &
  Partial<Pick<G.ProjectAutopilot, "working" | "asks" | "wake_at">> & {
    session?: Pick<G.AutopilotSession, "id" | "number"> & Partial<Pick<G.AutopilotSession, "status">>;
  };

/** Where an autopilot stands, as its line in the sidebar says it. */
export interface Glance {
  /** Its own session works, or is starting: the comet runs round its mark. */
  live: boolean;
  /** Its own session is starting. */
  waking?: boolean;
  paused: boolean;
  /** When it wakes next, as a time ("14:20") or a day ("Nov 12"), while it rests. */
  wake?: string;
  /** What it asks people now. */
  asks: number;
  /** A sentence of where it stands, for the line's tooltip. */
  words: string;
}

/** Where `a` stands at a glance; none while it is off, or the project has none. */
export function glance(a: AutopilotGlance | undefined, now = Date.now()): Glance | undefined {
  if (!a?.state || a.state === "off") return;
  const asks = a.asks ?? 0;
  const asking = asks > 0 ? ` It asks you ${asks === 1 ? "one thing" : `${asks} things`}.` : "";
  if (a.state === "paused")
    return {
      live: false,
      paused: true,
      asks,
      words: `Paused: it wakes again once someone resumes it.${asking}`,
    };
  const n = a.session ? ` #${a.session.number}` : "";
  if (a.working) {
    const s = a.session?.status;
    const starting = s === "queued" || s === "provisioning" || s === "preparing";
    return {
      live: true,
      waking: starting,
      paused: false,
      asks,
      words: starting
        ? `Waking: its session${n} is starting.${asking}`
        : `At work in its session${n}.${asking}`,
    };
  }
  const at = a.wake_at;
  if (at == null || at - now < 60_000)
    return {
      live: false,
      paused: false,
      asks,
      words: `Resting: it wakes ${at == null ? "when something needs it" : "now"}.${asking}`,
    };
  const today = at - now < 86_400_000;
  return {
    live: false,
    paused: false,
    asks,
    wake: today ? clock(at) : day(at),
    words: `Resting: it wakes ${today ? `at ${clock(at)}` : `on ${day(at)} at ${clock(at)}`}, unless something needs it sooner.${asking}`,
  };
}

/** "It checks in at 14:20, unless something needs it sooner." */
export function wakeWords(a: Pick<G.Autopilot, "wake_at" | "wake_reason">, now = Date.now()): string {
  if (!a.wake_at) return "";
  const at = a.wake_at;
  const when =
    at - now < 60_000 ? "now" : at - now < 86_400_000 ? `at ${clock(at)}` : `on ${day(at)} at ${clock(at)}`;
  switch (a.wake_reason) {
    case "asked":
      return `It wakes ${when}, as it asked, unless something needs it sooner.`;
    case "check_in":
    case undefined:
      return `It checks in ${when}, unless something needs it sooner.`;
    case "retry":
      return `Its last session failed: it tries again ${when}.`;
    case "fresh":
      return `A fresh session takes over ${when}.`;
    default:
      return `It wakes ${when} for what's new.`;
  }
}

/** What woke it, as a line, as its log says it: "Woke for its check-in", "Woke as it was resumed". */
export function wokeLine(w: G.AutopilotWake): string {
  const [only, more] = w.reasons;
  if (!more && only === "started") return "Woke as it was put on";
  if (!more && only === "resumed") return "Woke as it was resumed";
  if (!more && only === "woken") return "Woke as someone woke it";
  if (!more && only === "moved") return "Woke as it was moved to another Claude subscription";
  if (!more && only === "fresh") return "Woke as someone asked for a fresh session";
  return `Woke for ${wakeText(w)}`;
}

/** What woke it, in a few words: "its check-in", "a note and session #41". */
export function wakeText(w: G.AutopilotWake): string {
  const parts: string[] = [];
  for (const r of w.reasons) {
    switch (r) {
      case "started":
        parts.push("being put on");
        break;
      case "resumed":
        parts.push("being resumed");
        break;
      case "check_in":
        parts.push("its check-in");
        break;
      case "asked":
        parts.push("the time it asked for");
        break;
      case "woken":
        parts.push("someone waking it");
        break;
      case "steered":
        parts.push(
          w.notes
            ? w.notes === 1
              ? "a new note"
              : `${w.notes} new notes`
            : w.direction
              ? "a new direction"
              : "people steering it",
        );
        break;
      case "answered":
        parts.push((w.answers ?? 0) > 1 ? `${w.answers} answers` : "an answer");
        break;
      case "crew": {
        const crew = w.crew ?? [];
        parts.push(
          crew.length === 1
            ? `session #${crew[0]?.number}`
            : crew.length
              ? `${crew.length} of its sessions`
              : "its sessions",
        );
        break;
      }
      case "retry":
        parts.push("a retry");
        break;
      case "moved":
        parts.push("being moved to another Claude subscription");
        break;
      case "fresh":
        parts.push("someone asking for a fresh session");
        break;
    }
  }
  if (!parts.length) return "its check-in";
  if (parts.length === 1) return parts[0] ?? "";
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

/** A direction to start from: what it should do, written as a good direction is. */
export interface DirectionIdea {
  key: string;
  name: string;
  line: string;
  direction: string;
}

export const DIRECTIONS: DirectionIdea[] = [
  {
    key: "build",
    name: "Build a product from a brief",
    line: "From a description to a product people use, a feature at a time.",
    direction: `Build [what, and for whom], from this brief:

- [The problem it solves, and the people it is for]
- [The features that matter first]
- [What it must not do]

Plan the work as small, shippable steps. Each step is a session of its own that opens a pull request with tests, and ends with the app working. Keep the UI clean and consistent, and the code easy to change. Ask me before choosing anything that's hard to undo: a framework, a data model, a paid service.`,
  },
  {
    key: "healthy",
    name: "Keep it healthy",
    line: "Dependencies current, CI green, flaky tests fixed, docs true.",
    direction: `Keep the project healthy, so that people can build on it without surprises:

- Dependencies current: patch and minor updates each week; major ones planned, with what they take.
- CI green and fast: fix failing and flaky tests at their cause, not by retrying them.
- Docs true to the code, and the README enough to get started.

Prefer small pull requests. Set up schedules for what comes back each week.`,
  },
  {
    key: "issues",
    name: "Work through the issues",
    line: "Triage what comes in, then fix what matters most first.",
    direction: `Work through the project's open issues, most valuable first:

- Triage new ones: labels, what's missing to act on them, duplicates.
- Fix bugs before features, and those many people hit before the rest.
- Each fix in a pull request that says which issue it closes, with a test that would have caught it.

Ask me when an issue needs a product decision.`,
  },
  {
    key: "polish",
    name: "Polish the UI and UX",
    line: "Find what feels rough in the app and make it feel great.",
    direction: `Make the app a pleasure to use:

- Go through every screen, at a phone's width and a computer's, in light and dark themes, and list what feels rough: spacing, alignment, states without feedback, empty states that say nothing, slow interactions.
- Fix the roughest first, a screen at a time, with screenshots before and after in each pull request.
- Keep to the app's design system; where it lacks something, propose it before building it.

When two directions are equally good, show me both with screenshots and ask me to choose.`,
  },
];

/** Whether a direction still holds an idea's placeholders, which say what to fill in. */
export const hasPlaceholders = (direction: string) => /\[[^\]\n]{3,}\]/.test(direction);

/**
 * A task an autopilot gave one of its sessions: the task, then fctl's note of who started it
 * (`crate::autopilots::crew_prompt`), which the page shows apart.
 */
export function splitCrewNote(text: string): { task: string; note?: string } {
  const m = /\n\n---\n(?=The autopilot of [^\n]* started this session: )/.exec(text);
  if (!m) return { task: text };
  return { task: text.slice(0, m.index), note: text.slice(m.index + m[0].length) };
}

/**
 * What a message fctl woke it with says, in a sentence: a start, a fresh session taking over from the
 * last (`fctl_domain::autopilot::takes_over_from`, after its briefing), or what woke it.
 */
export function wakeLead(text: string, wake?: G.AutopilotWake): string {
  const { briefing, news } = splitBriefing(text);
  // A start or a resume alone says so in a sentence of its own.
  const lone = wake?.reasons.length === 1 ? wake.reasons[0] : undefined;
  if (briefing)
    return news.startsWith("You take over from your last session")
      ? "A fresh session takes over: it reads its brief, then goes on from its plan and log."
      : "It was put on autopilot: it reads its brief, then plans the work.";
  if (lone === "resumed") return "People resumed it: it reads what's new, then goes on.";
  if (lone === "started") return "It was put on autopilot again: it reads what's new, then goes on.";
  if (lone === "woken") return "Someone woke it: it reads what's new, then goes on.";
  return `For ${wake ? wakeText(wake) : "its check-in"}.`;
}

/**
 * The first message of an autopilot's own session: its briefing (`fctl_domain::autopilot::briefing`),
 * the direction in it, then what woke it. Any later message of fctl's is what woke it alone.
 */
export function splitBriefing(text: string): { briefing?: string; direction?: string; news: string } {
  if (!text.startsWith("You are the autopilot of ")) return { news: text };
  const cut = text.indexOf("\n---\n\n");
  const briefing = cut === -1 ? text : text.slice(0, cut);
  const news = cut === -1 ? "" : text.slice(cut + 6);
  const d = /## Where to take it\n\n([\s\S]*?)\n\n## How you work/.exec(briefing);
  return { briefing, direction: d?.[1]?.trim(), news };
}

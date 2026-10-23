// Ticket trackers: what a project's choice means in words, how a ticket's state shows, and what a
// Linear sign-in came back with.
import type * as G from "@/gen";

/** The trackers a project chooses from, in the order the Tickets tab shows them. */
export const TRACKERS: { value: G.Tracker; label: string; summary: string }[] = [
  {
    value: "none",
    label: "None",
    summary: "Agents read and write no tickets, GitHub's issues included.",
  },
  {
    value: "github",
    label: "GitHub Issues",
    summary: "Agents read, open and comment on issues in the project's repositories.",
  },
  {
    value: "linear",
    label: "Linear",
    summary: "Delegate issues to fctl in Linear: each starts a session, which reports back there.",
  },
];

/** The project's tracker, as its settings say (GitHub Issues where they say none: as before trackers). */
export const trackerOf = (s: G.ProjectSettings): G.Tracker => s.tickets?.tracker ?? "github";

/** A ticket's state as a color token: done in accent, canceled muted, under way in blue. */
export function stateTone(kind?: G.StateKind): string {
  switch (kind) {
    case "started":
      return "text-blue";
    case "completed":
      return "text-accent";
    case "canceled":
      return "text-ink-4";
    default:
      return "text-ink-3";
  }
}

/** Where a Linear sign-in left the browser (`?linear=`), as api::linear says. */
export const LINEAR_OUTCOMES = [
  "connected",
  "linked",
  "denied",
  "expired",
  "failed",
  "other_user",
  "not_app",
] as const;
export type LinearOutcome = (typeof LINEAR_OUTCOMES)[number];

/** What to tell someone back from a Linear sign-in, and how it went. */
export function linearOutcome(o: LinearOutcome): { tone: "ok" | "warn" | "error"; text: string } {
  switch (o) {
    case "connected":
      return {
        tone: "ok",
        text: "Linear is connected: fctl's app is installed in your Linear workspace. Choose the team this project works on.",
      };
    case "linked":
      return {
        tone: "ok",
        text: "Your Linear account is linked: issues you delegate to fctl there start sessions as you.",
      };
    case "denied":
      return { tone: "warn", text: "Linear's sign-in was cancelled, so nothing changed." };
    case "expired":
      return { tone: "warn", text: "That sign-in took longer than ten minutes. Start it again." };
    case "other_user":
      return {
        tone: "error",
        text: "That link was for someone else's sign-in. Start it again from here.",
      };
    case "not_app":
      return {
        tone: "error",
        text: "Linear signed you in instead of installing fctl's app. Installing it takes an admin of the Linear workspace: try again as one.",
      };
    case "failed":
      return { tone: "error", text: "Linear didn't finish the sign-in. Try again in a minute." };
  }
}

/** A session's ticket by its key, with its tracker's name for screen readers and tooltips. */
export const ticketLabel = (t: G.SessionTicket) =>
  `${t.key}${t.title ? `: ${t.title}` : ""}, on ${t.tracker === "linear" ? "Linear" : "GitHub"}`;

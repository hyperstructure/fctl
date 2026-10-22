// Screenshots agents publish for pull requests (`fctl publish`): links
// anyone who has one can open, until they expire (fctl-domain `published`).

import type * as G from "@/gen";

/** Days a project's links work unless it chooses otherwise, and the choices offered. */
export const DEFAULT_DAYS = 30;
export const DAY_CHOICES = [1, 3, 7, 14, 30, 60];

/** "24 hours", "7 days". */
export const lasts = (days: number) => (days === 1 ? "24 hours" : `${days} days`);

/** The token in a published screenshot's link: all it takes to open, or delete, it. */
export function linkToken(url: string): string | undefined {
  return /\/api\/v1\/published\/([0-9A-Za-z]{43})(?:[/?#]|$)/.exec(url)?.[1];
}

/**
 * Whether a published screenshot's link is this instance's (`origin`, the app's): what runs in a sandbox can pose as
 * the agent, and a notice the control plane kept before it checked them could name any image or download. `anyOrigin`
 * in development, where the control plane and the app answer on different ports.
 */
export function isPublishedLink(url: string, origin: string, anyOrigin = false): boolean {
  let u: URL;
  try {
    u = new URL(url);
  } catch {
    return false;
  }
  const site = u.origin === origin || (anyOrigin && (u.protocol === "https:" || u.protocol === "http:"));
  return site && u.pathname.startsWith("/api/v1/published/") && linkToken(url) !== undefined;
}

/** Markdown that shows the screenshot, as `fctl publish` prints it. */
export function markdown(f: G.PublishedFile): string {
  return `![${f.name.replace(/[[\]\\\r\n]/g, "").trim()}](${f.url})`;
}

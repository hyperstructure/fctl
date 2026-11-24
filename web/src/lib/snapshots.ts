// What the web app says of sessions' snapshots (docs/architecture.md#session-snapshots): a
// session's conversation and the work its agent hadn't pushed, saved as its sandbox stops, so
// that it resumes on any runner as it was. The words follow the CLI's (`fctl show`,
// `fctl workspaces snapshots`).

import type * as G from "@/gen";
import { bytes } from "./format";

/** The days a workspace may keep each snapshot, which its admins choose from. */
export const SNAPSHOT_DAYS = [1, 3, 7, 14, 30, 60, 90, 180, 365];

/** The days a workspace keeps each snapshot until its admins choose (`DEFAULT_DAYS`). */
export const DEFAULT_SNAPSHOT_DAYS = 30;

/** "1 day", "30 days". */
export const daysWords = (n: number) => (n === 1 ? "1 day" : `${n} days`);

/** The days to choose from, with the workspace's own where it chose another. */
export function snapshotDays(
  current: number | undefined,
): { value: string; label: string; description?: string }[] {
  const days = [...SNAPSHOT_DAYS];
  if (current !== undefined && !days.includes(current)) days.push(current);
  return days
    .sort((a, b) => a - b)
    .map((d) => ({
      value: String(d),
      label: daysWords(d),
      description: d === DEFAULT_SNAPSHOT_DAYS ? "The default" : undefined,
    }));
}

/**
 * What a snapshot holds, as `fctl show` says it: "its conversation and uncommitted work",
 * "its conversation, without its images".
 */
export function holds(s: Pick<G.SessionSnapshot, "conversation" | "changes" | "left_out">): string {
  const what =
    s.conversation && s.changes
      ? "its conversation and uncommitted work"
      : s.conversation
        ? "its conversation"
        : s.changes
          ? "its uncommitted work"
          : "nothing";
  const left = s.left_out?.length ? `, without its ${s.left_out.join(" or ")}` : "";
  return what + left;
}

/** The bucket a location in one is in: `s3://acme-fctl` of `s3://acme-fctl/sessions/ses_1/snp_2`. */
export function bucketOf(location: string): string | undefined {
  const m = /^s3:\/\/([^/]+)\//.exec(location);
  return m ? `s3://${m[1]}` : undefined;
}

/** "1 snapshot", "14 snapshots". */
const snapshots = (n: number) => (n === 1 ? "1 snapshot" : `${n} snapshots`);

/**
 * Where a workspace's snapshots are now and what they take, in a sentence's words: "14 snapshots
 * with fctl (210 MB) and 3 in s3://acme-fctl (1.2 GB)"; "none yet" while there are none.
 */
export function usageWords(s: Pick<G.WorkspaceSnapshots, "kept" | "buckets">): string {
  const parts: string[] = [];
  if (s.kept.count > 0) parts.push(`${snapshots(s.kept.count)} with fctl (${bytes(s.kept.bytes)})`);
  for (const b of s.buckets) {
    const n = parts.length === 0 ? snapshots(b.count) : String(b.count);
    parts.push(`${n} in ${b.bucket} (${bytes(b.bytes)})`);
  }
  if (parts.length === 0) return "none yet";
  if (parts.length === 1) return parts[0] ?? "";
  return `${parts.slice(0, -1).join(", ")} and ${parts[parts.length - 1]}`;
}

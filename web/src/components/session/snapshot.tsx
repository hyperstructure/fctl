// A session's last snapshot in its details rail (docs/architecture.md#session-snapshots): what it
// holds, its size, where and when it was saved, and until when a resume restores it, as `fctl show`
// says it.

import type * as G from "@/gen";
import { ago, bytes, day, stamp } from "@/lib/format";
import { bucketOf, holds } from "@/lib/snapshots";

export function SnapshotRow({ snapshot: s }: { snapshot: G.SessionSnapshot }) {
  const bucket = s.location ? bucketOf(s.location) : undefined;
  const what = holds(s);
  // Names stay whole: "aws-9" doesn't break at its hyphen.
  const name = (n: string) => <span className="whitespace-nowrap">{n}</span>;
  return (
    <div className="py-0.5 text-[12.5px]">
      <div className="flex items-baseline justify-between gap-3">
        <span className="text-ink-4">Snapshot</span>
        <span className="min-w-0 truncate text-right text-ink-2" title={stamp(s.saved_at)}>
          {bytes(s.size)} · {ago(s.saved_at)}
        </span>
      </div>
      <p className="mt-0.5 text-[11.5px] leading-relaxed text-pretty text-ink-4">
        {what.charAt(0).toUpperCase() + what.slice(1)}, saved
        {s.saved_on && <> on {name(s.saved_on)}</>}, {bucket ? <>in {name(bucket)}</> : "kept by fctl"}.
        {s.expires_at
          ? ` A resume on another runner restores it until ${day(s.expires_at)}.`
          : " A resume on another runner restores it."}
      </p>
    </div>
  );
}

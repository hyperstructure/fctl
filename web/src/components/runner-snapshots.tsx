// Under a runner in the Runners tab and the admin's list: the bucket of its
// organization's own that its host keeps sessions' snapshots in
// (`[snapshots]` in its host.toml), and, in amber, why the last snapshot it
// put there or got from there failed, until one goes through again.

import type * as G from "@/gen";
import { ago, stamp } from "@/lib/format";

/** Nothing where its host keeps snapshots in the control plane. */
export function SnapshotStoreLine({ runner: r }: { runner: G.Runner }) {
  const store = r.snapshot_store;
  if (!store) return null;
  if (store.error) {
    return (
      <div className="mt-0.5 text-[12px] text-amber [overflow-wrap:anywhere]">
        Keeps sessions' snapshots in <span className="font-mono text-[0.92em]">{store.url}</span>, but the
        last one failed
        {store.failed_at ? <span title={stamp(store.failed_at)}> {ago(store.failed_at)}</span> : null}:{" "}
        {store.error.replace(/\.$/, "")}. Until the bucket works again, sessions may resume without their
        latest conversation and work.
      </div>
    );
  }
  return (
    <div className="mt-0.5 truncate text-[11.5px] text-ink-4">
      Keeps sessions' snapshots in <span className="font-mono text-[0.95em]">{store.url}</span>
    </div>
  );
}

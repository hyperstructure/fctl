// An audit log, newest first, a page at a time: the instance's or a
// workspace's. Older pages join the first in one query, so refetching it
// doesn't lose them.

import { useInfiniteQuery } from "@tanstack/react-query";
import { Button } from "@/components/ui";
import type * as G from "@/gen";
import { ago } from "@/lib/format";

export function AuditLog({
  queryKey,
  load,
  ip = false,
}: {
  queryKey: readonly unknown[];
  load: (before?: number) => Promise<G.AuditList>;
  /** Show where each action came from. */
  ip?: boolean;
}) {
  const log = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam }) => load(pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next_before ?? undefined,
  });
  const entries = log.data?.pages.flatMap((p) => p.entries) ?? [];
  return (
    <div>
      <ul className="border-t border-line">
        {entries.map((e) => {
          const actor = e.actor?.name ?? "system";
          return (
            <li key={e.id} className="border-b border-line py-2.5 text-[13px]">
              <div className="hidden items-baseline gap-3 sm:flex">
                <span className="w-24 shrink-0 text-[12px] text-ink-4">{ago(e.ts)}</span>
                <span className="w-40 shrink-0 truncate text-ink-2">{actor}</span>
                <span className="shrink-0 font-mono text-[12.5px] text-ink">{e.action}</span>
                <span className="min-w-0 flex-1 truncate font-mono text-[12px] text-ink-4">{e.target}</span>
                {ip && e.ip && <span className="shrink-0 font-mono text-[11.5px] text-ink-4">{e.ip}</span>}
              </div>
              {/* On a phone: what happened and when, then who and to what. */}
              <div className="sm:hidden">
                <div className="flex items-baseline justify-between gap-3">
                  <span className="min-w-0 truncate font-mono text-[12.5px] text-ink">{e.action}</span>
                  <span className="shrink-0 text-[12px] text-ink-4">{ago(e.ts)}</span>
                </div>
                <div className="mt-0.5 truncate text-[12px] text-ink-4">
                  <span className="text-ink-2">{actor}</span>
                  {e.target && <span className="font-mono"> · {e.target}</span>}
                  {ip && e.ip && <span className="font-mono"> · {e.ip}</span>}
                </div>
              </div>
            </li>
          );
        })}
        {log.data && entries.length === 0 && <li className="py-6 text-[13px] text-ink-4">Nothing yet.</li>}
      </ul>
      {log.isError && <p className="mt-4 text-[13px] text-danger">{log.error.message}</p>}
      {log.hasNextPage && (
        <div className="mt-4">
          <Button
            size="sm"
            variant="ghost"
            loading={log.isFetchingNextPage}
            onClick={() => void log.fetchNextPage()}
          >
            Older
          </Button>
        </div>
      )}
    </div>
  );
}

// An audit log, newest first, a page at a time: the instance's or a
// workspace's. Older pages join the first in one query, so refetching it
// doesn't lose them. Each entry reads as a sentence: who did what, to what
// (lib/audit.ts), with the action's key and its target where the pointer rests.

import { useInfiniteQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { useMemo } from "react";
import { Avatar, Button, Logo } from "@/components/ui";
import type * as G from "@/gen";
import { type AuditNames, type AuditPart, auditActor, describeAudit, sessionRefs } from "@/lib/audit";
import { ago, stamp } from "@/lib/format";

export function AuditLog({
  queryKey,
  load,
  ip = false,
  names,
}: {
  queryKey: readonly unknown[];
  load: (before?: number) => Promise<G.AuditList>;
  /** Show where each action came from. */
  ip?: boolean;
  /** What the page knows, to name the people and projects entries point at. */
  names?: AuditNames;
}) {
  const log = useInfiniteQuery({
    queryKey,
    queryFn: ({ pageParam }) => load(pageParam),
    initialPageParam: undefined as number | undefined,
    getNextPageParam: (last) => last.next_before ?? undefined,
  });
  const entries = useMemo(() => log.data?.pages.flatMap((p) => p.entries) ?? [], [log.data]);
  // A session an entry stops or resumes is named as the entry that started it named it.
  const refs = useMemo(() => sessionRefs(entries), [entries]);
  const all: AuditNames = { ...names, session: (id) => refs.get(id) ?? names?.session?.(id) };
  return (
    <div>
      <ul className="border-t border-line">
        {entries.map((e) => (
          <Entry key={e.id} e={e} names={all} ip={ip} />
        ))}
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

function Entry({ e, names, ip }: { e: G.AuditEntry; names: AuditNames; ip: boolean }) {
  // The log's own actor has no picture: the page's people do.
  const actor = e.actor && (names.person?.(e.actor.id) ?? e.actor);
  const parts = describeAudit(e, names);
  // "Noor Haddad's autopilot started …" follows the name without a space.
  const owns = typeof parts[0] === "string" && parts[0].startsWith("'");
  return (
    <li className="flex items-start gap-3 border-b border-line py-2.5 text-[13.5px] leading-5">
      <span className="flex h-5 shrink-0 items-center">
        {actor ? <Avatar user={actor} size={20} /> : <Logo withWord={false} className="[&_svg]:size-5" />}
      </span>
      <div className="min-w-0 flex-1">
        <p
          className="text-ink-3 [overflow-wrap:anywhere]"
          title={`${e.action}${e.target ? ` · ${e.target}` : ""}`}
        >
          <span className="font-medium text-ink">{auditActor(e)}</span>
          {owns ? "" : " "}
          {parts.map((part, i) => (
            // The parts of one sentence, which never reorder.
            <Part key={i} part={part} />
          ))}
        </p>
        {/* Where it came from, under the sentence on a phone, where an IPv6 address beside it
            would leave the sentence no room. */}
        {ip && e.ip && <p className="mt-0.5 truncate font-mono text-[11.5px] text-ink-4 sm:hidden">{e.ip}</p>}
      </div>
      <span className="flex shrink-0 items-baseline gap-3">
        <span className="text-[12px] text-ink-4 tabular-nums" title={stamp(e.ts)}>
          {ago(e.ts)}
        </span>
        {ip && e.ip && (
          <span
            className="max-w-[18ch] truncate font-mono text-[11.5px] text-ink-4 max-sm:hidden"
            title={e.ip}
          >
            {e.ip}
          </span>
        )}
      </span>
    </li>
  );
}

function Part({ part }: { part: AuditPart }) {
  if (typeof part === "string") return <>{part}</>;
  const look = part.code ? "font-mono text-[12.5px] text-ink-2" : "text-ink-2";
  if (!part.to) return <span className={look}>{part.text}</span>;
  const link = `${look} text-ink underline-offset-2 transition-colors hover:underline`;
  // A session's reference stays whole: "driver-app#77" broke at its hyphen on a phone.
  return "session" in part.to ? (
    <Link to="/s/$id" params={{ id: part.to.session }} className={`${link} whitespace-nowrap`}>
      {part.text}
    </Link>
  ) : (
    <Link to="/p/$slug" params={{ slug: part.to.project }} search={{ tab: undefined }} className={link}>
      {part.text}
    </Link>
  );
}

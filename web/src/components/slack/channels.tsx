// Choosing a public channel of a Slack workspace a workspace works in, for its admins and projects'
// maintainers: a dialog that
// searches Slack's channels by name as they type, says which fctl is in already, and which are
// taken for what's being chosen.

import { useQuery } from "@tanstack/react-query";
import { Hash, Search, Users } from "lucide-react";
import { type KeyboardEvent, useEffect, useId, useRef, useState } from "react";
import { Banner, Dialog, Input, Skeleton } from "@/components/ui";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys } from "@/lib/queries";
import { bareChannel } from "@/lib/slack";

/** How long typing rests before Slack is asked. */
const SETTLE_MS = 250;

export function ChannelPicker({
  workspace,
  team,
  project,
  open,
  onClose,
  onPick,
  title,
  description,
  note,
  taken,
}: {
  workspace: string;
  team: string;
  /** The project whose maintainers choose, who aren't the workspace's admins. */
  project?: string;
  open: boolean;
  onClose: () => void;
  onPick: (c: G.SlackChannelChoice) => void;
  title: string;
  description: string;
  /** Under the list: what it leaves out, and the way there. */
  note?: string;
  /** Channels that can't be chosen, with why: "Readable already". */
  taken?: (c: G.SlackChannelChoice) => string | undefined;
}) {
  return (
    <Dialog open={open} onClose={onClose} title={title} description={description} fit>
      <Channels
        workspace={workspace}
        team={team}
        project={project}
        taken={taken}
        note={note}
        onPick={(c) => {
          onPick(c);
          onClose();
        }}
      />
    </Dialog>
  );
}

function Channels({
  workspace,
  team,
  project,
  taken,
  note = "Private channels aren't listed: their members invite fctl themselves.",
  onPick,
}: {
  workspace: string;
  team: string;
  project?: string;
  taken?: (c: G.SlackChannelChoice) => string | undefined;
  note?: string;
  onPick: (c: G.SlackChannelChoice) => void;
}) {
  const [typed, setTyped] = useState("");
  const [query, setQuery] = useState("");
  useEffect(() => {
    const t = setTimeout(() => setQuery(bareChannel(typed).toLowerCase()), SETTLE_MS);
    return () => clearTimeout(t);
  }, [typed]);
  const list = useQuery({
    queryKey: keys.slackChannels(workspace, team, query),
    queryFn: () => workspaces.slackChannels(workspace, team, query || undefined, project),
    staleTime: 60_000,
    // The channels found before stay until the new ones come, so that the list doesn't blink.
    placeholderData: (previous) => previous,
    retry: false,
  });
  const listId = useId();
  const first = useRef<HTMLButtonElement>(null);
  const channels = list.data?.channels ?? [];
  // Down from the field goes to the first channel; the list's own buttons take Tab and the arrows.
  const down = (e: KeyboardEvent) => {
    if (e.key === "ArrowDown") {
      e.preventDefault();
      first.current?.focus();
    }
  };
  const step = (e: KeyboardEvent<HTMLButtonElement>) => {
    if (e.key !== "ArrowDown" && e.key !== "ArrowUp") return;
    e.preventDefault();
    const li = e.currentTarget.closest("li");
    const next = (e.key === "ArrowDown" ? li?.nextElementSibling : li?.previousElementSibling)?.querySelector(
      "button:not(:disabled)",
    );
    if (next instanceof HTMLButtonElement) next.focus();
  };
  // Touch screens: the keyboard would cover the channels.
  const touch = typeof window !== "undefined" && window.matchMedia?.("(hover: none)").matches;
  return (
    <div className="flex min-h-0 flex-auto flex-col gap-3">
      <div className="relative">
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute top-1/2 left-3 size-4 -translate-y-1/2 text-ink-4"
        />
        <Input
          value={typed}
          onChange={(e) => setTyped(e.target.value)}
          onKeyDown={down}
          placeholder="Find a channel: incidents, deploys…"
          aria-label="Find a channel"
          aria-controls={listId}
          autoFocus={!touch}
          autoComplete="off"
          spellCheck={false}
          className="pl-9"
        />
      </div>
      <div className="min-h-0 flex-auto overflow-y-auto">
        {list.isPending ? (
          <div className="flex flex-col gap-2 py-1">
            <Skeleton className="h-11" />
            <Skeleton className="h-11" />
            <Skeleton className="h-11" />
          </div>
        ) : list.isError ? (
          <Banner tone="error">
            Slack's channels didn't load: {list.error.message}{" "}
            <button
              type="button"
              className="underline underline-offset-2"
              onClick={() => void list.refetch()}
            >
              Try again
            </button>
          </Banner>
        ) : channels.length === 0 ? (
          <p className="py-6 text-center text-[13px] text-ink-3">
            {query
              ? `No public channel's name has “${query}”.`
              : "This Slack workspace has no public channel fctl can see."}
          </p>
        ) : (
          <ul
            id={listId}
            aria-label="Channels"
            className={cn("flex flex-col", list.isFetching && "opacity-70")}
          >
            {channels.map((c, i) => {
              const why = taken?.(c);
              return (
                <li key={c.id}>
                  <button
                    ref={i === 0 ? first : undefined}
                    type="button"
                    disabled={!!why}
                    onClick={() => onPick(c)}
                    onKeyDown={step}
                    className="flex w-full items-start gap-2.5 rounded-md px-2.5 py-2 text-left transition-colors enabled:hover:bg-hover disabled:cursor-default"
                  >
                    <Hash aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-ink-4" />
                    <span className="min-w-0 flex-1">
                      <span className="flex flex-wrap items-baseline gap-x-2">
                        <span
                          className={cn(
                            "text-[13.5px] font-medium [overflow-wrap:anywhere]",
                            why ? "text-ink-3" : "text-ink",
                          )}
                        >
                          {c.name}
                        </span>
                        {why ? (
                          <span className="text-[11.5px] text-ink-4">{why}</span>
                        ) : (
                          c.member && <span className="text-[11.5px] text-accent">fctl is in it</span>
                        )}
                      </span>
                      {c.topic && (
                        <span className="mt-0.5 block truncate text-[12px] text-ink-3">{c.topic}</span>
                      )}
                    </span>
                    {c.members !== undefined && (
                      <span className="mt-[2px] inline-flex shrink-0 items-center gap-1 text-[11.5px] text-ink-4 tabular-nums">
                        <Users aria-hidden="true" className="size-3" />
                        {c.members}
                        <span className="sr-only">{c.members === 1 ? " member" : " members"}</span>
                      </span>
                    )}
                  </button>
                </li>
              );
            })}
          </ul>
        )}
      </div>
      <p className="text-[12px] leading-relaxed text-ink-4">
        {list.data?.more ? "Slack has more channels than these: type to narrow them. " : ""}
        {note}
      </p>
    </div>
  );
}

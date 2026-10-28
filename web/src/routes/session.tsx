// A session: the agent at work, live.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  ArrowUpRight,
  Bell,
  BellOff,
  BellRing,
  ChevronRight,
  Ellipsis,
  ExternalLink,
  GitBranch,
  Globe,
  PanelRight,
  Play,
  Search,
  Share2,
  Square,
  Terminal,
  Undo2,
  Users,
} from "lucide-react";
import { useEffect, useRef, useState } from "react";
import { GithubAccessChip, GithubAccessLabel } from "@/components/github-access";
import { ReviewFacts, ReviewFooter, useSessionReview } from "@/components/review";
import { Changes } from "@/components/session/changes";
import { Composer } from "@/components/session/composer";
import { FindBar, findShortcut, Reveal, useFind } from "@/components/session/find";
import {
  SessionMemory,
  SessionRetrospectives,
  useRetrospectives,
  useSessionMemory,
} from "@/components/session/kept";
import { AgentOrb } from "@/components/session/orb";
import {
  PullRequestChecksIcon,
  PullRequestReviewLine,
  PullRequestReviewMark,
  PullRequestStateIcon,
} from "@/components/session/pull-request";
import { ScheduleLink, TicketLink } from "@/components/session/row";
import { Transcript } from "@/components/session/transcript";
import { HeaderMenuButton } from "@/components/shell/menu-button";
import {
  Avatar,
  AvatarStack,
  Banner,
  Button,
  CopyButton,
  Dialog,
  Field,
  IconButton,
  Input,
  Menu,
  MenuItem,
  Select,
  type SelectOption,
  Sheet,
  Skeleton,
  StatusPill,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { followUpHint } from "@/lib/follow-ups";
import { ago, duration, num, until } from "@/lib/format";
import { hasChanges, repoChanges } from "@/lib/git";
import { onGithub, sessionGithub } from "@/lib/github-access";
import { useLiveSession } from "@/lib/live";
import {
  checksText,
  prFullRef,
  prLabel,
  prRef,
  prReview,
  prState,
  prStateText,
  prTitle,
  pullRequests,
} from "@/lib/pulls";
import { keys, queryClient, useMe, useProject } from "@/lib/queries";
import { claudeAppLink, remoteControlUrl, remoteControlUser } from "@/lib/remote-control";
import { type Stall, stallLabel, stallText, useStall } from "@/lib/stall";
import { type StatusNote, statusNotes, statusReason } from "@/lib/status";
import { useStickToBottom } from "@/lib/stick-to-bottom";
import type { Item } from "@/lib/transcript";
import { agentTime, type ContextUse, contextNote, contextUse, turnsTaken } from "@/lib/work";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

export function SessionPage() {
  const { id } = useParams({ from: "/app/s/$id" });
  // From a search of all sessions: what it found here, and where.
  const { find, at } = useSearch({ from: "/app/s/$id" });
  const navigate = useNavigate();
  const { data: me } = useMe();
  const initial = useQuery({ queryKey: keys.session(id), queryFn: () => api.session(id) });
  const [s, store] = useLiveSession(id, initial.data);
  const session = s.session ?? initial.data;
  const prs = session ? pullRequests(session) : [];
  const workspaceIndex = useWorkspaceIndex();
  const stall = useStall(session, s);
  const review = useSessionReview(session);
  const [rail, setRail] = useState(true);
  // The details sheet, where the rail doesn't fit.
  const [details, setDetails] = useState(false);
  const [share, setShare] = useState(false);
  const transcript = useRef<HTMLDivElement>(null);
  const header = useRef<HTMLDivElement>(null);
  const footer = useRef<HTMLDivElement>(null);
  // The newest work stays in view while the reader is at the bottom.
  const letGo = useStickToBottom(transcript, id);
  // What a search of all sessions found: the find bar takes it over, and the address forgets it.
  const [landing, setLanding] = useState<{ query: string; seq?: number }>();
  useEffect(() => {
    if (!find) return;
    setLanding({ query: find, seq: at });
    // Without a view transition of its own: one would cut short the one that opened the page.
    void navigate({ to: "/s/$id", params: { id }, search: {}, replace: true, viewTransition: false });
  }, [find, at, id, navigate]);
  const finder = useFind({
    store,
    items: s.items,
    cwd: s.items.find((it) => it.kind === "init")?.cwd,
    firstSeq: s.firstSeq,
    hasMoreBefore: s.hasMoreBefore,
    loadingOlder: s.loadingOlder,
    olderFailed: s.olderFailed,
    transcript,
    header,
    footer,
    letGo,
    landing,
  });

  useEffect(() => {
    if (session) document.title = `${session.title} · ${session.project_slug}#${session.number}`;
    return () => {
      document.title = "fctl";
    };
  }, [session]);

  const resume = useMutation({
    mutationFn: (b?: { prompt?: string; attachments?: G.AttachmentId[] }) =>
      api.resume(id, b?.prompt, b?.attachments),
  });
  const stop = useMutation({
    mutationFn: () => api.stop(id),
    onSuccess: () => toast("Stopping the session"),
  });
  // Live updates go to everyone watching and leave out whether *you* follow it.
  const following = initial.data?.following ?? false;
  const follow = useMutation({
    mutationFn: () => (following ? api.unfollow(id) : api.follow(id)),
    onSuccess: async (updated) => {
      queryClient.setQueryData<G.Session>(keys.session(id), (old) =>
        old ? { ...old, ...updated } : updated,
      );
      if (!updated.following) {
        toast("You no longer get notifications about this session");
        return;
      }
      const n = await queryClient
        .fetchQuery({ queryKey: keys.notifications, queryFn: api.notifications, staleTime: 60_000 })
        .catch(() => undefined);
      toast(
        n && n.devices.length === 0
          ? "Following. Turn on notifications in Settings to get them in a browser."
          : "Following: you get a notification when it waits for you, pauses or fails",
      );
    },
    onError: (e) => toast(e.message, "error"),
  });

  if (initial.isError && !session) {
    return (
      <div className="mx-auto max-w-xl px-6 py-24">
        <Banner tone="error">This session does not exist, or you cannot see it.</Banner>
      </div>
    );
  }

  const status = session?.status;
  // The session's workspace as your list has it now (renames and avatars).
  const workspace = freshest(workspaceIndex, session?.workspace ?? initial.data?.workspace);
  // Only the person on whose Claude subscription it runs can follow it in the Claude apps.
  const claudeUrl = session && remoteControlUrl(session.remote_control, session.created_by, me?.user.id);
  const role = s.role ?? session?.role;
  const isOwner = role === "owner";
  const notes = session ? statusNotes(session, { reconnecting: s.connection === "reconnecting", stall }) : [];

  return (
    <div className="flex min-h-dvh">
      <div className="flex min-w-0 flex-1 flex-col">
        {/* Header. On phones it holds the menu button, in place of the top bar it would cover,
            and has room for the title and status only: the orb and title open the rest, with
            what the rail shows on wider screens, in a sheet. */}
        <div ref={header} className="sticky top-0 z-10">
          {/* Above the find bar, so that its menu opens over it. */}
          <header className="relative z-1 border-b border-line bg-canvas/80 backdrop-blur-xl max-md:h-14">
            {/* On phones as tall as the top bar, with the menu button where the bar has it. */}
            <div className="flex h-full items-center gap-3 px-2.5 md:gap-4 md:py-3.5 md:pr-6 md:pl-6">
              {/* Over the edge of the title's button, which reaches out to it. */}
              <HeaderMenuButton className="relative z-1 -mr-3" />
              <div className="relative flex min-w-0 flex-1 items-center gap-3 md:gap-4">
                <AgentOrb status={status} activity={s.activity} className="size-9 md:size-11" />
                <div className="min-w-0 flex-1">
                  {session ? (
                    <>
                      <h1 className="flex items-center gap-0.5 font-display text-[16px] leading-5 font-medium tracking-[-0.011em] text-ink md:text-[21px] md:leading-tight md:tracking-[-0.012em]">
                        <span className="truncate">{session.title}</span>
                        <ChevronRight className="size-4 shrink-0 text-ink-4 md:hidden" />
                      </h1>
                      {/* Phones: the status, and what says most about it. */}
                      <p className="mt-0.5 flex items-center gap-1.5 text-[12.5px] leading-4 md:hidden">
                        <StatusPill
                          status={session.status}
                          label={reviewLabel(session)}
                          className="shrink-0"
                        />
                        <span aria-hidden="true" className="text-ink-4">
                          ·
                        </span>
                        {notes[0] ? (
                          <span className={cn("truncate", notes[0].warn ? "text-amber" : "text-ink-3")}>
                            {notes[0].text}
                          </span>
                        ) : (
                          <span className="truncate font-mono text-[12px] text-ink-3">
                            {session.project_slug}#{session.number}
                          </span>
                        )}
                      </p>
                      <div className="max-md:hidden">
                        {session.description && (
                          <p className="mt-0.5 truncate text-[13.5px] text-ink-2" title={session.description}>
                            {session.description}
                          </p>
                        )}
                        <div className="mt-1 flex flex-wrap items-center gap-x-3 gap-y-1 text-[12.5px] text-ink-3">
                          <span className="inline-flex min-w-0 items-center gap-1.5">
                            {workspace && (
                              <>
                                <Link
                                  to="/w/$slug"
                                  params={{ slug: workspace.slug }}
                                  className="inline-flex min-w-0 items-center gap-1.5 text-ink-2 hover:text-ink"
                                >
                                  <WorkspaceAvatar workspace={workspace} size={15} />
                                  <span className="max-w-[12rem] truncate">{workspace.name}</span>
                                </Link>
                                <span className="text-ink-4">/</span>
                              </>
                            )}
                            <Link
                              to="/p/$slug"
                              params={{ slug: session.project_slug }}
                              search={{ tab: undefined }}
                              className="font-mono text-ink-2 hover:text-ink"
                            >
                              {session.project_slug}#{session.number}
                            </Link>
                          </span>
                          <StatusPill status={session.status} label={reviewLabel(session)} />
                          <ScheduleLink session={session} />
                          <TicketLink session={session} />
                          {/* A review agent only reads, and its footer says what fctl posts for it. */}
                          {!session.review && <SessionGithub session={session} />}
                          {statusReason(session) && (
                            <span className="truncate text-ink-4">{statusReason(session)}</span>
                          )}
                          {session.resume_at && (
                            <span className="text-amber">resumes in {until(session.resume_at)}</span>
                          )}
                          {session.branch && (
                            <span className="inline-flex min-w-0 items-center gap-1 font-mono text-[12px] text-ink-3">
                              <GitBranch className="size-3 shrink-0" />
                              <span className="[overflow-wrap:anywhere]">{session.branch}</span>
                            </span>
                          )}
                          {prs.length > 0 && (
                            // Together, so that they wrap to the next line as one list.
                            <span className="inline-flex flex-wrap items-center gap-x-3 gap-y-1">
                              {prs.map((pr) => (
                                <a
                                  key={pr.url}
                                  href={pr.url}
                                  target="_blank"
                                  rel="noreferrer"
                                  title={prTitle(pr)}
                                  aria-label={prLabel(pr)}
                                  className="inline-flex items-center gap-1 text-ink-2 hover:text-ink"
                                >
                                  <PullRequestStateIcon pr={pr} className="size-3" />
                                  {prRef(pr)}
                                  <PullRequestChecksIcon pr={pr} className="size-3" />
                                  <PullRequestReviewMark pr={pr} />
                                </a>
                              ))}
                            </span>
                          )}
                          {s.connection === "reconnecting" && (
                            <span className="text-amber">Reconnecting…</span>
                          )}
                          {stall && <span className="text-amber">{stallLabel(stall)}</span>}
                        </div>
                      </div>
                    </>
                  ) : (
                    <>
                      <Skeleton className="h-5 w-40 md:h-6 md:w-80" />
                      <Skeleton className="mt-1.5 h-3.5 w-24 md:mt-2 md:w-56" />
                    </>
                  )}
                </div>
                {session && (
                  // Over the orb and the title, which are what people tap.
                  <button
                    type="button"
                    aria-label="Details"
                    aria-haspopup="dialog"
                    aria-expanded={details}
                    onClick={() => setDetails(true)}
                    className="absolute -inset-x-1.5 -inset-y-1 rounded-xl transition-colors duration-150 hover:bg-hover active:bg-hover md:hidden"
                  />
                )}
              </div>
              <div className="flex shrink-0 items-center gap-1.5">
                {s.viewers.length > 1 && (
                  // On phones the details list who watches.
                  <span className="hidden md:contents">
                    <AvatarStack users={s.viewers.map((v) => v.user)} />
                  </span>
                )}
                {claudeUrl && (
                  <a
                    {...claudeAppLink(claudeUrl)}
                    className="hidden h-8 items-center gap-1.5 rounded-md border border-accent/30 bg-accent-soft px-3 text-[12.5px] font-medium text-accent transition-colors hover:border-accent/60 md:inline-flex"
                  >
                    Open in Claude <ArrowUpRight className="size-3.5" />
                  </a>
                )}
                {session && (
                  // On phones it is in the menu, where the title keeps its room.
                  <IconButton
                    label={`Find in this session (${findShortcut()})`}
                    aria-expanded={finder.open}
                    onClick={finder.show}
                    className={cn("max-md:hidden", finder.open && "text-ink")}
                  >
                    <Search className="size-4" />
                  </IconButton>
                )}
                {session && (
                  // On phones it is in the menu, where the title keeps its room.
                  <span className="hidden md:contents">
                    <IconButton
                      label={
                        following
                          ? "Following: you get its notifications. Click to stop."
                          : "Follow: get a notification when it waits for you, pauses or fails"
                      }
                      aria-pressed={following}
                      onClick={() => follow.mutate()}
                      disabled={follow.isPending}
                      className={cn(following && "bg-accent-soft")}
                    >
                      {following ? <BellRing className="size-4 text-accent" /> : <Bell className="size-4" />}
                    </IconButton>
                  </span>
                )}
                {isOwner && (
                  // Also in the menu on phones (`hidden` would lose to IconButton's `inline-flex`).
                  <IconButton label="Share" onClick={() => setShare(true)} className="max-md:hidden">
                    <Share2 className="size-4" />
                  </IconButton>
                )}
                {/* The rail where it fits. */}
                <IconButton
                  label="Details"
                  onClick={() => setRail(!rail)}
                  className={cn("max-lg:hidden", rail && "text-ink")}
                >
                  <PanelRight className="size-4" />
                </IconButton>
                {session && (
                  // Narrower, the details sheet: on phones the title opens it.
                  <IconButton
                    label="Details"
                    aria-haspopup="dialog"
                    aria-expanded={details}
                    onClick={() => setDetails(true)}
                    className="max-md:hidden lg:hidden"
                  >
                    <PanelRight className="size-4" />
                  </IconButton>
                )}
                <Menu
                  trigger={(toggle, open) => (
                    <IconButton
                      label="More"
                      aria-haspopup="menu"
                      aria-expanded={open}
                      onClick={toggle}
                      className="max-md:size-11"
                    >
                      <Ellipsis className="size-5 md:size-4" />
                    </IconButton>
                  )}
                >
                  {(close) => (
                    <>
                      {claudeUrl && (
                        <>
                          {/* The header has no room for the button on phones. */}
                          <MenuItem
                            icon={<ArrowUpRight />}
                            link={claudeAppLink(claudeUrl)}
                            onClick={close}
                            className="md:hidden"
                          >
                            Open in Claude
                          </MenuItem>
                          <MenuItem
                            icon={<Globe />}
                            link={{ href: claudeUrl, target: "_blank", rel: "noreferrer" }}
                            onClick={close}
                          >
                            Open on claude.ai
                          </MenuItem>
                        </>
                      )}
                      {isOwner && status && !["stopped", "failed"].includes(status) && (
                        <MenuItem
                          icon={<Square />}
                          onClick={() => {
                            close();
                            stop.mutate();
                          }}
                        >
                          {session?.review ? "Stop the review" : "Stop the session"}
                        </MenuItem>
                      )}
                      {isOwner && !session?.review && (status === "stopped" || status === "failed") && (
                        <MenuItem
                          icon={<Play />}
                          onClick={() => {
                            close();
                            resume.mutate(undefined);
                          }}
                        >
                          Resume
                        </MenuItem>
                      )}
                      {session && (
                        // The header has no room for its button on phones.
                        <MenuItem
                          icon={<Search />}
                          onClick={() => {
                            close();
                            finder.show();
                          }}
                          className="md:hidden"
                        >
                          Find in this session
                        </MenuItem>
                      )}
                      {session && (
                        <MenuItem
                          icon={following ? <BellOff /> : <Bell />}
                          onClick={() => {
                            close();
                            follow.mutate();
                          }}
                        >
                          {following ? "Stop following" : "Follow"}
                        </MenuItem>
                      )}
                      {isOwner && (
                        // The header has no room for its button on phones.
                        <MenuItem
                          icon={<Share2 />}
                          onClick={() => {
                            close();
                            setShare(true);
                          }}
                          className="md:hidden"
                        >
                          Share
                        </MenuItem>
                      )}
                      <MenuItem
                        icon={<Terminal />}
                        onClick={() => {
                          close();
                          void navigator.clipboard.writeText(`fctl attach ${id}`);
                          toast("Copied `fctl attach` command");
                        }}
                      >
                        Copy `fctl attach`
                      </MenuItem>
                      <MenuItem
                        icon={<ExternalLink />}
                        onClick={() => {
                          close();
                          void navigator.clipboard.writeText(location.href);
                          toast("Link copied");
                        }}
                      >
                        Copy link
                      </MenuItem>
                    </>
                  )}
                </Menu>
              </div>
            </div>
          </header>
          {/* Finding in the transcript: under the header, over the page, which stays where it is. */}
          {finder.open && (
            <div className="absolute inset-x-0 top-full border-b border-line bg-canvas/85 backdrop-blur-xl">
              <div className="mx-auto w-full max-w-[760px] px-2.5 py-2 md:px-6 md:py-2.5">
                <FindBar {...finder.bar} />
              </div>
            </div>
          )}
        </div>

        {/* Transcript */}
        <div ref={transcript} className="mx-auto w-full max-w-[760px] flex-1 px-6 pt-6 pb-4">
          {s.error && (
            <Banner tone="error" className="mb-4">
              {s.error}
            </Banner>
          )}
          {!session || (s.connection === "connecting" && !s.items.length) ? (
            <div className="space-y-4 pt-4">
              <Skeleton className="h-16 w-3/4" />
              <Skeleton className="h-4 w-1/2" />
              <Skeleton className="h-24 w-full" />
            </div>
          ) : (
            <Reveal.Provider value={finder.reveal}>
              <Transcript
                sessionId={id}
                items={s.items}
                live={s.live}
                status={status}
                stall={stall}
                canManage={isOwner}
                startedBy={session.created_by}
                me={me?.user.id}
                projectSlug={session.project_slug}
                onLoadOlder={() => void store.loadOlder()}
                hasMoreBefore={s.hasMoreBefore}
                loadingOlder={s.loadingOlder}
                olderFailed={s.olderFailed}
              />
            </Reveal.Provider>
          )}
        </div>

        {/* Composer: a review agent takes no messages, and says what becomes of its review instead. */}
        <div
          ref={footer}
          className="sticky bottom-0 z-10 bg-gradient-to-t from-canvas via-canvas/95 to-transparent pt-6 pb-[max(1.25rem,env(safe-area-inset-bottom))]"
        >
          <div className="mx-auto w-full max-w-[760px] px-6">
            {session?.review ? (
              <ReviewFooter session={session} review={review} />
            ) : (
              <Composer
                status={status}
                role={role}
                viewers={s.viewers}
                meId={me?.user.id}
                sessionId={id}
                onSend={(t, files) =>
                  store.send(
                    t,
                    me?.user
                      ? { id: me.user.id, name: me.user.name, avatar_url: me.user.avatar_url }
                      : undefined,
                    files,
                  )
                }
                onInterrupt={() => store.interrupt()}
                onTyping={() => store.typing()}
                onResume={async (t, files) => {
                  await resume.mutateAsync({
                    prompt: t,
                    attachments: files.attachments.map((a) => a.id),
                  });
                  for (const url of Object.values(files.previews)) URL.revokeObjectURL(url);
                }}
              />
            )}
          </div>
        </div>
      </div>

      {rail && session && (
        <aside className="sticky top-0 hidden h-dvh w-[315px] shrink-0 overflow-y-auto border-l border-line bg-surface/40 lg:block">
          <Details
            session={session}
            review={review}
            me={me?.user.id}
            git={s.git}
            items={s.items}
            viewers={s.viewers}
            model={s.model}
            runnerConnected={s.runnerConnected}
            stall={stall}
          />
        </aside>
      )}
      {details && session && (
        <Sheet title={session.title} description={session.description} onClose={() => setDetails(false)}>
          <Overview session={session} workspace={workspace} notes={notes} />
          <Details
            session={session}
            review={review}
            me={me?.user.id}
            git={s.git}
            items={s.items}
            viewers={s.viewers}
            model={s.model}
            runnerConnected={s.runnerConnected}
            stall={stall}
            pullsFirst
          />
        </Sheet>
      )}
      {session && <ShareDialog open={share} onClose={() => setShare(false)} session={session} />}
    </div>
  );
}

/** A review agent's session waits for nobody: once its turn ends, it finishes and stops. */
const reviewLabel = (session: G.Session) =>
  session.review && session.status === "idle" ? "Finishing" : undefined;

/** What the session's agent may do on GitHub, beside its status, when that is less than push. */
function SessionGithub({ session }: { session: G.Session }) {
  const project = useProject(session.project_slug).data;
  return <GithubAccessChip access={sessionGithub(session, project)} />;
}

function RailSection({ title, children }: { title: string; children: React.ReactNode }) {
  return (
    <section className="border-b border-line px-5 py-4 last:border-b-0">
      <h3 className="mb-2.5 text-[12.5px] font-medium text-ink-3">{title}</h3>
      {children}
    </section>
  );
}

function Row({ k, v }: { k: string; v: React.ReactNode }) {
  return (
    <div className="flex items-baseline justify-between gap-3 py-0.5 text-[12.5px]">
      <span className="text-ink-4">{k}</span>
      <span className="min-w-0 truncate text-right text-ink-2">{v}</span>
    </div>
  );
}

/**
 * The agent's time so far, counting up each second while a turn is under
 * way, up to `until` when its work stopped coming through.
 */
function AgentTime({ stats, until }: { stats: G.SessionStats; until?: number }) {
  const ticking = stats.turn_started_at != null && until == null;
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    if (!ticking) return;
    setNow(Date.now());
    const t = setInterval(() => setNow(Date.now()), 1000);
    return () => clearInterval(t);
  }, [ticking]);
  return <span className="tabular-nums">{duration(agentTime(stats, now, until))}</span>;
}

/**
 * How full Claude's context is: the tokens in it and the window, a meter
 * split where Claude compacts the conversation, amber nine tenths of the way
 * there, and how often it compacted. The text says what the meter shows for
 * screen readers.
 */
function ContextMeter({ stats }: { stats: G.SessionStats }) {
  const use = contextUse(stats);
  if (!use) return null;
  const note = contextNote(use);
  const exact = (n: number) => n.toLocaleString();
  const [before, after] = use.parts;
  return (
    <>
      <Row
        k="Context"
        v={
          <span
            className="tabular-nums"
            title={`${exact(use.tokens)}${use.window ? ` of ${exact(use.window)}` : ""} tokens`}
          >
            {use.window ? `${num(use.tokens)} / ${num(use.window)}` : num(use.tokens)}
          </span>
        }
      />
      {before && (
        <div aria-hidden className="mt-1.5 flex h-1 gap-0.5">
          <ContextPart {...before} close={use.close} />
          {after && <ContextPart {...after} close={use.close} />}
        </div>
      )}
      {note && (
        <p
          className={cn("mt-1.5 text-[11.5px]", use.close ? "text-amber" : "text-ink-4")}
          title={
            use.compactAt != null
              ? `Claude summarizes the conversation to make room once its context holds ${exact(use.compactAt)} tokens`
              : undefined
          }
        >
          {note}
        </p>
      )}
    </>
  );
}

/** One part of the context meter, as wide as the share of the window it spans. */
function ContextPart({ span, used, close }: ContextUse["parts"][number] & { close: boolean }) {
  return (
    <div className="overflow-hidden rounded-full bg-line" style={{ flex: `${span} 1 0` }}>
      <div
        className={cn(
          "h-full rounded-full transition-[width] duration-300 motion-reduce:transition-none",
          close ? "bg-amber" : "bg-ink-3",
        )}
        style={{ width: `${used * 100}%` }}
      />
    </div>
  );
}

/**
 * Every pull request of the session, with the repository of each, its state, its checks and how
 * fctl's review of it stands, which opens the review's session.
 */
function PullRequests({ prs, session }: { prs: G.PullRequestRef[]; session: G.Session }) {
  return (
    <ul className="flex flex-col gap-1.5">
      {prs.map((pr) => {
        const known = prState(pr) !== "unknown";
        const checks = checksText(pr);
        // A review agent's session has its review in the rail's Review section.
        const review = prReview(pr);
        const reviewed = review && review.session_id !== session.id ? review : undefined;
        return (
          <li
            key={pr.url}
            className="rounded-md border border-line transition-colors hover:border-line-strong"
          >
            <a
              href={pr.url}
              target="_blank"
              rel="noreferrer"
              title={prTitle(pr)}
              className="flex items-start gap-2.5 px-3 py-2 text-[12.5px] text-ink-2 transition-colors hover:text-ink"
            >
              <PullRequestStateIcon pr={pr} className="mt-px size-4" />
              <span className="min-w-0 flex-1">
                <span className="block truncate">{pr.title ?? `Pull request #${pr.number}`}</span>
                <span className="block truncate font-mono text-[11.5px] text-ink-4">{prFullRef(pr)}</span>
                {(known || checks) && (
                  // The checks' icon sets them apart, and leads their line when they wrap.
                  <span className="mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-0.5 text-[11.5px] text-ink-3">
                    {known && <span>{prStateText(pr)}</span>}
                    {checks && (
                      <span className="inline-flex items-center gap-1">
                        <PullRequestChecksIcon pr={pr} className="size-3" />
                        {checks}
                      </span>
                    )}
                  </span>
                )}
              </span>
              <ArrowUpRight className="mt-0.5 size-3.5 shrink-0 text-ink-4" />
            </a>
            {reviewed && <PullRequestReviewLine pr={pr} review={reviewed} />}
          </li>
        );
      })}
    </ul>
  );
}

/**
 * What the header has no room for on phones: the status with all it says,
 * where the session is and its branch. The details sheet starts with it.
 */
function Overview({
  session,
  workspace,
  notes,
}: {
  session: G.Session;
  workspace?: G.WorkspaceRef;
  notes: StatusNote[];
}) {
  return (
    <section className="flex flex-col gap-3 border-b border-line px-5 py-4 text-[12.5px]">
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <StatusPill status={session.status} label={reviewLabel(session)} />
        {notes.map((n) => (
          <span key={n.text} className={n.warn ? "text-amber" : "text-ink-3"}>
            {n.text}
          </span>
        ))}
      </div>
      <div className="flex min-w-0 items-center gap-1.5">
        {workspace && (
          <>
            <Link
              to="/w/$slug"
              params={{ slug: workspace.slug }}
              className="-my-1 inline-flex min-w-0 items-center gap-1.5 py-1 text-ink-2 hover:text-ink"
            >
              <WorkspaceAvatar workspace={workspace} size={16} />
              <span className="truncate">{workspace.name}</span>
            </Link>
            <span className="text-ink-4">/</span>
          </>
        )}
        <Link
          to="/p/$slug"
          params={{ slug: session.project_slug }}
          search={{ tab: undefined }}
          className="-my-1 shrink-0 py-1 font-mono text-ink-2 hover:text-ink"
        >
          {session.project_slug}#{session.number}
        </Link>
      </div>
      {session.branch && (
        <div className="flex items-start gap-2 text-ink-3">
          <GitBranch className="mt-px size-3.5 shrink-0" />
          <span className="min-w-0 flex-1 font-mono text-[12px] [overflow-wrap:anywhere]">
            {session.branch}
          </span>
          <CopyButton text={session.branch} className="-my-1 -mr-2 shrink-0" />
        </div>
      )}
    </section>
  );
}

/** What the rail shows beside the transcript, and the details sheet where the rail doesn't fit. */
function Details({
  session,
  review,
  me,
  git,
  items,
  viewers,
  model,
  runnerConnected,
  stall,
  pullsFirst,
}: {
  session: G.Session;
  /** What came of it, for a review agent's session. */
  review?: G.Review;
  /** The signed-in person. */
  me?: G.UserId;
  git?: G.GitState;
  /** The transcript, whose retrospectives come in live. */
  items: Item[];
  viewers: G.Presence[];
  model?: string;
  runnerConnected: boolean;
  stall?: Stall;
  /** The pull requests before the changes, as the sheet has them: on phones they're what people open. */
  pullsFirst?: boolean;
}) {
  const st = session.stats;
  const project = useProject(session.project_slug).data;
  const retrospectives = useRetrospectives(session, items);
  const memory = useSessionMemory(session, project);
  const [primary, ...others] = repoChanges(
    git ?? { ahead: 0, behind: 0, files: [], uncommitted: 0, pushed: false },
    sessionGithub(session, project) === "push",
  );
  // Several repositories: each under its name, those without changes left out.
  const changed = others.filter(hasChanges);
  const prs = pullRequests(session);
  const rc = session.remote_control;
  const claudeUrl = remoteControlUrl(rc, session.created_by, me);
  const rcUser = remoteControlUser(rc, session.created_by);
  // The Claude apps show it only to the person on whose subscription it runs.
  const claudeApps = claudeUrl ? (
    <>
      <a
        {...claudeAppLink(claudeUrl)}
        className="flex items-center justify-between rounded-md border border-accent/25 bg-accent-soft px-3 py-2 text-[12.5px] text-accent"
      >
        Open in the Claude app <ArrowUpRight className="size-3.5" />
      </a>
      <div className="mt-3 flex items-center justify-between">
        <span className="text-[12.5px] text-ink-4">Browser</span>
        <a
          href={claudeUrl}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1.5 rounded-md px-2 py-1 text-[12px] text-ink-3 transition-colors hover:bg-hover hover:text-ink"
        >
          claude.ai <ArrowUpRight className="size-3.5" />
        </a>
      </div>
    </>
  ) : rcUser.id !== me ? (
    rc?.session_url ? (
      <p className="text-[12.5px] leading-relaxed text-ink-4">
        Only {rcUser.name} can follow it in the Claude apps: it runs on their Claude subscription.
      </p>
    ) : null
  ) : (
    <p className="text-[12.5px] leading-relaxed text-ink-4">
      {rc?.unavailable_reason ?? "Remote Control appears here when the session's Claude login supports it."}
    </p>
  );
  const hint = session.review ? undefined : followUpHint(project?.settings.follow_ups);
  const pulls = prs.length > 0 && (
    <RailSection title="Pull requests">
      <PullRequests prs={prs} session={session} />
      {hint && (
        <p className="mt-3 flex gap-2 text-[12px] leading-relaxed text-ink-4">
          <Undo2 aria-hidden="true" className="mt-[3px] size-3 shrink-0" />
          {hint}
        </p>
      )}
    </RailSection>
  );
  return (
    <>
      {session.review && (
        <RailSection title="Review">
          <ReviewFacts session={session} review={review} />
        </RailSection>
      )}
      {pullsFirst && pulls}
      <RailSection title={session.review ? "Changes reviewed" : "Changes"}>
        {primary && others.length === 0 ? (
          <Changes repo={primary} showTotals={!!git} />
        ) : (
          <div className="flex flex-col gap-5">
            {primary && (hasChanges(primary) || changed.length === 0) && (
              <Changes
                repo={primary}
                title={project?.repo ? `${project.repo.owner}/${project.repo.name}` : "~/workspace"}
                showTotals={hasChanges(primary)}
              />
            )}
            {changed.map((r) => (
              <Changes key={r.name} repo={r} title={r.name} showTotals />
            ))}
          </div>
        )}
      </RailSection>
      {!pullsFirst && pulls}
      {(retrospectives.length > 0 || (project?.settings.retrospectives && !session.review)) && (
        <RailSection
          title={retrospectives.length > 1 ? `Retrospectives (${retrospectives.length})` : "Retrospective"}
        >
          <SessionRetrospectives list={retrospectives} project={project} />
        </RailSection>
      )}
      {project && memory.shown && !session.review && (
        <RailSection title="Memory">
          <SessionMemory project={project} memory={memory} />
        </RailSection>
      )}
      <RailSection title="Work">
        <Row k="Turns" v={turnsTaken(st)} />
        <Row k="Tool calls" v={st.tool_calls} />
        <Row k="Agent time" v={<AgentTime stats={st} until={stall?.since} />} />
        <Row k="Tokens in / out" v={`${num(st.input_tokens)} / ${num(st.output_tokens)}`} />
        <Row k="Cache reads" v={num(st.cache_read_tokens)} />
        <ContextMeter stats={st} />
      </RailSection>
      <RailSection title="Session">
        <Row
          k={session.review ? "Runs on" : "Started by"}
          v={
            <span
              className="inline-flex items-center gap-1.5"
              title={session.review ? `${session.created_by.name}'s Claude subscription` : undefined}
            >
              <Avatar user={session.created_by} size={16} />
              {session.created_by.name}
            </span>
          }
        />
        <Row k="Created" v={ago(session.created_at)} />
        {session.schedule && (
          <Row k="Schedule" v={<ScheduleLink session={session} className="justify-end" />} />
        )}
        {session.ticket && <Row k="Ticket" v={<TicketLink session={session} className="justify-end" />} />}
        {session.base_branch && <Row k="From" v={<span className="font-mono">{session.base_branch}</span>} />}
        {(model ?? session.model) && <Row k="Model" v={model ?? session.model} />}
        {session.effort && <Row k="Effort" v={session.effort} />}
        {project && onGithub(project) && (
          <Row k="On GitHub" v={<GithubAccessLabel access={sessionGithub(session, project)} />} />
        )}
        {session.github_issues && (
          <Row
            k="Issues"
            v={
              <span title="Its agent reaches the GitHub issues of the repositories it writes to, whatever the project's tracker says.">
                GitHub's too
              </span>
            }
          />
        )}
        {session.runner && (
          <Row
            k="Runner"
            v={
              <span className="inline-flex items-center gap-1.5" title={stall && stallText(stall).title}>
                <span
                  className={cn(
                    "size-1.5 rounded-full",
                    stall?.kind === "unreachable" ? "bg-amber" : runnerConnected ? "bg-accent" : "bg-ink-4",
                  )}
                />
                {session.runner.name}
              </span>
            }
          />
        )}
        <Row k="Billing" v="Claude subscription" />
      </RailSection>
      <RailSection title="Follow along">
        {claudeApps}
        <div className={cn("flex items-center justify-between", claudeApps && "mt-3")}>
          <span className="text-[12.5px] text-ink-4">CLI</span>
          <CopyButton text={`fctl attach ${session.id}`} label="fctl attach" />
        </div>
      </RailSection>
      <RailSection title={`Watching (${viewers.length})`}>
        <ul className="flex flex-col gap-1.5">
          {viewers.map((v) => (
            <li key={v.user.id} className="flex items-center gap-2 text-[12.5px] text-ink-2">
              <Avatar user={v.user} size={18} />
              <span className="min-w-0 flex-1 truncate">{v.user.name}</span>
              {v.connections > 1 && <span className="text-ink-4">×{v.connections}</span>}
            </li>
          ))}
        </ul>
      </RailSection>
    </>
  );
}

const SHARE_ROLES: SelectOption<G.SessionRole>[] = [
  { value: "viewer", label: "Can view", description: "Reads the transcript" },
  { value: "editor", label: "Can message", description: "Also messages and interrupts the agent" },
];

function ShareDialog({ open, onClose, session }: { open: boolean; onClose: () => void; session: G.Session }) {
  const shares = useQuery({
    queryKey: keys.shares(session.id),
    queryFn: () => api.shares(session.id),
    enabled: open,
  });
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<G.SessionRole>("editor");
  const add = useMutation({
    mutationFn: () => api.share(session.id, { email, role }),
    onSuccess: (r) => {
      shares.refetch();
      setEmail("");
      toast(`Shared with ${r.shares[r.shares.length - 1]?.user.name ?? email}`);
    },
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Share this session"
      description="Project members already see it. Share it with anyone else on this instance."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          add.mutate();
        }}
      >
        <Field label="Email" error={add.error?.message}>
          {(id) => (
            <div className="flex gap-2">
              <Input
                id={id}
                type="email"
                required
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="teammate@example.com"
                className="min-w-0 flex-1"
              />
              <Select aria-label="Access" value={role} onChange={setRole} options={SHARE_ROLES} />
              <Button type="submit" variant="primary" size="field" loading={add.isPending}>
                Share
              </Button>
            </div>
          )}
        </Field>
      </form>
      <ul className="mt-5 flex flex-col gap-2">
        {shares.data?.shares.map((sh) => (
          <li key={sh.user.id} className="flex items-center gap-2.5 text-[13px]">
            <Avatar user={sh.user} size={22} />
            <span className="min-w-0 flex-1">
              <span className="block truncate text-ink">{sh.user.name}</span>
              <span className="block truncate text-[12px] text-ink-4">{sh.email}</span>
            </span>
            <span className="text-[12px] text-ink-3">
              {sh.role === "viewer" ? "Can view" : sh.role === "editor" ? "Can message" : "Owner"}
            </span>
            <button
              type="button"
              className="text-[12px] text-ink-4 hover:text-danger"
              onClick={() => api.unshare(session.id, sh.user.id).then(() => shares.refetch())}
            >
              Remove
            </button>
          </li>
        ))}
        {shares.data?.shares.length === 0 && (
          <li className="flex items-center gap-2 text-[12.5px] text-ink-4">
            <Users className="size-3.5" /> Not shared with anyone outside the project.
          </li>
        )}
      </ul>
    </Dialog>
  );
}

// The new session page, which people try beside the classic one (routes/session.tsx) until they
// have compared the two: `SessionRoute` shows the one each person picked. The session reads in
// turns (components/session/turns.tsx), which open with a cover that says what the session is;
// what the agent does now shows above the message box; and long sessions have ways through them:
// the bar that says which turn the reader is in, the list of turns, j and k, the way back down to
// the newest work and a line where what came while the reader was away starts. What it shows
// beside the transcript is the classic page's.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  ArrowUpRight,
  Bell,
  BellOff,
  BellRing,
  Clock,
  Ellipsis,
  ExternalLink,
  GitBranch,
  Globe,
  MessagesSquare,
  PanelRight,
  Play,
  Search,
  Share2,
  Square,
  Terminal,
} from "lucide-react";
import { type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { ReviewFooter, useSessionReview } from "@/components/review";
import { Composer } from "@/components/session/composer";
import { DiffDialog, type DiffTarget } from "@/components/session/diff-dialog";
import { FindBar, findShortcut, Reveal, useFind } from "@/components/session/find";
import { BusyMark } from "@/components/session/lines";
import { JumpPill, NowLine, toBottom, useAtBottom } from "@/components/session/now-line";
import { AgentOrb } from "@/components/session/orb";
import {
  PullRequestChecksIcon,
  PullRequestReviewMark,
  PullRequestStateIcon,
} from "@/components/session/pull-request";
import { QuestionCard } from "@/components/session/question";
import { ScheduleLink, TicketLink } from "@/components/session/row";
import { SessionChoices } from "@/components/session/settings";
import { thinkingNow } from "@/components/session/thinking";
import { TurnBar, TurnList, TurnOutline, useGoToTurn, useTurnPlace } from "@/components/session/turn-nav";
import { TurnTranscript } from "@/components/session/turns";
import { useViewShortcut, ViewSwitch } from "@/components/session/view-switch";
import { HeaderMenuButton } from "@/components/shell/menu-button";
import {
  Avatar,
  AvatarStack,
  Banner,
  CopyButton,
  DotWave,
  IconButton,
  Menu,
  MenuItem,
  MenuSeparator,
  Sheet,
  Skeleton,
  StatusPill,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { setPageTitle } from "@/lib/attention";
import { cn } from "@/lib/cn";
import { stamp, when } from "@/lib/format";
import { useLiveSession } from "@/lib/live";
import { nowOf } from "@/lib/now";
import { prLabel, prRef, prTitle, pullRequests } from "@/lib/pulls";
import { keys, queryClient, useMe } from "@/lib/queries";
import { claudeAppLink, remoteControlUrl } from "@/lib/remote-control";
import { parts } from "@/lib/runs";
import { useSessionView } from "@/lib/session-view";
import { ariaShortcut, useShortcut, withShortcut } from "@/lib/shortcuts";
import { diskOf, useStall } from "@/lib/stall";
import { statusNotes } from "@/lib/status";
import { useStickToBottom } from "@/lib/stick-to-bottom";
import type { Item } from "@/lib/transcript";
import { factsOf, type Turn, type TurnFacts, turnsOf, turnTitle } from "@/lib/turns";
import { cameAfter, newestSeq } from "@/lib/unseen";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";
import {
  Details,
  noteTone,
  Overview,
  RailSection,
  reviewLabel,
  SessionGithub,
  SessionPage,
  ShareDialog,
  useDiffGroups,
} from "./session";

/** A session's page: the classic one, or the new one, as this browser shows sessions. */
export function SessionRoute() {
  const [view] = useSessionView();
  return view === "new" ? <NextSessionPage /> : <SessionPage />;
}

/** Statuses in which the agent is at it, or getting there: the header's edge runs with light. */
const BUSY = new Set<G.SessionStatus>(["running", "provisioning", "preparing"]);

function NextSessionPage() {
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
  const disk = diskOf(session, s, stall);
  const review = useSessionReview(session);
  const [rail, setRail] = useState(true);
  // The details sheet, where the rail doesn't fit.
  const [details, setDetails] = useState(false);
  const [share, setShare] = useState(false);
  // The diffs shown over the page, at the file opened in Changes (or the first, with c).
  const [diffAt, setDiffAt] = useState<DiffTarget>();
  // Only the conversation: the agent's work and the milestones left out.
  const [quiet, setQuiet] = useState(false);
  // They belong to the session they opened on (as on the classic page).
  const [overlaysOf, setOverlaysOf] = useState(id);
  if (overlaysOf !== id) {
    setOverlaysOf(id);
    setDetails(false);
    setShare(false);
    setDiffAt(undefined);
    setQuiet(false);
  }
  const diffGroups = useDiffGroups(session, s.git);
  useShortcut("changes", () => {
    const first = diffGroups.find((g) => g.files.length > 0);
    const file = first?.files[0];
    if (first && file) setDiffAt({ repo: first.repo, path: file.path });
    else if (session) toast(session.review ? "No changes to review yet" : "No changes yet");
  });
  useShortcut(
    "details",
    () => {
      if (!session) return;
      const sheet = details && document.querySelector<HTMLDialogElement>("dialog.sheet[open]");
      if (sheet) {
        if (sheet.dispatchEvent(new Event("cancel", { cancelable: true }))) sheet.close();
      } else if (window.matchMedia?.("(min-width: 64rem)").matches ?? true) setRail((r) => !r);
      else setDetails(true);
    },
    { inDialogs: "dialog.sheet" },
  );
  useViewShortcut();
  const transcript = useRef<HTMLDivElement>(null);
  // What sticks to the top: the header, and the bar under it. Where the header ends, the reader's view starts.
  const header = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLElement>(null);
  const footer = useRef<HTMLDivElement>(null);
  // The newest work stays in view while the reader is at the bottom.
  const letGo = useStickToBottom(transcript, id);
  const [landing, setLanding] = useState<{ query: string; seq?: number }>();
  useEffect(() => {
    if (!find) return;
    setLanding({ query: find, seq: at });
    void navigate({ to: "/s/$id", params: { id }, search: {}, replace: true, viewTransition: false });
  }, [find, at, id, navigate]);
  const cwd = s.items.find((it) => it.kind === "init")?.cwd;
  const finder = useFind({
    store,
    items: s.items,
    cwd,
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
    if (session) setPageTitle(`${session.title} · ${session.project_slug}#${session.number}`);
    return () => setPageTitle(undefined);
  }, [session]);

  const resume = useMutation({
    mutationFn: (b?: { prompt?: string; attachments?: G.AttachmentId[] }) =>
      api.resume(id, b?.prompt, b?.attachments),
  });
  const stop = useMutation({
    mutationFn: () => api.stop(id),
    onSuccess: () => toast("Stopping the session"),
  });
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

  // The turns, for the bar and the list of them; the transcript works them out as it shows them.
  const turns = useMemo(() => turnsOf(parts(s.items)), [s.items]);
  const facts = useFactsOf(cwd);
  const place = useTurnPlace(transcript, bar);
  const go = useGoToTurn(transcript, bar, letGo);
  const current = turns[place.at];
  // j: the next turn, or the newest work after the last; k: back to where this one starts, then the one before.
  useShortcut(
    "nextTurn",
    (e) => {
      if (!turns.length) return;
      if (e.key === "j") {
        if (place.at < turns.length - 1) go(place.at + 1);
        else toBottom();
        return;
      }
      const start = transcript.current?.querySelectorAll<HTMLElement>("[data-turn]")[place.at];
      const top = bar.current?.getBoundingClientRect().bottom ?? 0;
      if (start && start.getBoundingClientRect().top < top - 4) go(place.at);
      else go(Math.max(0, place.at - 1));
    },
    { repeat: true },
  );

  // Coming back to the page: where what came meanwhile starts, and back up to it.
  const away = useAway(s.items, id);
  const atBottom = useAtBottom();
  const newSince = useNewSince(s.items, atBottom);
  const coverTitle = useRef<HTMLDivElement>(null);
  const coverShows = useShows(coverTitle, bar, !s.hasMoreBefore && !!session);

  if (initial.isError && !session) {
    return (
      <div className="mx-auto max-w-xl px-6 py-24">
        <Banner tone="error">This session does not exist, or you cannot see it.</Banner>
      </div>
    );
  }

  const status = session?.status;
  const workspace = freshest(workspaceIndex, session?.workspace ?? initial.data?.workspace);
  const claudeUrl = session && remoteControlUrl(session.remote_control, session.created_by, me?.user.id);
  const role = s.role ?? session?.role;
  const isOwner = role === "owner";
  const notes = session
    ? statusNotes(session, { reconnecting: s.connection === "reconnecting", stall, disk })
    : [];
  const now = nowOf({
    status,
    items: s.items,
    live: s.live,
    stall,
    cwd,
    reason: session?.status_reason,
    resumeAt: session?.resume_at,
    thinking: thinkingNow,
  });
  const live = !!status && BUSY.has(status) && !stall;
  // With the find bar open, all of the transcript shows, for it to go to what it finds: asking for
  // the conversation alone then puts the bar away.
  const quietNow = quiet && !finder.open;
  const toggleQuiet = () => {
    if (quietNow) {
      setQuiet(false);
      return;
    }
    setQuiet(true);
    if (finder.open) finder.close();
  };
  const outline = turns.length > 0 && (
    <TurnOutline
      turns={turns}
      facts={facts}
      at={place.at}
      live={!!status && BUSY.has(status)}
      hasMoreBefore={s.hasMoreBefore}
      onGo={go}
    />
  );

  return (
    <div className="session-next flex min-h-dvh">
      <div className="flex min-w-0 flex-1 flex-col">
        <div ref={header} className="sticky top-(--viewport-shift) z-10">
          <header
            ref={bar}
            className="relative z-2 h-14 border-b border-line bg-canvas/80 backdrop-blur-xl md:h-[60px]"
          >
            {/* While the agent works, light runs along the header's edge. */}
            {live && <span aria-hidden="true" className="header-live" />}
            <div className="flex h-full items-center gap-2.5 px-2.5 md:gap-3 md:pr-5 md:pl-5">
              <HeaderMenuButton className="relative z-1 -mr-2.5" />
              <div className="relative flex min-w-0 flex-1 items-center gap-2.5 md:gap-3">
                <AgentOrb status={status} activity={s.activity} className="size-8 md:size-9" />
                <div className="min-w-0 flex-1">
                  {session ? (
                    <>
                      <h1
                        className={cn(
                          "truncate font-display text-[15.5px] leading-5 font-medium tracking-[-0.011em] text-ink transition-[opacity,translate] duration-300 ease-[var(--ease-out)] md:text-[16.5px]",
                          coverShows && "translate-y-1 opacity-0",
                        )}
                      >
                        {session.title}
                      </h1>
                      <p className="mt-0.5 flex min-w-0 items-center gap-1.5 text-[12px] leading-4 text-ink-3">
                        <StatusPill
                          status={session.status}
                          label={reviewLabel(session)}
                          className="shrink-0 text-[12px]"
                        />
                        {notes[0] && (
                          <>
                            <span aria-hidden="true" className="text-ink-4">
                              ·
                            </span>
                            <span className={cn("truncate", noteTone(notes[0]))}>{notes[0].text}</span>
                          </>
                        )}
                        <span aria-hidden="true" className="text-ink-4 max-md:hidden">
                          ·
                        </span>
                        <Link
                          to="/p/$slug"
                          params={{ slug: session.project_slug }}
                          search={{ tab: undefined }}
                          className="relative z-1 shrink-0 font-mono text-[11.5px] text-ink-3 hover:text-ink max-md:hidden"
                        >
                          {session.project_slug}#{session.number}
                        </Link>
                        {prs.length > 0 && (
                          <span className="inline-flex min-w-0 items-center gap-2.5 overflow-hidden pl-1 max-md:hidden">
                            {prs.map((pr) => (
                              <a
                                key={pr.url}
                                href={pr.url}
                                target="_blank"
                                rel="noreferrer"
                                title={prTitle(pr)}
                                aria-label={prLabel(pr)}
                                className="inline-flex shrink-0 items-center gap-1 text-ink-2 hover:text-ink"
                              >
                                <PullRequestStateIcon pr={pr} className="size-3" />
                                {prRef(pr)}
                                <PullRequestChecksIcon pr={pr} className="size-3" />
                                <PullRequestReviewMark pr={pr} />
                              </a>
                            ))}
                          </span>
                        )}
                      </p>
                    </>
                  ) : (
                    <>
                      <Skeleton className="h-5 w-40 md:w-72" />
                      <Skeleton className="mt-1.5 h-3 w-24 md:w-48" />
                    </>
                  )}
                </div>
                {session && (
                  // On phones the orb and title open the details, as on the classic page.
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
              <div className="flex shrink-0 items-center gap-1">
                {s.viewers.length > 1 && (
                  <span className="hidden pr-1 md:contents">
                    <AvatarStack users={s.viewers.map((v) => v.user)} />
                  </span>
                )}
                {claudeUrl && (
                  <a
                    {...claudeAppLink(claudeUrl)}
                    className="mr-1 hidden h-8 items-center gap-1.5 rounded-md border border-accent/30 bg-accent-soft px-3 text-[12.5px] font-medium text-accent transition-colors hover:border-accent/60 xl:inline-flex"
                  >
                    Open in Claude <ArrowUpRight className="size-3.5" />
                  </a>
                )}
                {session && (
                  <IconButton
                    label={quietNow ? "Show the agent's work" : "Show only the conversation"}
                    aria-pressed={quietNow}
                    onClick={toggleQuiet}
                    className={cn("max-md:hidden", quietNow && "bg-accent-soft text-accent")}
                  >
                    <MessagesSquare className="size-4" />
                  </IconButton>
                )}
                {session && (
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
                  <IconButton label="Share" onClick={() => setShare(true)} className="max-md:hidden">
                    <Share2 className="size-4" />
                  </IconButton>
                )}
                <IconButton
                  label="Details"
                  title={withShortcut("Details", "details")}
                  aria-keyshortcuts={ariaShortcut("details")}
                  aria-pressed={rail}
                  onClick={() => setRail(!rail)}
                  className={cn("max-lg:hidden", rail && "text-ink")}
                >
                  <PanelRight className="size-4" />
                </IconButton>
                {session && (
                  <IconButton
                    label="Details"
                    title={withShortcut("Details", "details")}
                    aria-keyshortcuts={ariaShortcut("details")}
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
                          <MenuItem
                            icon={<ArrowUpRight />}
                            link={claudeAppLink(claudeUrl)}
                            onClick={close}
                            className="xl:hidden"
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
                          icon={<MessagesSquare />}
                          onClick={() => {
                            close();
                            toggleQuiet();
                          }}
                          className="md:hidden"
                        >
                          {quietNow ? "Show the agent's work" : "Show only the conversation"}
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
                      <MenuSeparator />
                      <ViewSwitch onClick={close} />
                    </>
                  )}
                </Menu>
              </div>
            </div>
          </header>
          {/* Under the header: finding in the transcript, or the turn the reader is in. */}
          {finder.open ? (
            <div className="absolute inset-x-0 top-full border-b border-line bg-canvas/85 backdrop-blur-xl">
              <div className="mx-auto w-full max-w-[760px] px-2.5 py-2 md:px-6 md:py-2.5">
                <FindBar {...finder.bar} />
              </div>
            </div>
          ) : (
            session && (
              <TurnBar
                turn={current}
                title={current ? turnTitle(current, place.at === 0, s.hasMoreBefore) : ""}
                shown={place.pinned && !!current}
                onBack={() => go(place.at)}
                outline={outline}
              />
            )
          )}
        </div>

        {/* Transcript */}
        <div ref={transcript} className="mx-auto w-full max-w-[760px] flex-1 px-4 pt-2 pb-4 md:px-6 md:pt-4">
          {s.error && (
            <Banner tone="error" className="mb-4">
              {s.error}
            </Banner>
          )}
          {!session || (s.connection === "connecting" && !s.items.length) ? (
            <div className="space-y-4 pt-8">
              <Skeleton className="h-8 w-2/3" />
              <Skeleton className="h-4 w-1/2" />
              <Skeleton className="mt-8 h-24 w-full rounded-2xl" />
              <Skeleton className="ml-[34px] h-4 w-1/3" />
              <Skeleton className="ml-[34px] h-4 w-2/5" />
            </div>
          ) : (
            <Reveal.Provider value={finder.reveal}>
              <BusyMark value={DotWave}>
                <TurnTranscript
                  sessionId={id}
                  items={s.items}
                  live={s.live}
                  status={status}
                  stall={stall}
                  disk={disk}
                  canManage={isOwner}
                  startedBy={session.created_by}
                  me={me?.user.id}
                  projectSlug={session.project_slug}
                  onLoadOlder={() => void store.loadOlder()}
                  hasMoreBefore={s.hasMoreBefore}
                  loadingOlder={s.loadingOlder}
                  olderFailed={s.olderFailed}
                  cover={<Cover session={session} workspace={workspace} titleRef={coverTitle} />}
                  quiet={quietNow}
                  newFrom={away.from}
                  diffGroups={diffGroups}
                  onDiff={setDiffAt}
                  resting={now?.kind === "working"}
                />
              </BusyMark>
            </Reveal.Provider>
          )}
        </div>

        {/* The message box, and above it what the agent does now and the way back to the newest work. */}
        <div
          ref={footer}
          className="pointer-events-none sticky -bottom-(--viewport-shift) z-10 bg-gradient-to-t from-canvas via-canvas/95 to-transparent pt-12 pb-[max(1.25rem,env(safe-area-inset-bottom))]"
        >
          <div className="relative mx-auto w-full max-w-[760px] px-4 md:px-6">
            <div className="absolute inset-x-4 -top-10 flex h-8 items-center justify-between gap-2 md:inset-x-6">
              <NowLine now={now} />
              <Pill away={away} atBottom={atBottom} newSince={newSince} letGo={letGo} header={bar} />
            </div>
            <div className="pointer-events-auto">
              {session?.question && !session.review && (
                <QuestionCard
                  key={session.question.asked_at}
                  question={session.question}
                  canAnswer={(role === "editor" || role === "owner") && status !== "failed"}
                  onAnswer={(t) =>
                    store.send(
                      t,
                      me?.user
                        ? { id: me.user.id, name: me.user.name, avatar_url: me.user.avatar_url }
                        : undefined,
                      { attachments: [], previews: {} },
                    )
                  }
                  className="mb-3"
                />
              )}
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
                  choices={
                    isOwner && session ? (
                      <SessionChoices session={session} perTurnEffort={s.perTurnEffort} />
                    ) : undefined
                  }
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
      </div>

      {rail && session && (
        <aside className="sticky top-(--viewport-shift) hidden h-dvh w-[340px] shrink-0 overflow-y-auto border-l border-line bg-surface/40 lg:block">
          <Details
            key={session.id}
            session={session}
            onDiff={setDiffAt}
            role={role}
            review={review}
            me={me?.user.id}
            git={s.git}
            items={s.items}
            viewers={s.viewers}
            model={s.model}
            runnerConnected={s.runnerConnected}
            stall={stall}
            disk={disk}
            turns={
              turns.length > 1 && (
                <RailSection title={`Turns (${turns.length})`}>
                  <TurnList
                    turns={turns}
                    facts={facts}
                    at={place.at}
                    live={!!status && BUSY.has(status)}
                    hasMoreBefore={s.hasMoreBefore}
                    onGo={go}
                  />
                </RailSection>
              )
            }
          />
        </aside>
      )}
      {details && session && (
        <Sheet title={session.title} description={session.description} onClose={() => setDetails(false)}>
          <Overview session={session} workspace={workspace} notes={notes} />
          <Details
            session={session}
            role={role}
            review={review}
            me={me?.user.id}
            git={s.git}
            items={s.items}
            viewers={s.viewers}
            model={s.model}
            runnerConnected={s.runnerConnected}
            stall={stall}
            disk={disk}
            pullsFirst
            onDiff={setDiffAt}
          />
        </Sheet>
      )}
      {diffAt && session && (
        <DiffDialog
          session={session.id}
          cliRef={`${session.project_slug}#${session.number}`}
          groups={diffGroups}
          initial={diffAt}
          onClose={() => setDiffAt(undefined)}
        />
      )}
      {session && <ShareDialog open={share} onClose={() => setShare(false)} session={session} />}
    </div>
  );
}

/**
 * What opens the session, once its start is loaded: where it is, its title and what it is
 * about, and who started it, when, on what branch. The header's title gives way to it while it
 * shows.
 */
function Cover({
  session,
  workspace,
  titleRef,
}: {
  session: G.Session;
  workspace?: G.WorkspaceRef;
  titleRef: RefObject<HTMLDivElement | null>;
}) {
  return (
    <header className="cover pt-6 pb-8 md:pt-10 md:pb-10">
      <div className="flex min-w-0 flex-wrap items-center gap-x-1.5 gap-y-1 text-[12.5px] text-ink-3">
        {workspace && (
          <>
            <Link
              to="/w/$slug"
              params={{ slug: workspace.slug }}
              className="inline-flex min-w-0 items-center gap-1.5 text-ink-2 transition-colors hover:text-ink"
            >
              <WorkspaceAvatar workspace={workspace} size={16} />
              <span className="max-w-[14rem] truncate">{workspace.name}</span>
            </Link>
            <span className="text-ink-4">/</span>
          </>
        )}
        <Link
          to="/p/$slug"
          params={{ slug: session.project_slug }}
          search={{ tab: undefined }}
          className="font-mono text-[12px] text-ink-2 transition-colors hover:text-ink"
        >
          {session.project_slug}#{session.number}
        </Link>
      </div>
      {/* The header's title says it to screen readers. */}
      <div
        ref={titleRef}
        aria-hidden="true"
        className="mt-3 font-display text-[26px] leading-[1.15] font-semibold tracking-[-0.022em] text-balance text-ink md:text-[32px]"
      >
        {session.title}
      </div>
      {session.description && (
        <p className="mt-3 max-w-[62ch] text-[15px] leading-relaxed text-pretty text-ink-2 md:text-[15.5px]">
          {session.description}
        </p>
      )}
      <div className="mt-5 flex flex-wrap items-center gap-x-4 gap-y-2 text-[12.5px] text-ink-3">
        <span className="inline-flex min-w-0 items-center gap-1.5">
          <Avatar user={session.created_by} size={16} />
          <span className="truncate">
            {session.review ? "On " : "Started by "}
            <span className="text-ink-2">{session.created_by.name}</span>
            {session.review && "'s Claude subscription"}
          </span>
        </span>
        <span className="inline-flex items-center gap-1.5" title={stamp(session.created_at)}>
          <Clock className="size-3.5 text-ink-4" />
          {when(session.created_at)}
        </span>
        {session.branch && (
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <GitBranch className="size-3.5 shrink-0 text-ink-4" />
            <span className="truncate font-mono text-[12px] text-ink-2">{session.branch}</span>
            <CopyButton text={session.branch} label="Copy" className="-my-1 py-0.5 max-sm:hidden" />
          </span>
        )}
        <ScheduleLink session={session} />
        <TicketLink session={session} />
        {!session.review && <SessionGithub session={session} />}
      </div>
    </header>
  );
}

/**
 * Each turn's facts, worked out once for each turn as it is: the list of turns asks for them as it
 * opens, and again only for a turn that changed.
 */
function useFactsOf(cwd?: string): (turn: Turn) => TurnFacts {
  const cache = useRef(new WeakMap<object, TurnFacts>());
  return useCallback(
    (turn: Turn) => {
      // A turn is the same while its last part and its end are.
      const last = turn.body[turn.body.length - 1] ?? turn.after[turn.after.length - 1];
      const tip = last ? (last.kind === "run" ? last.steps[last.steps.length - 1] : last.item) : undefined;
      const k = tip ?? turn.ask;
      if (!k) return factsOf(turn, cwd);
      let f = cache.current.get(k);
      if (!f) {
        f = factsOf(turn, cwd);
        cache.current.set(k, f);
      }
      return f;
    },
    [cwd],
  );
}

/**
 * Whether `ref` shows below the header: the cover's title, for the header's to give way to it.
 * Measured as the page scrolls; never while not `enabled`.
 */
function useShows(
  ref: RefObject<HTMLElement | null>,
  header: RefObject<HTMLElement | null>,
  enabled: boolean,
) {
  const [shows, setShows] = useState(false);
  useEffect(() => {
    if (!enabled) {
      setShows(false);
      return;
    }
    let raf = 0;
    const measure = () => {
      raf = 0;
      const el = ref.current;
      const top = header.current?.getBoundingClientRect().bottom ?? 0;
      setShows(!!el && el.getBoundingClientRect().bottom > top + 4);
    };
    const on = () => {
      if (!raf) raf = requestAnimationFrame(measure);
    };
    window.addEventListener("scroll", on, { passive: true });
    window.addEventListener("resize", on, { passive: true });
    measure();
    // Its place changes as the transcript loads.
    const grown = new ResizeObserver(on);
    grown.observe(document.documentElement);
    return () => {
      cancelAnimationFrame(raf);
      window.removeEventListener("scroll", on);
      window.removeEventListener("resize", on);
      grown.disconnect();
    };
  }, [ref, header, enabled]);
  return shows;
}

/**
 * What came while the page was out of sight (another tab, another app): the first part of it,
 * which a line marks until the reader leaves again, and how many of the transcript's items came
 * since. Nothing came: nothing to mark.
 */
function useAway(items: Item[], session: string): { from?: string; count: number } {
  // When the page went out of sight: the newest event then, the last part, and whether the reader
  // is back, after which what comes isn't what came while they were away. Earlier activity that
  // loads meanwhile came before.
  const left = useRef<{ seq: number; part?: string; back: boolean } | undefined>(undefined);
  const [away, setAway] = useState<{ from?: string; count: number }>({ count: 0 });
  const latest = useRef(items);
  latest.current = items;
  useEffect(() => {
    const seen = () => {
      if (document.visibilityState === "hidden") {
        // A message on its way gets another key once it is stored: the part before it.
        const ps = parts(latest.current).filter(
          (p) => !(p.kind === "item" && p.item.kind === "user" && p.item.pending),
        );
        left.current = { seq: newestSeq(latest.current), part: ps[ps.length - 1]?.key, back: false };
        setAway({ count: 0 });
      } else if (left.current) left.current.back = true;
    };
    document.addEventListener("visibilitychange", seen);
    return () => document.removeEventListener("visibilitychange", seen);
  }, []);
  useEffect(() => {
    const l = left.current;
    const count = l && !l.back ? cameAfter(items, l.seq) : 0;
    if (!l || !count) return;
    const ps = parts(items);
    const i = ps.findIndex((p) => p.key === l.part);
    const from = i >= 0 ? ps[i + 1]?.key : undefined;
    setAway((a) => (a.from === from && a.count === count ? a : { from, count }));
  }, [items]);
  // Another session: nothing came while away from it.
  // biome-ignore lint/correctness/useExhaustiveDependencies: another session starts it over
  useEffect(() => {
    left.current = undefined;
    setAway({ count: 0 });
  }, [session]);
  return away;
}

/**
 * How many of the transcript's items came since the reader scrolled up from the bottom: after the
 * newest event then, not the earlier activity that loads as they scroll up.
 */
function useNewSince(items: Item[], atBottom: boolean): number {
  const [left, setLeft] = useState<number>();
  if (atBottom && left !== undefined) setLeft(undefined);
  if (!atBottom && left === undefined) setLeft(newestSeq(items));
  return left === undefined ? 0 : cameAfter(items, left);
}

/**
 * The button back to what the reader hasn't seen: up to where what came while they were away
 * starts, while that is above them, or down to the newest work while they are up the page.
 */
function Pill({
  away,
  atBottom,
  newSince,
  letGo,
  header,
}: {
  away: { from?: string; count: number };
  atBottom: boolean;
  newSince: number;
  letGo: () => void;
  header: RefObject<HTMLElement | null>;
}) {
  // Whether the line that marks what came while away is above the reader: measured once the page
  // has it, and again as the reader scrolls.
  const [above, setAbove] = useState(false);
  useLayoutEffect(() => {
    if (!away.from) {
      setAbove(false);
      return;
    }
    const measure = () => {
      const line = document.querySelector<HTMLElement>(".new-line");
      const top = header.current?.getBoundingClientRect().bottom ?? 0;
      setAbove(!!line && line.getBoundingClientRect().bottom < top);
    };
    measure();
    window.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure, { passive: true });
    return () => {
      window.removeEventListener("scroll", measure);
      window.removeEventListener("resize", measure);
    };
  }, [away.from, header]);
  if (above) {
    return (
      <JumpPill
        up
        count={away.count}
        onClick={() => {
          const line = document.querySelector<HTMLElement>(".new-line");
          if (!line) return;
          letGo();
          const reduce = matchMedia?.("(prefers-reduced-motion: reduce)").matches;
          const top = header.current?.getBoundingClientRect().bottom ?? 0;
          const y = line.getBoundingClientRect().top + window.scrollY - top - 16;
          window.scrollTo({ top: Math.max(0, y), behavior: reduce ? "instant" : "smooth" });
        }}
      />
    );
  }
  if (atBottom) return null;
  return <JumpPill count={newSince} onClick={toBottom} />;
}

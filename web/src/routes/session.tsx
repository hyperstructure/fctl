// A session's page: the agent at work, live. The session reads in turns
// (components/session/turns.tsx), which open with a cover that says what the session is; what the
// agent does now shows above the message box; and long sessions have ways through them: the bar
// that says which turn the reader is in, the list of turns, j and k, the way back down to the
// newest work and a line where what came while the reader was away starts. Beside the transcript,
// the details: the rail where it fits, else a sheet.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import {
  ArrowRightLeft,
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
  Undo2,
  Users,
} from "lucide-react";
import { type RefObject, useCallback, useEffect, useLayoutEffect, useMemo, useRef, useState } from "react";
import { GithubAccessChip, GithubAccessLabel } from "@/components/github-access";
import { ReviewFacts, ReviewFooter, useSessionReview } from "@/components/review";
import { AutopilotLink, AutopilotStarter } from "@/components/session/autopilot";
import { Changes } from "@/components/session/changes";
import { Composer } from "@/components/session/composer";
import { DiffDialog, type DiffGroup, type DiffTarget } from "@/components/session/diff-dialog";
import { FindBar, findShortcut, Reveal, useFind } from "@/components/session/find";
import { Glance } from "@/components/session/glance";
import {
  SessionMemory,
  SessionRetrospectives,
  useRetrospectives,
  useSessionMemory,
} from "@/components/session/kept";
import { JumpPill, NowLine, toBottom, useAtBottom } from "@/components/session/now-line";
import { AgentOrb } from "@/components/session/orb";
import { PlanList } from "@/components/session/plan";
import {
  PullRequestChecksIcon,
  PullRequestReviewLine,
  PullRequestReviewMark,
  PullRequestStateIcon,
} from "@/components/session/pull-request";
import { QuestionCard, useQuestionFloats } from "@/components/session/question";
import { ScheduleLink, TicketLink } from "@/components/session/row";
import { SessionChoices } from "@/components/session/settings";
import { TakeOverDialog } from "@/components/session/take-over";
import {
  TurnBar,
  TurnList,
  TurnOutline,
  useGoToCall,
  useGoToTurn,
  useTurnPlace,
} from "@/components/session/turn-nav";
import { TurnTranscript } from "@/components/session/turns";
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
import { setPageTitle } from "@/lib/attention";
import { cn } from "@/lib/cn";
import { followUpHint } from "@/lib/follow-ups";
import { ago, duration, num, stamp, when } from "@/lib/format";
import { blobLink, branchOf, hasChanges, repoChanges } from "@/lib/git";
import { onGithub, sessionGithub } from "@/lib/github-access";
import { useLiveSession } from "@/lib/live";
import { nowOf, waitingOf } from "@/lib/now";
import { planOf } from "@/lib/plan";
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
import { parts } from "@/lib/runs";
import { ariaShortcut, useShortcut, withShortcut } from "@/lib/shortcuts";
import { type DiskShort, diskOf, diskText, type Stall, stallText, useStall } from "@/lib/stall";
import { type StatusNote, statusNotes, waitsForYou } from "@/lib/status";
import { useStickToBottom } from "@/lib/stick-to-bottom";
import { takeOverOf } from "@/lib/subscription";
import type { Item } from "@/lib/transcript";
import { factsOf, type Turn, type TurnFacts, turnOfSeq, turnSeq, turnsOf, turnTitle } from "@/lib/turns";
import { cameAfter, newestSeq } from "@/lib/unseen";
import { agentTime, type ContextUse, contextNote, contextUse, turnsTaken } from "@/lib/work";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

/**
 * How far back the list of background tasks looks for those that started before the activity
 * loaded here, in pages of it (lib/live.ts), as finding does: 40,000 events, 4,000 at a time.
 */
const LOOK_BACK_PAGES = 100;
const LOOK_BACK_AT_ONCE = 10;

/** Statuses in which the agent is at it, or getting there: the header's edge runs with light. */
const BUSY = new Set<G.SessionStatus>(["running", "provisioning", "preparing"]);

export function SessionPage() {
  const { id } = useParams({ from: "/app/s/$id" });
  // From a search of all sessions: what it found here, and where.
  const { find, at, turn: linked } = useSearch({ from: "/app/s/$id" });
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
  const [moving, setMoving] = useState(false);
  // The diffs shown over the page, at the file opened in Changes (or the first, with c).
  const [diffAt, setDiffAt] = useState<DiffTarget>();
  // Only the conversation: the agent's work and the milestones left out.
  const [quiet, setQuiet] = useState(false);
  // The turn a link to it opens the page at, until the page has gone there.
  const [landTurn, setLandTurn] = useState<{ session: string; seq: number }>();
  // Pages of earlier activity the list of background tasks loaded, looking for those it misses.
  const [lookedBack, setLookedBack] = useState(0);
  // They belong to the session they opened on. The router keeps this page when a link (in the
  // sheet, to the review agent's session) or Back leads to another session, so they close as it
  // comes in: in the same render, without sliding away, which the view transition would show
  // over the next session.
  const [overlaysOf, setOverlaysOf] = useState(id);
  if (overlaysOf !== id) {
    setOverlaysOf(id);
    setDetails(false);
    setShare(false);
    setDiffAt(undefined);
    setQuiet(false);
    setLandTurn(undefined);
    setLookedBack(0);
  }
  const diffGroups = useDiffGroups(session, s.git);
  // c opens the changes at the first file; d shows or hides the details: the rail where it fits,
  // else the sheet.
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
      // With the sheet open, it puts the sheet away as Escape does: it slides away first.
      const sheet = details && document.querySelector<HTMLDialogElement>("dialog.sheet[open]");
      if (sheet) {
        if (sheet.dispatchEvent(new Event("cancel", { cancelable: true }))) sheet.close();
      } else if (window.matchMedia?.("(min-width: 64rem)").matches ?? true) setRail((r) => !r);
      else setDetails(true);
    },
    { inDialogs: "dialog.sheet" },
  );
  const transcript = useRef<HTMLDivElement>(null);
  // What sticks to the top: the header, and the bar under it. Where the header ends, the reader's view starts.
  const header = useRef<HTMLDivElement>(null);
  const bar = useRef<HTMLElement>(null);
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

  // The turns, for the bar and the list of them; the transcript works them out as it shows them.
  const turns = useMemo(() => turnsOf(parts(s.items)), [s.items]);
  // The agent's plan, as it last wrote it.
  const plan = useMemo(() => planOf(s.items), [s.items]);
  // What the agent waits for in the background, while its session waits for that.
  const idleFor = session?.status === "idle" ? session.status_reason : undefined;
  const waiting = useMemo(() => waitingOf(s.tasks, s.items, idleFor), [s.tasks, s.items, idleFor]);
  const facts = useFactsOf(cwd);
  const place = useTurnPlace(transcript, bar);
  const go = useGoToTurn(transcript, bar, letGo);
  const goToCall = useGoToCall(transcript, bar, letGo, finder.reveal, s.items, cwd);
  // A link to a turn: the page goes there once it has the turn, loading earlier activity until it
  // does, and the address forgets it.
  const landed = useRef<string | undefined>(undefined);
  useEffect(() => {
    // Once for each link to a session's turn: the address forgets it as the page goes there,
    // after which the same link goes there again.
    if (!linked) {
      landed.current = undefined;
      return;
    }
    const key = `${id}:${linked}`;
    if (landed.current === key) return;
    landed.current = key;
    setLandTurn({ session: id, seq: linked });
    void navigate({ to: "/s/$id", params: { id }, search: {}, replace: true, viewTransition: false });
  }, [linked, id, navigate]);
  useEffect(() => {
    if (landTurn?.session !== id || !s.items.length) return;
    const { seq } = landTurn;
    // The turn starts before what has loaded: earlier activity, until it has it.
    if (s.hasMoreBefore && s.firstSeq > seq && !s.olderFailed) {
      if (!s.loadingOlder) void store.loadEarlier({ until: seq });
      return;
    }
    setLandTurn(undefined);
    // The turn the event is in, however much of it has loaded since the link was made.
    const i = turnOfSeq(turns, seq);
    if (i >= 0) requestAnimationFrame(() => go(i));
  }, [
    landTurn,
    id,
    turns,
    s.items.length,
    s.hasMoreBefore,
    s.firstSeq,
    s.olderFailed,
    s.loadingOlder,
    store,
    go,
  ]);
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
  // Once the agent hands the turn over while the reader watches, the message box asks for a reply.
  const handedOver = s.session
    ? !s.session.review && !s.session.question && waitsForYou(s.session)
    : undefined;
  const invite = useJustNow(handedOver, id);
  const questionFloats = useQuestionFloats();

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
  // Whether you may move it to your Claude subscription now.
  const takeOver = takeOverOf(session, role, me);
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
    waiting,
  });
  const live = !!status && BUSY.has(status) && !stall;
  // The agent ended its turn and waits for someone to write, who can.
  const yourTurn =
    !!session &&
    !session.review &&
    !session.question &&
    waitsForYou(session) &&
    (role === "editor" || role === "owner");
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
  // The plan the agent works on: written in the turn under way. An older one stays in the
  // details, as it was left, without the step it was at moving.
  const lastTurn = turns[turns.length - 1];
  const lastStart = lastTurn && !lastTurn.end ? turnSeq(lastTurn) : undefined;
  const planNow = plan && lastStart !== undefined && plan.seq >= lastStart ? plan : undefined;
  const railPlan = plan && (
    <RailSection title="Plan">
      <PlanList plan={plan} live={!!planNow && !!status && BUSY.has(status)} />
    </RailSection>
  );
  // A task's call, which the conversation alone leaves out: it shows with the rest again.
  const goToTask = (call: string) => {
    if (quietNow) setQuiet(false);
    goToCall(call);
  };
  const focusReply = () =>
    document.querySelector<HTMLTextAreaElement>('textarea[aria-label="Message the agent"]')?.focus();
  // What the agent asks: above the message box where the window has room for it, else at the
  // transcript's end, where it scrolls away with the rest rather than covering it.
  const question = session?.question && !session.review && (
    <QuestionCard
      key={session.question.asked_at}
      question={session.question}
      canAnswer={(role === "editor" || role === "owner") && status !== "failed"}
      onAnswer={(t) =>
        store.send(
          t,
          me?.user ? { id: me.user.id, name: me.user.name, avatar_url: me.user.avatar_url } : undefined,
          { attachments: [], previews: {} },
        )
      }
      floating={questionFloats}
      className={questionFloats ? "mb-3" : "mt-6"}
    />
  );
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
    <div className="flex min-h-dvh">
      <div className="flex min-w-0 flex-1 flex-col">
        <div ref={header} className="sticky top-(--viewport-shift) z-10">
          <header ref={bar} className="relative z-2 h-14 bg-canvas/80 backdrop-blur-xl md:h-[60px]">
            {/* While the agent works, light runs along the header's edge. */}
            {live && <span aria-hidden="true" className="header-live" />}
            <div className="flex h-full items-center gap-2.5 px-2.5 md:gap-3 md:pr-5 md:pl-5">
              <HeaderMenuButton className="relative z-1 -mr-2.5" />
              <div className="relative flex min-w-0 flex-1 items-center gap-2.5 md:gap-3">
                <AgentOrb status={status} activity={s.activity} className="size-8 md:size-9" />
                <div className="min-w-0 flex-1">
                  {session ? (
                    <>
                      <h1 className="truncate font-display text-[15.5px] leading-5 font-medium tracking-[-0.011em] text-ink md:text-[16.5px]">
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
                  // On phones the header has room for the title and status only: the orb and
                  // title open the rest, with what the rail shows on wider screens, in a sheet.
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
                      {takeOver && (
                        <MenuItem
                          icon={<ArrowRightLeft />}
                          onClick={() => {
                            close();
                            setMoving(true);
                          }}
                        >
                          Continue on my subscription
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
                    </>
                  )}
                </Menu>
              </div>
            </div>
          </header>
          {/* Under the header: finding in the transcript, or the turn the reader is in. */}
          {finder.open ? (
            <div className="absolute inset-x-0 top-full bg-canvas/85 backdrop-blur-xl">
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
                cover={
                  <Cover
                    session={session}
                    workspace={workspace}
                    glance={
                      <Glance
                        session={session}
                        turns={turns}
                        facts={facts}
                        live={!!status && BUSY.has(status)}
                        hasMoreBefore={s.hasMoreBefore}
                        onGo={go}
                      />
                    }
                  />
                }
                quiet={quietNow}
                newFrom={away.from}
                diffGroups={diffGroups}
                onDiff={setDiffAt}
                tasks={s.tasks}
              />
            </Reveal.Provider>
          )}
          {/* Also while the transcript loads: the question is known with the session. */}
          {!questionFloats && question}
        </div>

        {/* The message box, and above it what the agent does now and the way back to the newest work. */}
        <div
          ref={footer}
          className="pointer-events-none sticky -bottom-(--viewport-shift) z-10 bg-gradient-to-t from-canvas via-canvas/95 to-transparent pt-12 pb-[max(1.25rem,env(safe-area-inset-bottom))]"
        >
          <div className="relative mx-auto w-full max-w-[760px] px-4 md:px-6">
            <div className="absolute inset-x-4 -top-10 flex h-8 items-center justify-between gap-2 md:inset-x-6">
              <NowLine
                now={now}
                plan={planNow}
                onReply={yourTurn ? focusReply : undefined}
                autopilot={session?.autopilot}
                background={
                  waiting && {
                    waiting,
                    cwd,
                    earlier: s.hasMoreBefore,
                    looking: s.loadingOlder,
                    failed: s.olderFailed,
                    onLookBack:
                      s.hasMoreBefore && lookedBack < LOOK_BACK_PAGES
                        ? () => {
                            setLookedBack((n) => n + LOOK_BACK_AT_ONCE);
                            void store.loadEarlier({ pages: LOOK_BACK_AT_ONCE });
                          }
                        : undefined,
                    onGo: goToTask,
                  }
                }
              />
              <Pill away={away} atBottom={atBottom} newSince={newSince} letGo={letGo} header={bar} />
            </div>
            <div className={cn("pointer-events-auto", invite && "invite")}>
              {questionFloats && question}
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
                  onTakeOver={takeOver?.waits ? () => setMoving(true) : undefined}
                  limitOf={takeOver?.whose}
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
        <aside className="sticky top-(--viewport-shift) hidden h-dvh w-[340px] shrink-0 overflow-y-auto bg-surface/40 lg:block">
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
            plan={railPlan}
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
            plan={railPlan}
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
      {session && takeOver && (
        <TakeOverDialog
          session={session}
          takeOver={takeOver}
          open={moving}
          onClose={() => setMoving(false)}
        />
      )}
    </div>
  );
}

/**
 * What opens the session, once its start is loaded: where it is, its title and what it is
 * about, and who started it, when, on what branch.
 */
function Cover({
  session,
  workspace,
  glance,
}: {
  session: G.Session;
  workspace?: G.WorkspaceRef;
  /** The session at a glance: its figures, its turns and its pull requests. */
  glance: React.ReactNode;
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
        {session.autopilot ? (
          // Its autopilot's, which acts as the person it runs for.
          <AutopilotStarter session={session} />
        ) : (
          <span className="inline-flex min-w-0 items-center gap-1.5">
            <Avatar user={session.created_by} size={16} />
            <span className="truncate">
              {session.review ? "On " : "Started by "}
              <span className="text-ink-2">{session.created_by.name}</span>
              {session.review && "'s Claude subscription"}
            </span>
          </span>
        )}
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
      {glance}
    </header>
  );
}

/**
 * Whether `on` just came on while the page showed `key` (a session): for a couple of seconds after
 * it does, not when the page opens with it on. Until it is known (`undefined`), it is neither.
 */
function useJustNow(on: boolean | undefined, key: string, ms = 2400): boolean {
  const [state, setState] = useState<{ key: string; on?: boolean; at?: number }>({ key, on });
  if (state.key !== key) setState({ key, on });
  else if (on !== undefined && state.on === undefined) setState({ key, on });
  else if (on !== undefined && state.on !== on) setState({ key, on, at: on ? Date.now() : undefined });
  const at = state.key === key ? state.at : undefined;
  const [, rerender] = useState(0);
  useEffect(() => {
    if (at === undefined) return;
    const t = setTimeout(() => rerender((n) => n + 1), Math.max(0, at + ms - Date.now()));
    return () => clearTimeout(t);
  }, [at, ms]);
  return at !== undefined && Date.now() - at < ms;
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
  // has it, after each render (the conversation alone, finding and earlier activity move it or
  // take it out), and as the reader scrolls or the page changes size.
  const [above, setAbove] = useState(false);
  const measure = useCallback(() => {
    const line = away.from ? document.querySelector<HTMLElement>(".new-line") : null;
    const top = header.current?.getBoundingClientRect().bottom ?? 0;
    setAbove(!!line && line.getBoundingClientRect().bottom < top);
  }, [away.from, header]);
  useLayoutEffect(measure);
  useEffect(() => {
    if (!away.from) return;
    window.addEventListener("scroll", measure, { passive: true });
    window.addEventListener("resize", measure, { passive: true });
    const grown = new ResizeObserver(measure);
    grown.observe(document.documentElement);
    return () => {
      window.removeEventListener("scroll", measure);
      window.removeEventListener("resize", measure);
      grown.disconnect();
    };
  }, [away.from, measure]);
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

/** What the details call the session's primary repository. */
const primaryTitleOf = (project?: G.Project) =>
  project?.repo ? `${project.repo.owner}/${project.repo.name}` : "~/workspace";

/**
 * The changed files of each of the session's repositories, as the diffs over the page show them:
 * the primary repository's, then each other's that has changes.
 */
function useDiffGroups(session: G.Session | undefined, git: G.GitState | undefined): DiffGroup[] {
  // The details' own (useProject), once the session says which.
  const slug = session?.project_slug ?? "";
  const project = useQuery({
    queryKey: keys.project(slug),
    queryFn: () => api.project(slug),
    enabled: !!slug,
  }).data;
  if (!session) return [];
  const [primary, ...others] = repoChanges(
    git ?? { ahead: 0, behind: 0, files: [], uncommitted: 0, pushed: false },
    sessionGithub(session, project) === "push",
  );
  const primaryTitle = primaryTitleOf(project);
  const merged = new Set(
    pullRequests(session)
      .filter((p) => p.state === "merged")
      .map((p) => p.url),
  );
  return [
    {
      title: primaryTitle,
      files: primary?.files ?? [],
      blob: blobLink(project?.repo && primaryTitle, branchOf(git), merged),
    },
    ...others.filter(hasChanges).map((r) => ({
      repo: r.name,
      title: r.name,
      files: r.files,
      blob: blobLink(r.name, branchOf(git, r.name), merged),
    })),
  ].filter((g) => g.files.length > 0);
}

/** The color of the runner's dot: whether it is connected, reaches the agent, and has room on its disk. */
function runnerDot(connected: boolean, stall?: Stall, disk?: DiskShort): string {
  if (stall?.kind === "unreachable" || stall?.kind === "paused") return "bg-amber";
  if (!connected) return "bg-ink-4";
  if (disk) return disk.room === "out" ? "bg-danger" : "bg-amber";
  return "bg-accent";
}

/** The color of a note beside the status pill. */
const noteTone = (n: StatusNote) => (n.danger ? "text-danger" : n.warn ? "text-amber" : "text-ink-3");

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
    <section className="px-5 py-4">
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
 * fctl's review of it stands, which opens the review's session, and what fctl held back of it from
 * the agent, which those who may message the agent pass on.
 */
function PullRequests({
  prs,
  session,
  role,
}: {
  prs: G.PullRequestRef[];
  session: G.Session;
  role?: G.SessionRole;
}) {
  // A failed session takes nothing until it is resumed.
  const passOnTo =
    (role === "editor" || role === "owner") && session.status !== "failed" ? session.id : undefined;
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
            {reviewed && <PullRequestReviewLine pr={pr} review={reviewed} passOnTo={passOnTo} />}
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
    <section className="flex flex-col gap-3 px-5 py-4 text-[12.5px]">
      <div className="flex flex-wrap items-center gap-x-2.5 gap-y-1">
        <StatusPill status={session.status} label={reviewLabel(session)} />
        {notes.map((n) => (
          <span key={n.text} className={noteTone(n)}>
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
  role,
  review,
  me,
  git,
  items,
  viewers,
  model,
  runnerConnected,
  stall,
  disk,
  pullsFirst,
  onDiff,
  turns,
  plan,
}: {
  session: G.Session;
  /** The signed-in person's, in the session. */
  role?: G.SessionRole;
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
  /** How short of disk its runner is. */
  disk?: DiskShort;
  /** The pull requests before the changes, as the sheet has them: on phones they're what people open. */
  pullsFirst?: boolean;
  /** Opens the diffs over the page, at a file of the changes. */
  onDiff: (at: DiffTarget) => void;
  /** The list of turns, after the pull requests. */
  turns?: React.ReactNode;
  /** The agent's plan, first. */
  plan?: React.ReactNode;
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
  const primaryTitle = primaryTitleOf(project);
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
      <PullRequests prs={prs} session={session} role={role} />
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
      {plan}
      {pullsFirst && pulls}
      <RailSection title={session.review ? "Changes reviewed" : "Changes"}>
        {primary && others.length === 0 ? (
          <Changes repo={primary} showTotals={!!git} onOpen={(path) => onDiff({ path })} />
        ) : (
          <div className="flex flex-col gap-5">
            {primary && (hasChanges(primary) || changed.length === 0) && (
              <Changes
                repo={primary}
                title={primaryTitle}
                showTotals={hasChanges(primary)}
                onOpen={(path) => onDiff({ path })}
              />
            )}
            {changed.map((r) => (
              <Changes
                key={r.name}
                repo={r}
                title={r.name}
                showTotals
                onOpen={(path) => onDiff({ repo: r.name, path })}
              />
            ))}
          </div>
        )}
      </RailSection>
      {!pullsFirst && pulls}
      {turns}
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
        {session.autopilot && (
          <Row k="Autopilot" v={<AutopilotLink session={session} className="justify-end" />} />
        )}
        {session.base_branch && <Row k="From" v={<span className="font-mono">{session.base_branch}</span>} />}
        {/* What its agent says, also after a switch, which no new `init` says until the next turn. */}
        {(session.model ?? model) && <Row k="Model" v={session.model ?? model} />}
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
              <span
                className="inline-flex items-center gap-1.5"
                title={stall ? stallText(stall).title : disk && diskText(disk).title}
              >
                <span className={cn("size-1.5 rounded-full", runnerDot(runnerConnected, stall, disk))} />
                {session.runner.name}
              </span>
            }
          />
        )}
        <Row
          k="Billing"
          v={
            session.claude
              ? session.claude.id === me
                ? "Your Claude subscription"
                : `${session.claude.name}'s Claude subscription`
              : "Claude subscription"
          }
        />
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

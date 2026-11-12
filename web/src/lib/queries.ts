// React Query hooks. Keys are stable arrays so live updates can patch caches.
import { QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type * as G from "@/gen";
import { ApiFailure, api, workspaces } from "./api";
import { clearLog } from "./app-log";
import { FAILED_FOR } from "./needs";
import { withPendingProjects, withPendingWorkspaces } from "./order";
import { forgetSidebar } from "./sidebar";

/** Whether a failed query goes again: up to 3 times, unless the server refused it (4xx). */
export function retries(count: number, err: unknown): boolean {
  return !(err instanceof ApiFailure && err.status >= 400 && err.status < 500) && count < 3;
}

/**
 * Whether a check the app makes as it starts goes again: once if the server didn't answer in
 * time, as each try takes that long; otherwise as any query does.
 */
export function startRetries(count: number, err: unknown): boolean {
  return err instanceof ApiFailure && err.timedOut ? count < 1 : retries(count, err);
}

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      retry: retries,
      refetchOnWindowFocus: true,
    },
  },
});

export const keys = {
  instance: ["instance"] as const,
  me: ["me"] as const,
  projects: ["projects"] as const,
  project: (id: string) => ["project", id] as const,
  sessions: (q: object) => ["sessions", q] as const,
  session: (id: string) => ["session", id] as const,
  /** A search of all sessions: what was typed, and how many sessions it lists. */
  search: (q: string, limit: number) => ["search", q, limit] as const,
  members: (id: string) => ["members", id] as const,
  secrets: (id: string) => ["secrets", id] as const,
  projectRepos: (id: string) => ["project-repos", id] as const,
  /** A project's memory, by project id; a session's notes under it. */
  memories: (project: string) => ["memories", project] as const,
  sessionMemories: (project: string, session: string) => ["memories", project, "session", session] as const,
  /** A project's retrospectives, by project id: all its lists. */
  projectRetrospectives: (project: string) => ["retrospectives", project] as const,
  /** A project's retrospectives, by project id, of one outcome or all. */
  retrospectives: (project: string, outcome?: string) =>
    ["retrospectives", project, outcome ?? "all"] as const,
  sessionRetrospectives: (id: string) => ["session-retrospectives", id] as const,
  /** A changed file's diff, by session, repository ("" for the primary one), path and digest. */
  fileDiff: (session: string, repo: string, path: string, digest: string) =>
    ["file-diff", session, repo, path, digest] as const,
  /** All of a session's diffs at once, which fill the cache of each file's (`fileDiff`). */
  sessionDiff: (session: string) => ["session-diff", session] as const,
  /** A project's review agents, by project id. */
  reviewAgents: (project: string) => ["review-agents", project] as const,
  /** The apps that opened a project's newest pull requests, and one looked up by its bot, by project id. */
  reviewBots: (project: string) => ["review-bots", project] as const,
  reviewBot: (project: string, login: string) => ["review-bot", project, login] as const,
  /** A project's reviews, by project id: the list, and each session's under it. */
  reviews: (project: string) => ["reviews", project] as const,
  sessionReview: (project: string, session: string) => ["reviews", project, "session", session] as const,
  /** A project's autopilot, by project id or slug, and its log under it. */
  autopilot: (project: string) => ["autopilot", project] as const,
  autopilotLog: (project: string) => ["autopilot", project, "log"] as const,
  /** What autopilots ask you, across your workspaces. */
  autopilotAsks: ["autopilot-asks"] as const,
  /** A project's schedules, by project id. */
  schedules: (project: string) => ["schedules", project] as const,
  /** A schedule's runs, by schedule id. */
  scheduleRuns: (schedule: string) => ["schedule-runs", schedule] as const,
  /** A project's ticket tracker as it works now, by project id. */
  projectTickets: (project: string) => ["project-tickets", project] as const,
  /** Your Linear accounts. */
  myLinear: ["me", "linear"] as const,
  runners: ["runners"] as const,
  users: ["users"] as const,
  allowlist: ["allowlist"] as const,
  accessRequests: ["admin", "access-requests"] as const,
  tokens: ["tokens"] as const,
  github: ["github"] as const,
  repos: (workspace: string) => ["repos", workspace] as const,
  workspaces: ["workspaces"] as const,
  workspace: (id: string) => ["workspace", id] as const,
  workspaceMembers: (id: string) => ["workspace", id, "members"] as const,
  workspaceRunners: (id: string) => ["workspace", id, "runners"] as const,
  enrollmentTokens: (id: string) => ["workspace", id, "enrollment-tokens"] as const,
  workspaceGithub: (id: string) => ["workspace", id, "github"] as const,
  /** Under the workspace's GitHub, so that what changes its accounts asks again. */
  workspaceReviewApp: (id: string) => ["workspace", id, "github", "review-app"] as const,
  workspaceLinear: (id: string) => ["workspace", id, "linear"] as const,
  /** The teams fctl's app reaches in a Linear workspace a workspace works in. */
  linearTeams: (id: string, organization: string) =>
    ["workspace", id, "linear", organization, "teams"] as const,
  workspaceAudit: (id: string) => ["workspace", id, "audit"] as const,
  adminWorkspaces: ["admin", "workspaces"] as const,
  adminGithub: ["admin", "github"] as const,
  shares: (id: string) => ["shares", id] as const,
  audit: ["audit"] as const,
  instanceSettings: ["admin", "settings"] as const,
  reservedSlugs: ["admin", "reserved-slugs"] as const,
  accountDeletion: ["me", "deletion"] as const,
  notifications: ["me", "notifications"] as const,
};

export const useInstance = () =>
  useQuery({ queryKey: keys.instance, queryFn: () => api.instance(), staleTime: 60_000 });

/**
 * You, or null when you aren't signed in: then this browser forgets the sidebar it kept for you,
 * and the app's log, which has the pages you were on.
 * The server answers within `within` ms, if given.
 */
export async function fetchMe(within?: number): Promise<G.Me | null> {
  try {
    return await api.me(within);
  } catch (e) {
    if (e instanceof ApiFailure && (e.status === 401 || e.status === 403)) {
      forgetSidebar();
      clearLog();
      return null;
    }
    throw e;
  }
}

export const useMe = () => useQuery({ queryKey: keys.me, queryFn: () => fetchMe(), staleTime: 60_000 });

/** Your projects, in your order: under your workspaces in theirs. */
export const useProjects = () =>
  useQuery({ queryKey: keys.projects, queryFn: () => api.projects().then(withPendingProjects) });
/** Your workspaces, in your order, and the invitations waiting for you. */
export const useWorkspaces = () =>
  useQuery({
    queryKey: keys.workspaces,
    queryFn: () => workspaces.list().then(withPendingWorkspaces),
    staleTime: 30_000,
  });
/** One of your workspaces, by id or slug. */
export const useWorkspace = (id: string | undefined) =>
  useQuery({
    queryKey: keys.workspace(id ?? ""),
    queryFn: () => workspaces.get(id ?? ""),
    enabled: !!id,
  });
export const useProject = (id: string) =>
  useQuery({ queryKey: keys.project(id), queryFn: () => api.project(id) });
export const useProjectRepos = (id?: string) =>
  useQuery({
    queryKey: keys.projectRepos(id ?? ""),
    queryFn: () => api.projectRepos(id ?? ""),
    enabled: !!id,
  });
/** A project's memory, by project id. */
export const useMemories = (project?: string) =>
  useQuery({
    queryKey: keys.memories(project ?? ""),
    queryFn: () => api.memories(project ?? ""),
    enabled: !!project,
  });
export const useSessions = (
  q: {
    project?: string;
    workspace?: string;
    status?: string;
    limit?: number;
    mine?: boolean;
  },
  { enabled = true }: { enabled?: boolean } = {},
) => useQuery({ queryKey: keys.sessions(q), queryFn: () => api.sessions(q), enabled });
/** How long the sidebar goes on listing a session after it ends. */
export const RECENT_MS = 60 * 60_000;
// The sidebar's sessions, under one key whatever the time it asks from.
const SIDEBAR_SESSIONS = { since: "1h", limit: 100 };
/**
 * The sessions the sidebar lists under their projects (not review agents'),
 * newest first: those that haven't ended, and those that ended in the last
 * hour (`recentSessions` keeps those still in it). Until they come, those
 * the lists in the cache have.
 */
export const useSidebarSessions = () => {
  const qc = useQueryClient();
  return useQuery({
    queryKey: keys.sessions(SIDEBAR_SESSIONS),
    queryFn: () => api.sessions({ activeSince: Date.now() - RECENT_MS, limit: SIDEBAR_SESSIONS.limit }),
    placeholderData: () => ({ sessions: cachedSessions(qc) }),
  });
};
const YOUR_SESSIONS = { yours: true, since: "1d", limit: 200 };
/**
 * The sessions the person started or follows that haven't ended or ended in the last day, which
 * the sidebar's Needs you looks through, under one key whatever the time it asks from.
 */
export const useYourSessions = (enabled: boolean) =>
  useQuery({
    queryKey: keys.sessions(YOUR_SESSIONS),
    queryFn: () =>
      api.sessions({ yours: true, activeSince: Date.now() - FAILED_FOR, limit: YOUR_SESSIONS.limit }),
    enabled,
  });
/**
 * The sessions in `sessions` that haven't ended, or ended in the hour before
 * `now`, and aren't archived: the feed marks one that ends or is archived
 * before the list comes again.
 */
export const recentSessions = (sessions: G.Session[], now: number) =>
  sessions.filter((s) => !s.archived && (!ended(s) || (s.ended_at != null && s.ended_at >= now - RECENT_MS)));
/** A changed file of a session: in which repository (`owner/name`, none for the primary one), and how. */
export interface ChangedFile {
  repo?: string;
  file: G.FileChange;
}

/** The options of the query for a changed file's diff, for `useFileDiff` and for loading it ahead. */
export const fileDiffQuery = (session: string, f: ChangedFile) => ({
  queryKey: keys.fileDiff(session, f.repo ?? "", f.file.path, f.file.digest ?? ""),
  queryFn: () => api.fileDiff(session, f.file.path, f.repo),
  // A digest's diff never changes.
  staleTime: Number.POSITIVE_INFINITY,
  gcTime: 10 * 60_000,
});

/**
 * All of a session's diffs, in one request, for the diff dialog that shows
 * them all: each goes into its file's cache (`fileDiffQuery`), from which
 * `useFileDiff` takes it without asking again. Its own data is only when it
 * came, for the dialog to know.
 */
export function useSessionDiffs(session: string, enabled = true) {
  const qc = useQueryClient();
  return useQuery({
    queryKey: keys.sessionDiff(session),
    enabled,
    queryFn: async () => {
      const all = await api.sessionDiff(session);
      for (const d of all.files) {
        const key = keys.fileDiff(session, d.repo ?? "", d.path, d.digest);
        if (qc.getQueryData(key) === undefined) qc.setQueryData(key, d);
      }
      return Date.now();
    },
    // Each file's diff then follows its digest, one at a time.
    staleTime: Number.POSITIVE_INFINITY,
    gcTime: 0,
    retry: 1,
  });
}

/**
 * A changed file's diff, for agents that send them (the file has a digest).
 * The agent sends a diff before the state that lists it, but the session
 * may still have the one before: until the digests agree, it asks again.
 * `wait`: not yet, while all of them come at once (`useSessionDiffs`).
 */
export const useFileDiff = (session: string, f: ChangedFile | undefined, wait = false) => {
  const digest = f?.file.digest;
  return useQuery({
    ...fileDiffQuery(session, f ?? { file: { path: "", status: "" } }),
    enabled: !!f && !!digest && !wait,
    retry: (count, err) => (err instanceof ApiFailure && err.status === 404 ? count < 4 : count < 2),
    retryDelay: (count) => 600 * 2 ** count,
    refetchInterval: (q) => {
      const d = q.state.data;
      return d && d.digest !== digest && q.state.dataUpdateCount < 8 ? 1500 : false;
    },
    // The same file, with its diff before, until the new one comes.
    placeholderData: (previous, previousQuery) =>
      previousQuery?.queryKey[3] === f?.file.path && previousQuery?.queryKey[2] === (f?.repo ?? "")
        ? previous
        : undefined,
  });
};

export const useRunners = () =>
  useQuery({ queryKey: keys.runners, queryFn: api.runners, refetchInterval: 30_000 });

/** Whether a session has ended: stopped or failed. */
const ended = (s: G.Session) => s.status === "stopped" || s.status === "failed";

/** The sessions the lists in the cache hold, each once, newest first. */
function cachedSessions(qc: QueryClient): G.Session[] {
  const byId = new Map<string, G.Session>();
  for (const [, list] of qc.getQueriesData<G.SessionList>({ queryKey: ["sessions"] }))
    for (const s of list?.sessions ?? []) if (!byId.has(s.id)) byId.set(s.id, s);
  return [...byId.values()].sort((a, b) => b.created_at - a.created_at);
}

/**
 * A cached summary with a newer one over it. The newer one leaves out what is empty (the server
 * skips `None`), and the feed's the caller's role too, so the cached one fills those in, except a
 * question: one the newer summary doesn't have was answered or taken back, maybe with no change of
 * status that would refetch the lists.
 */
function merged(old: G.Session, s: G.Session): G.Session {
  const next = { ...old, ...s, role: old.role ?? s.role };
  if (s.question === undefined) delete next.question;
  return next;
}

/** After a session summary changes, patch every cached list that holds it. */
export function patchSession(qc: QueryClient, s: G.Session) {
  qc.setQueryData<G.Session>(keys.session(s.id), (old) => (old ? merged(old, s) : old));
  qc.setQueriesData<G.SessionList>({ queryKey: ["sessions"] }, (old) => {
    if (!old) return old;
    const i = old.sessions.findIndex((x) => x.id === s.id);
    const was = old.sessions[i];
    if (!was) return old;
    const sessions = old.sessions.slice();
    sessions[i] = merged(was, s);
    return { ...old, sessions };
  });
}

/**
 * After a session's summary changes, its project's last activity in the cached list of projects,
 * which is its sessions' latest (review agents' left out).
 */
export function patchProjectActivity(qc: QueryClient, s: G.Session) {
  if (s.review) return;
  const at = s.last_activity_at ?? s.created_at;
  qc.setQueryData<G.ProjectList>(keys.projects, (old) => {
    const i = old?.projects.findIndex((p) => p.id === s.project_id && (p.last_activity_at ?? 0) < at) ?? -1;
    if (!old || i === -1) return old;
    const projects = old.projects.slice();
    projects[i] = { ...old.projects[i], last_activity_at: at } as G.Project;
    return { ...old, projects };
  });
}

export function useInvalidate() {
  const qc = useQueryClient();
  return (...ks: readonly unknown[][]) => Promise.all(ks.map((k) => qc.invalidateQueries({ queryKey: k })));
}

export { useMutation, useQuery, useQueryClient };

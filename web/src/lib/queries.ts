// React Query hooks. Keys are stable arrays so live updates can patch caches.
import { QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type * as G from "@/gen";
import { ApiFailure, api, workspaces } from "./api";
import { withPendingProjects, withPendingWorkspaces } from "./order";
import { forgetSidebar } from "./sidebar";

export const queryClient = new QueryClient({
  defaultOptions: {
    queries: {
      staleTime: 15_000,
      retry: (count, err) =>
        !(err instanceof ApiFailure && err.status >= 400 && err.status < 500) && count < 3,
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
  /** A project's review agents, by project id. */
  reviewAgents: (project: string) => ["review-agents", project] as const,
  /** A project's reviews, by project id: the list, and each session's under it. */
  reviews: (project: string) => ["reviews", project] as const,
  sessionReview: (project: string, session: string) => ["reviews", project, "session", session] as const,
  /** A project's schedules, by project id. */
  schedules: (project: string) => ["schedules", project] as const,
  /** A schedule's runs, by schedule id. */
  scheduleRuns: (schedule: string) => ["schedule-runs", schedule] as const,
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
  workspaceGithub: (id: string) => ["workspace", id, "github"] as const,
  workspaceAudit: (id: string) => ["workspace", id, "audit"] as const,
  adminWorkspaces: ["admin", "workspaces"] as const,
  adminGithub: ["admin", "github"] as const,
  shares: (id: string) => ["shares", id] as const,
  audit: ["audit"] as const,
  instanceSettings: ["admin", "settings"] as const,
  accountDeletion: ["me", "deletion"] as const,
  notifications: ["me", "notifications"] as const,
};

export const useInstance = () =>
  useQuery({ queryKey: keys.instance, queryFn: api.instance, staleTime: 60_000 });

/** You, or null when you aren't signed in: then this browser forgets the sidebar it kept for you. */
export async function fetchMe(): Promise<G.Me | null> {
  try {
    return await api.me();
  } catch (e) {
    if (e instanceof ApiFailure && (e.status === 401 || e.status === 403)) {
      forgetSidebar();
      return null;
    }
    throw e;
  }
}

export const useMe = () => useQuery({ queryKey: keys.me, queryFn: fetchMe, staleTime: 60_000 });

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
export const useSessions = (q: {
  project?: string;
  workspace?: string;
  status?: string;
  limit?: number;
  mine?: boolean;
}) => useQuery({ queryKey: keys.sessions(q), queryFn: () => api.sessions(q) });
/**
 * A project's sessions that haven't ended (not review agents'), newest
 * first, by project id: the sidebar lists them under it on its page and
 * its sessions'. Until they come, those the lists in the cache have.
 */
export const useLiveSessions = (project: string | undefined) => {
  const qc = useQueryClient();
  const q = { project, status: "active", limit: 50 };
  return useQuery({
    queryKey: keys.sessions(q),
    queryFn: () => api.sessions(q),
    enabled: !!project,
    placeholderData: () =>
      project ? { sessions: cachedSessions(qc).filter((s) => s.project_id === project) } : undefined,
    select: unended,
  });
};
/**
 * A session's summary, by id, for what shows around its page: as a list
 * has it until its own comes, so that a session opened from a list is
 * known at once.
 */
export const useSessionSummary = (id: string | undefined) => {
  const qc = useQueryClient();
  return useQuery({
    queryKey: keys.session(id ?? ""),
    queryFn: () => api.session(id ?? ""),
    enabled: !!id,
    placeholderData: () => (id ? cachedSessions(qc).find((s) => s.id === id) : undefined),
  });
};
export const useRunners = () =>
  useQuery({ queryKey: keys.runners, queryFn: api.runners, refetchInterval: 30_000 });

/** The sessions of a list that haven't ended: the feed marks one that ends before the list comes again. */
const unended = (list: G.SessionList) =>
  list.sessions.filter((s) => s.status !== "stopped" && s.status !== "failed");

/** The sessions the lists in the cache hold, each once, newest first. */
function cachedSessions(qc: QueryClient): G.Session[] {
  const byId = new Map<string, G.Session>();
  for (const [, list] of qc.getQueriesData<G.SessionList>({ queryKey: ["sessions"] }))
    for (const s of list?.sessions ?? []) if (!byId.has(s.id)) byId.set(s.id, s);
  return [...byId.values()].sort((a, b) => b.created_at - a.created_at);
}

/** After a session summary changes, patch every cached list that holds it. */
export function patchSession(qc: QueryClient, s: G.Session) {
  qc.setQueryData<G.Session>(keys.session(s.id), (old) =>
    old ? { ...old, ...s, role: old.role ?? s.role } : old,
  );
  qc.setQueriesData<G.SessionList>({ queryKey: ["sessions"] }, (old) => {
    if (!old) return old;
    const i = old.sessions.findIndex((x) => x.id === s.id);
    if (i === -1) return old;
    const sessions = old.sessions.slice();
    sessions[i] = { ...sessions[i], ...s, role: sessions[i]?.role ?? s.role } as G.Session;
    return { ...old, sessions };
  });
}

export function useInvalidate() {
  const qc = useQueryClient();
  return (...ks: readonly unknown[][]) => Promise.all(ks.map((k) => qc.invalidateQueries({ queryKey: k })));
}

export { useMutation, useQuery, useQueryClient };

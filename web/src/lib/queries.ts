// React Query hooks. Keys are stable arrays so live updates can patch caches.
import { QueryClient, useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import type * as G from "@/gen";
import { ApiFailure, api } from "./api";

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
  runners: ["runners"] as const,
  users: ["users"] as const,
  allowlist: ["allowlist"] as const,
  tokens: ["tokens"] as const,
  github: ["github"] as const,
  repos: ["repos"] as const,
  shares: (id: string) => ["shares", id] as const,
  audit: ["audit"] as const,
  accountDeletion: ["me", "deletion"] as const,
  notifications: ["me", "notifications"] as const,
};

export const useInstance = () =>
  useQuery({ queryKey: keys.instance, queryFn: api.instance, staleTime: 60_000 });

export function useMe() {
  return useQuery({
    queryKey: keys.me,
    queryFn: async () => {
      try {
        return await api.me();
      } catch (e) {
        if (e instanceof ApiFailure && (e.status === 401 || e.status === 403)) return null;
        throw e;
      }
    },
    staleTime: 60_000,
  });
}

export const useProjects = () => useQuery({ queryKey: keys.projects, queryFn: api.projects });
export const useProject = (id: string) =>
  useQuery({ queryKey: keys.project(id), queryFn: () => api.project(id) });
export const useProjectRepos = (id?: string) =>
  useQuery({
    queryKey: keys.projectRepos(id ?? ""),
    queryFn: () => api.projectRepos(id ?? ""),
    enabled: !!id,
  });
export const useSessions = (q: { project?: string; status?: string; limit?: number; mine?: boolean }) =>
  useQuery({ queryKey: keys.sessions(q), queryFn: () => api.sessions(q) });
export const useRunners = () =>
  useQuery({ queryKey: keys.runners, queryFn: api.runners, refetchInterval: 30_000 });

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

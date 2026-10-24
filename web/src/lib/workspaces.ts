// Workspaces in the app: finding a project's or session's workspace as the
// person's list has it now (names and avatars change), grouping projects
// under their workspaces, and putting both in the order the person wants.

import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type * as G from "@/gen";
import { workspaces as workspacesApi } from "./api";
import { pending, withPendingProjects, withPendingWorkspaces } from "./order";
import { keys, queryClient, useMe, useWorkspaces } from "./queries";

/** Whether you create projects in `w`: its admins do, while it is open to them (two-factor sign-in). */
export function createsProjects(w: Pick<G.Workspace, "role" | "blocked">): boolean {
  return (w.role === "owner" || w.role === "admin") && !w.blocked;
}

/** Your workspaces, against how many you can be in. */
type Counted = Pick<G.WorkspaceList, "limit" | "unlimited"> & { workspaces: readonly unknown[] };

/** Whether you can create or join another workspace: the primary instance owner always can. */
export const roomForAnother = (l: Counted): boolean => !!l.unlimited || l.workspaces.length < l.limit;

/** Why you can't create or join another workspace, if you can't (fctl_domain::workspace::check_limit). */
export function limitReached(l: Counted): string | undefined {
  if (roomForAnother(l)) return undefined;
  const n = l.workspaces.length;
  return n === l.limit
    ? `You are in ${n} workspaces, the most you can be. Leave one first (Settings → Workspaces).`
    : `You are in ${n} workspaces, and you can be in ${l.limit} at most. Leave ${n + 1 - l.limit} first (Settings → Workspaces).`;
}

/** How many workspaces you are in, of how many you can be in. */
export function workspaceCount(l: Counted): string {
  const n = l.workspaces.length;
  if (l.unlimited)
    return `You are in ${n === 1 ? "1 workspace" : `${n} workspaces`}. As the primary instance owner, you can be in any number.`;
  return n > l.limit
    ? `You are in ${n} workspaces, and you can be in ${l.limit} at most.`
    : `You are in ${n} of the ${l.limit} workspaces you can be in.`;
}

/** Your workspaces by id, with the reference something carries as a fallback. */
export function useWorkspaceIndex() {
  const { data } = useWorkspaces();
  return useMemo(() => {
    const byId = new Map<string, G.Workspace>();
    for (const w of data?.workspaces ?? []) byId.set(w.id, w);
    return byId;
  }, [data]);
}

/** The freshest reference to `ref`'s workspace: from your list when it has it. */
export function freshest(
  index: Map<string, G.Workspace>,
  ref: G.WorkspaceRef | undefined,
): G.WorkspaceRef | undefined {
  if (!ref) return undefined;
  return index.get(ref.id) ?? ref;
}

export interface WorkspaceGroup<W = G.Workspace, P = G.Project> {
  workspace: W;
  projects: P[];
}

/** Projects under their workspaces, both in your order. */
export function groupProjects<W extends { id: string }, P extends { workspace?: { id: string } }>(
  workspaces: readonly W[],
  projects: readonly P[],
): WorkspaceGroup<W, P>[] {
  return workspaces.map((workspace) => ({
    workspace,
    projects: projects.filter((p) => p.workspace?.id === workspace.id),
  }));
}

// New orders go to the control plane one at a time, in the order they were
// made; a newer one for the same list takes the place of one still waiting.
interface Save {
  put: () => Promise<unknown>;
  settle: (error?: unknown) => void;
}
const waiting = new Map<string, Save>();
let saving = false;

async function send() {
  if (saving) return;
  saving = true;
  for (let next = waiting.entries().next(); !next.done; next = waiting.entries().next()) {
    const [list, save] = next.value;
    waiting.delete(list);
    try {
      await save.put();
      save.settle();
    } catch (e) {
      save.settle(e);
    }
  }
  saving = false;
}

/**
 * Puts your workspaces in `order` (ids): lists show it at once, and keep
 * it while it is saved. `failed` hears when it wasn't saved; then they show
 * the order the control plane has.
 */
export function reorderWorkspaces(order: string[], failed: (e: unknown) => void) {
  pending.setWorkspaces(order);
  queryClient.setQueryData<G.WorkspaceList>(keys.workspaces, (old) => old && withPendingWorkspaces(old));
  queryClient.setQueryData<G.ProjectList>(keys.projects, (old) => old && withPendingProjects(old));
  waiting.set("workspaces", {
    put: () => workspacesApi.order(order),
    settle: (error) => {
      if (pending.workspaces() !== order) return; // a newer order is on its way
      pending.setWorkspaces(undefined);
      // Fetched again, dropping what was fetched before it was saved.
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      if (error) failed(error);
    },
  });
  void send();
}

/** Puts a workspace's projects in `order` (ids), as `reorderWorkspaces` does your workspaces. */
export function reorderProjects(workspace: string, order: string[], failed: (e: unknown) => void) {
  pending.setProjects(workspace, order);
  queryClient.setQueryData<G.ProjectList>(keys.projects, (old) => old && withPendingProjects(old));
  waiting.set(`projects:${workspace}`, {
    put: () => workspacesApi.updateMembership(workspace, { project_order: order }),
    settle: (error) => {
      if (pending.projects(workspace) !== order) return;
      pending.setProjects(workspace, undefined);
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      if (error) failed(error);
    },
  });
  void send();
}

// Which workspaces are folded in the sidebar, remembered in this browser.
const FOLDED = "fctl:folded-workspaces";
const listeners = new Set<() => void>();

function readFolded(): string {
  try {
    return localStorage.getItem(FOLDED) ?? "[]";
  } catch {
    return "[]";
  }
}

export function useFolded(): [Set<string>, (id: string) => void] {
  const raw = useSyncExternalStore(
    (fn) => {
      listeners.add(fn);
      window.addEventListener("storage", fn);
      return () => {
        listeners.delete(fn);
        window.removeEventListener("storage", fn);
      };
    },
    readFolded,
    () => "[]",
  );
  const folded = useMemo(() => {
    try {
      return new Set<string>(JSON.parse(raw) as string[]);
    } catch {
      return new Set<string>();
    }
  }, [raw]);
  const toggle = useCallback(
    (id: string) => {
      const next = new Set(folded);
      if (next.has(id)) next.delete(id);
      else next.add(id);
      try {
        localStorage.setItem(FOLDED, JSON.stringify([...next]));
      } catch {
        // Private browsing without storage: folding lasts the page.
      }
      for (const l of listeners) l();
    },
    [folded],
  );
  return [folded, toggle];
}

/**
 * How many runners of a list run your sessions in a workspace, and how many of those are online: the
 * ones that serve it, but not those another member added for their own sessions, and only yours
 * while you keep your sessions there on your own runners (`own_runners`). `others`: the ones that
 * serve it but don't run your sessions. The list is what someone sees: every runner for instance
 * admins, those of their workspaces for others. Undefined when it can't tell: control planes before
 * workspaces don't say whose runners are whose.
 */
export function runnersOf(
  runners: readonly G.Runner[],
  w: Pick<G.Workspace, "id" | "own_runners">,
): { total: number; online: number; others: number } | undefined {
  if (runners.some((r) => !r.workspaces)) return undefined;
  const serving = runners.filter((r) => r.workspaces?.some((x) => x.id === w.id));
  const mine = serving.filter((r) => runsMine(r, w));
  return {
    total: mine.length,
    online: mine.filter((r) => r.status === "online").length,
    others: serving.length - mine.length,
  };
}

/** Whether a runner of the workspace runs your sessions there (`fctl_domain::schedule::runs_for`). */
export function runsMine(r: G.Runner, w: Pick<G.Workspace, "id" | "own_runners">): boolean {
  return r.yours || (!w.own_runners && !r.personal?.includes(w.id));
}

/** "Primary admin", "Admin", "Member". */
export function roleLabel(role: G.WorkspaceRole): string {
  return role === "owner" ? "Primary admin" : role === "admin" ? "Admin" : "Member";
}

/** What a workspace's base role lets its members do, in a few words. */
export const BASE_ROLES: { value: G.BaseRole; label: string; description: string }[] = [
  { value: "none", label: "No base role", description: "Members see the projects they are members of" },
  { value: "viewer", label: "Viewer", description: "Members see every project's sessions" },
  {
    value: "editor",
    label: "Editor",
    description: "Members start sessions and message agents in every project",
  },
  {
    value: "maintainer",
    label: "Maintainer",
    description: "Members also change every project's settings, secrets and members",
  },
];

/**
 * Workspaces that require two-factor sign-in open or close as your second
 * factors change: fetch them again then (the feed says so too, when it is
 * connected).
 */
export function useFactorsWatch() {
  const { data: me } = useMe();
  const factors = me ? `${me.user.two_factor}:${me.user.passkeys}` : undefined;
  const last = useRef(factors);
  useEffect(() => {
    if (last.current !== undefined && factors !== undefined && last.current !== factors) {
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      void queryClient.invalidateQueries({ queryKey: ["workspace"] });
    }
    last.current = factors;
  }, [factors]);
}

/** How a Claude connection reads in lists: its account, else its label. */
export const claudeName = (c: G.ClaudeCredentialInfo) =>
  c.account_email ?? c.label ?? (c.on_runners ? "Token on your runners" : "Claude");

/**
 * The Claude connections someone's sessions in a workspace can run on:
 * their default, then each working one, and the one chosen there even
 * when it stopped working, so the picker says so instead of showing an id.
 * `prefix` goes before each label (in a row of other things).
 */
export function claudeOptions(
  me: G.Me | null | undefined,
  current: string | undefined,
  prefix = "",
): { value: string; label: string; description?: string }[] {
  const fallback = me?.claude.find((c) => c.is_default);
  const options: { value: string; label: string; description?: string }[] = [
    {
      value: "default",
      // Sentence case after a prefix: "Claude: your default".
      label: `${prefix}${prefix ? "your default" : "Your default"}${fallback ? ` (${claudeName(fallback)})` : ""}`,
      description: "The one Settings → Claude marks as default",
    },
  ];
  for (const c of me?.claude ?? []) {
    if (c.status === "active")
      options.push({
        value: c.id,
        label: `${prefix}${claudeName(c)}`,
        description: c.on_runners
          ? "Kept on your runners: sessions run only on yours that keep it"
          : (c.subscription_type ?? c.kind),
      });
    else if (c.id === current)
      options.push({
        value: c.id,
        label: `${prefix}${claudeName(c)} (reconnect it)`,
        description: "It stopped working: sessions here run on your default until you reconnect it",
      });
  }
  if (current && !options.some((o) => o.value === current))
    options.push({
      value: current,
      label: `${prefix}A connection you removed`,
      description: "Sessions here run on your default",
    });
  return options;
}

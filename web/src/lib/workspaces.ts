// Workspaces in the app: finding a project's or session's workspace as the
// person's list has it now (names and avatars change), and grouping projects
// under their workspaces in the order the person joined them.

import { useCallback, useEffect, useMemo, useRef, useSyncExternalStore } from "react";
import type * as G from "@/gen";
import { keys, queryClient, useMe, useWorkspaces } from "./queries";

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

export interface WorkspaceGroup {
  workspace: G.Workspace;
  projects: G.Project[];
}

/** Projects under their workspaces, the workspaces in your order (oldest membership first). */
export function groupProjects(
  workspaces: readonly G.Workspace[],
  projects: readonly G.Project[],
): WorkspaceGroup[] {
  return workspaces.map((workspace) => ({
    workspace,
    projects: projects.filter((p) => p.workspace?.id === workspace.id),
  }));
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
 * How many runners of a list serve a workspace, and how many of those are online. The list is
 * what someone sees: every runner for instance admins, those of their workspaces for others.
 * Undefined when it can't tell: control planes before workspaces don't say whose runners are whose.
 */
export function runnersOf(
  runners: readonly G.Runner[],
  workspace: string,
): { total: number; online: number } | undefined {
  if (runners.some((r) => !r.workspaces)) return undefined;
  const serving = runners.filter((r) => r.workspaces?.some((w) => w.id === workspace));
  return { total: serving.length, online: serving.filter((r) => r.status === "online").length };
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
export const claudeName = (c: G.ClaudeCredentialInfo) => c.account_email ?? c.label ?? "Claude";

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
        description: c.subscription_type ?? c.kind,
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

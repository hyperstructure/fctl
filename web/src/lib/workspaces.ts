// Workspaces in the app: finding a project's or session's workspace as the
// person's list has it now (names and avatars change), and grouping projects
// under their workspaces in the order the person joined them.

import { useCallback, useMemo, useSyncExternalStore } from "react";
import type * as G from "@/gen";
import { useWorkspaces } from "./queries";

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

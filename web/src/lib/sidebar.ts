// The sidebar's workspaces and projects, kept in this browser as it last
// showed them: when the app opens, the sidebar shows them at once, and the
// lists the control plane sends take their place once both have come. Only
// what the sidebar shows is kept (names, colors and avatars, your role, how
// many sessions each project has going), for the person signed in; signing
// out forgets it.

import { useEffect, useMemo } from "react";
import type * as G from "@/gen";

const KEY = "fctl:sidebar";
// The shape of what is kept (`Kept`): a copy in another one is not read.
const VERSION = 1;

/** A workspace as the sidebar shows it. */
export type SidebarWorkspace = Pick<
  G.Workspace,
  "id" | "slug" | "name" | "color" | "avatar_url" | "role" | "blocked"
>;
/** A project as the sidebar shows it, under its workspace. */
export type SidebarProject = Pick<G.Project, "id" | "slug" | "name" | "active_sessions"> & {
  workspace?: Pick<G.WorkspaceRef, "id">;
};
/** An invitation to a workspace, as the sidebar shows it. */
export type SidebarInvitation = Pick<G.WorkspaceInvitation, "id" | "workspace">;

/** Your workspaces, the invitations waiting for you, and how many workspaces you may be in. */
export interface SidebarWorkspaces extends Pick<G.WorkspaceList, "limit" | "unlimited"> {
  workspaces: SidebarWorkspace[];
  invitations: SidebarInvitation[];
}

/** What the sidebar shows. */
export interface SidebarLists {
  workspaces?: SidebarWorkspaces;
  projects?: SidebarProject[];
  /** They are the lists kept from before: the control plane's haven't both come yet. */
  kept: boolean;
}

interface Kept {
  version: number;
  user: string;
  workspaces: SidebarWorkspaces;
  projects: SidebarProject[];
}

const ref = (w: G.WorkspaceRef): G.WorkspaceRef => ({
  id: w.id,
  slug: w.slug,
  name: w.name,
  color: w.color,
  avatar_url: w.avatar_url,
});

/** Keeps what the sidebar shows of `user`'s lists. */
export function keepSidebar(user: string, workspaces: G.WorkspaceList, projects: G.ProjectList) {
  const kept: Kept = {
    version: VERSION,
    user,
    workspaces: {
      workspaces: workspaces.workspaces.map((w) => ({ ...ref(w), role: w.role, blocked: w.blocked })),
      invitations: workspaces.invitations.map((i) => ({ id: i.id, workspace: ref(i.workspace) })),
      limit: workspaces.limit,
      unlimited: workspaces.unlimited,
    },
    projects: projects.projects.map((p) => ({
      id: p.id,
      slug: p.slug,
      name: p.name,
      active_sessions: p.active_sessions,
      workspace: p.workspace && { id: p.workspace.id },
    })),
  };
  try {
    localStorage.setItem(KEY, JSON.stringify(kept));
  } catch {
    // Private browsing without storage, or no room: the sidebar waits for the lists.
  }
}

/** Forgets the lists kept (signing out). */
export function forgetSidebar() {
  try {
    localStorage.removeItem(KEY);
  } catch {}
}

const isObject = (v: unknown): v is Record<string, unknown> => typeof v === "object" && v !== null;
const isText = (v: unknown): v is string => typeof v === "string";

function readRef(v: unknown): G.WorkspaceRef | undefined {
  if (!isObject(v) || !isText(v.id) || !isText(v.slug) || !isText(v.name) || !isText(v.color)) return;
  return {
    id: v.id,
    slug: v.slug,
    name: v.name,
    color: v.color as G.WorkspaceColor,
    avatar_url: isText(v.avatar_url) ? v.avatar_url : undefined,
  };
}

function readWorkspace(v: unknown): SidebarWorkspace | undefined {
  const r = readRef(v);
  if (!r || !isObject(v) || !isText(v.role) || typeof v.blocked !== "boolean") return;
  return { ...r, role: v.role as G.WorkspaceRole, blocked: v.blocked };
}

function readInvitation(v: unknown): SidebarInvitation | undefined {
  if (!isObject(v) || !isText(v.id)) return;
  const workspace = readRef(v.workspace);
  return workspace && { id: v.id, workspace };
}

function readProject(v: unknown): SidebarProject | undefined {
  if (!isObject(v) || !isText(v.id) || !isText(v.slug) || !isText(v.name)) return;
  if (typeof v.active_sessions !== "number") return;
  const w = v.workspace;
  if (w !== undefined && !(isObject(w) && isText(w.id))) return;
  return {
    id: v.id,
    slug: v.slug,
    name: v.name,
    active_sessions: v.active_sessions,
    workspace: isObject(w) && isText(w.id) ? { id: w.id } : undefined,
  };
}

/** Each item of `v`, read by `read`: none unless `v` is a list whose every item reads. */
function readAll<T>(v: unknown, read: (item: unknown) => T | undefined): T[] | undefined {
  if (!Array.isArray(v)) return;
  const items: T[] = [];
  for (const item of v) {
    const got = read(item);
    if (!got) return;
    items.push(got);
  }
  return items;
}

/** The lists kept for `user`, unless there are none or they can't be read. */
export function keptSidebar(user: string): Omit<SidebarLists, "kept"> | undefined {
  let d: unknown;
  try {
    d = JSON.parse(localStorage.getItem(KEY) ?? "null");
  } catch {
    return;
  }
  if (!isObject(d) || d.version !== VERSION || d.user !== user || !isObject(d.workspaces)) return;
  const workspaces = readAll(d.workspaces.workspaces, readWorkspace);
  const invitations = readAll(d.workspaces.invitations, readInvitation);
  const projects = readAll(d.projects, readProject);
  const { limit, unlimited } = d.workspaces;
  if (!workspaces || !invitations || !projects || typeof limit !== "number") return;
  // Absent from lists kept before the primary instance owner could be in any number.
  if (unlimited !== undefined && typeof unlimited !== "boolean") return;
  return { workspaces: { workspaces, invitations, limit, unlimited }, projects };
}

/**
 * What the sidebar shows of `user`'s lists: the control plane's once both
 * have come, those kept from before until then, and without any kept, each
 * as it comes. It keeps the control plane's, as they change.
 */
export function useSidebarLists(
  user: string | undefined,
  workspaces: G.WorkspaceList | undefined,
  projects: G.ProjectList | undefined,
): SidebarLists {
  const kept = useMemo(() => (user ? keptSidebar(user) : undefined), [user]);
  useEffect(() => {
    if (user && workspaces && projects) keepSidebar(user, workspaces, projects);
  }, [user, workspaces, projects]);
  if (workspaces && projects) return { workspaces, projects: projects.projects, kept: false };
  if (kept) return { ...kept, kept: true };
  return { workspaces, projects: projects?.projects, kept: false };
}

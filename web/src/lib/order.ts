// The order people put their workspaces in, and each one's projects. The
// control plane keeps it, and lists come in it. Until a new order is saved,
// lists fetched meanwhile keep it, so nothing jumps back.

import type * as G from "@/gen";

/** `items` in `order` (ids): the ones it names first, in its order, then the others as they were. */
export function arrange<T>(items: readonly T[], order: readonly string[], id: (item: T) => string): T[] {
  const at = new Map(order.map((x, i) => [x, i]));
  return items
    .map((item, i) => ({ item, i, at: at.get(id(item)) ?? order.length }))
    .sort((a, b) => a.at - b.at || a.i - b.i)
    .map((x) => x.item);
}

/** Projects with the ones of `workspace` in `order`, in the places they had in the list. */
export function arrangeIn(
  projects: readonly G.Project[],
  workspace: string,
  order: readonly string[],
): G.Project[] {
  const theirs = arrange(
    projects.filter((p) => p.workspace?.id === workspace),
    order,
    (p) => p.id,
  );
  let i = 0;
  return projects.map((p) => (p.workspace?.id === workspace ? (theirs[i++] ?? p) : p));
}

/** Projects under their workspaces in `order` (ids), each one's in the order they had. */
export function byWorkspace(projects: readonly G.Project[], order: readonly string[]): G.Project[] {
  return arrange(projects, order, (p) => p.workspace?.id ?? "");
}

// The orders on their way to the control plane: your workspaces', and each workspace's projects'.
let workspaceOrder: readonly string[] | undefined;
const projectOrders = new Map<string, readonly string[]>();

export const pending = {
  workspaces: () => workspaceOrder,
  projects: (workspace: string) => projectOrders.get(workspace),
  setWorkspaces(order: readonly string[] | undefined) {
    workspaceOrder = order;
  },
  setProjects(workspace: string, order: readonly string[] | undefined) {
    if (order) projectOrders.set(workspace, order);
    else projectOrders.delete(workspace);
  },
};

/** Your workspaces as the control plane sent them, in the order being saved. */
export function withPendingWorkspaces(list: G.WorkspaceList): G.WorkspaceList {
  return workspaceOrder
    ? { ...list, workspaces: arrange(list.workspaces, workspaceOrder, (w) => w.id) }
    : list;
}

/** Projects as the control plane sent them, in the orders being saved. */
export function withPendingProjects(list: G.ProjectList): G.ProjectList {
  if (!workspaceOrder && !projectOrders.size) return list;
  let projects = list.projects;
  for (const [workspace, order] of projectOrders) projects = arrangeIn(projects, workspace, order);
  if (workspaceOrder) projects = byWorkspace(projects, workspaceOrder);
  return { ...list, projects };
}

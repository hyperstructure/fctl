// Claude connections whose token their owner keeps on their runners: which
// connection a workspace's sessions run on, and where each of your runners
// stands with the token (`fctl host claude set-token`).

import type * as G from "@/gen";

/**
 * The Claude connection your sessions in `w` run on, as the control plane picks it
 * (`api::sessions::credential_for`): the working one you chose there, else your default, else your
 * newest working one.
 */
export function connectionIn(
  me: Pick<G.Me, "claude"> | null | undefined,
  w: Pick<G.Workspace, "claude"> | undefined,
): G.ClaudeCredentialInfo | undefined {
  const working = (me?.claude ?? []).filter((c) => c.status === "active");
  const chosen = w?.claude ? working.find((c) => c.id === w.claude) : undefined;
  const newest = [...working].sort((a, b) => b.created_at - a.created_at)[0];
  return chosen ?? working.find((c) => c.is_default) ?? newest;
}

/** Whether your sessions in `w` run on the token you keep on your runners. */
export function tokenOnRunnersIn(
  me: Pick<G.Me, "claude"> | null | undefined,
  w: Pick<G.Workspace, "claude"> | undefined,
): boolean {
  return connectionIn(me, w)?.on_runners === true;
}

/**
 * Where a runner of yours stands with your Claude token: it keeps it (`kept`), keeps none (`none`),
 * or runs an fctl too old to keep one (`old`, which says nothing about it).
 */
export type TokenState = "kept" | "none" | "old";

export function tokenState(r: Pick<G.Runner, "claude_token">): TokenState {
  if (!r.claude_token) return "old";
  return r.claude_token.saved_at ? "kept" : "none";
}

/** A runner of yours, with where it stands with your Claude token. */
export interface RunnerToken {
  runner: G.Runner;
  state: TokenState;
}

/**
 * Your runners with where each stands with your Claude token: those that keep it first, then the
 * ones that could, then those too old to; online before offline within each, then by name.
 */
export function yourRunnerTokens(runners: readonly G.Runner[]): RunnerToken[] {
  const rank: Record<TokenState, number> = { kept: 0, none: 1, old: 2 };
  return runners
    .filter((r) => r.yours)
    .map((runner) => ({ runner, state: tokenState(runner) }))
    .sort(
      (a, b) =>
        rank[a.state] - rank[b.state] ||
        Number(b.runner.status === "online") - Number(a.runner.status === "online") ||
        a.runner.name.localeCompare(b.runner.name),
    );
}

/**
 * How many of your runners run your sessions in workspace `w` when you keep your Claude token on
 * them: `here`, yours that serve it; `kept`, those of them that keep the token; `online`, those of
 * them that are online too (what sessions start on).
 */
export function tokenRunnersIn(
  runners: readonly G.Runner[],
  w: Pick<G.Workspace, "id">,
): { here: number; kept: number; online: number } {
  const here = runners.filter((r) => r.yours && r.workspaces?.some((x) => x.id === w.id));
  const kept = here.filter((r) => tokenState(r) === "kept");
  return {
    here: here.length,
    kept: kept.length,
    online: kept.filter((r) => r.status === "online").length,
  };
}

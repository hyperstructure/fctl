// Keeping your Claude token out of sandboxes: Claude gets a stand-in that Anthropic refuses, and the
// runner's host puts your token in as Claude's requests leave for the Anthropic API, so that agents
// can neither read it nor send it anywhere (`Me.claude_stand_in`).

import { useMutation } from "@tanstack/react-query";
import { Switch, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { keys, queryClient } from "@/lib/queries";

export const STAND_IN_LABEL = "Keep my token out of sandboxes";

/** What the switch's row says under it, as it stands. */
export function standInHint(me: G.Me): string {
  if (!me.claude_stand_in) {
    return "Off: your token is in each of your sessions' sandboxes, where their agents can read it.";
  }
  const login = me.claude.some((c) => c.kind === "oauth" && !c.on_runners);
  return [
    "Claude gets a stand-in that Anthropic refuses. The runner puts your token in as each of Claude's requests leaves for the Anthropic API, so agents can neither read it nor send it anywhere.",
    "It applies to sessions that start after you change it.",
    login ? "Remote Control and claude.ai connectors may not work while it's on." : "",
  ]
    .filter(Boolean)
    .join(" ");
}

export function KeepTokenOut({ me }: { me: G.Me }) {
  const set = useMutation({
    mutationFn: (on: boolean) => api.updateMe({ claude_stand_in: on }),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      toast(
        m.claude_stand_in
          ? "New sessions keep your Claude token out of their sandboxes"
          : "New sessions get your Claude token in their sandboxes",
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <ul className="divide-y divide-line border-y border-line">
      <li className="flex items-center justify-between gap-4 py-2.5">
        <div className="min-w-0">
          <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[13.5px] text-ink">
            {STAND_IN_LABEL}
            <span className="rounded-full bg-raised px-1.5 py-0.5 text-[10.5px] text-ink-3">
              Experimental
            </span>
          </div>
          <div className="text-[12px] text-ink-4">{standInHint(me)}</div>
        </div>
        <Switch
          checked={me.claude_stand_in}
          onChange={(on) => set.mutate(on)}
          label={STAND_IN_LABEL}
          disabled={set.isPending}
        />
      </li>
    </ul>
  );
}

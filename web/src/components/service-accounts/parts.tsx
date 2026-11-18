// What the service accounts' pages share: how one shows where it stands (its glyph and pill), how a
// person's answer shows, and the control that changes yours.

import { useMutation } from "@tanstack/react-query";
import { TriangleAlert, UserRoundCog } from "lucide-react";
import { useId } from "react";
import { Button, Switch, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, queryClient } from "@/lib/queries";
import type { AccountState } from "@/lib/service-accounts";

/** A service account's symbol: an account that works on its own, amber with a warning once it stopped. */
export function AccountGlyph({
  state,
  size = 32,
  className,
}: {
  state?: AccountState;
  size?: 32 | 40;
  className?: string;
}) {
  const stopped = state === "stopped";
  return (
    <span
      aria-hidden="true"
      className={cn(
        "relative mt-px flex shrink-0 items-center justify-center rounded-lg bg-raised shadow-[inset_0_0_0_1px_var(--c-line)]",
        size === 40 ? "size-10 [&_svg]:size-5" : "size-8 [&_svg]:size-4",
        stopped ? "text-amber" : "text-ink",
        className,
      )}
    >
      {stopped ? <TriangleAlert /> : <UserRoundCog />}
      {state === "used" && (
        <span className="absolute -top-0.5 -right-0.5 size-2 rounded-full bg-checks-passed ring-2 ring-canvas" />
      )}
    </span>
  );
}

/** How a service account stands, in a word beside its name, when it isn't plainly at work. */
export function StatePill({ state }: { state: AccountState }) {
  if (state === "used") return null;
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center rounded-full px-2 text-[11.5px] font-medium",
        state === "stopped" ? "bg-amber-soft text-amber" : "bg-raised text-ink-3",
      )}
    >
      {state === "stopped" ? "Stopped" : "Not used yet"}
    </span>
  );
}

/** What a change to service accounts touches: the lists that hold them, the account, the workspaces. */
export function refreshServiceAccounts(id?: string) {
  void queryClient.invalidateQueries({ queryKey: ["service-accounts"] });
  void queryClient.invalidateQueries({ queryKey: keys.workspaces });
  void queryClient.invalidateQueries({ queryKey: ["workspace"] });
  if (id) void queryClient.invalidateQueries({ queryKey: keys.serviceAccount(id) });
}

/** Answering a service account: allowing it, or not, with what happened in a toast. */
export function useAnswer(account: Pick<G.ServiceAccount, "id" | "name">) {
  return useMutation({
    mutationFn: (allow: boolean) => api.answerServiceAccount(account.id, allow),
    onSuccess: (a, allow) => {
      refreshServiceAccounts(a.id);
      toast(
        allow
          ? `${a.name} may start sessions for you now. Take it back any time, in ${a.workspace.name}'s Service accounts.`
          : `${a.name} won't start sessions for you`,
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
}

/**
 * Your answer to a service account, which you change: two buttons while you haven't answered, then a
 * switch. Where the instance doesn't ask people, it says so instead.
 */
export function AnswerControl({
  account: a,
  size = "sm",
}: {
  account: G.ServiceAccount;
  size?: "sm" | "md";
}) {
  const answer = useAnswer(a);
  const id = useId();
  if (!a.consent) return <span className="text-[12.5px] text-ink-3">It acts for everyone here</span>;
  if (!a.answer)
    return (
      <div className="flex shrink-0 items-center gap-2">
        <Button
          size={size}
          variant="ghost"
          loading={answer.isPending && answer.variables === false}
          disabled={answer.isPending}
          onClick={() => answer.mutate(false)}
        >
          Don't allow
        </Button>
        <Button
          size={size}
          variant="primary"
          loading={answer.isPending && answer.variables === true}
          disabled={answer.isPending}
          onClick={() => answer.mutate(true)}
        >
          Allow
        </Button>
      </div>
    );
  const on = answer.isPending ? answer.variables === true : a.answer === "allowed";
  return (
    <div className="flex shrink-0 items-center gap-2.5 text-[12.5px]">
      <label htmlFor={id} className={cn("cursor-pointer", on ? "text-ink-2" : "text-ink-4")}>
        {on ? "Allowed" : "Not allowed"}
      </label>
      <Switch
        id={id}
        checked={on}
        disabled={answer.isPending}
        onChange={(v) => answer.mutate(v)}
        label={`Let ${a.name} start sessions for you`}
      />
    </div>
  );
}

/** What allowing a service account lets it do, and what it can't, for the people it asks. */
export function allowingMeans(workspace: string): { key: string; text: string }[] {
  return [
    {
      key: "yours",
      text: `It starts sessions for you in ${workspace}'s projects you work in: they're yours, on your Claude subscription, and you follow them as you follow your own.`,
    },
    {
      key: "steer",
      text: "It messages, interrupts and stops their agents for you, and reads what they do.",
    },
    {
      key: "not",
      text: "Nothing else: not your account, your settings or other workspaces. Take it back whenever you like.",
    },
  ];
}

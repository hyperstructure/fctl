// What a service account asks the people of its workspace: to start sessions for them. On the Agent
// Floor, a card per account that hasn't heard their answer, with what allowing it means and the two
// answers; on an invitation, what joining the workspace means for them, before they join.

import { Link } from "@tanstack/react-router";
import { Check, UserRoundCog } from "lucide-react";
import { Avatar } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { ago } from "@/lib/format";
import type { Ask } from "@/lib/service-accounts";
import { AnswerControl, allowingMeans } from "./parts";

/** The service accounts that ask you, on the Agent Floor: each until you answer. */
export function ServiceAccountAsks({ asks }: { asks: Ask[] }) {
  if (!asks.length) return null;
  return (
    <section aria-label="Service accounts that ask you" className="flex animate-rise flex-col gap-2">
      {asks.map((a) => (
        <AskCard key={a.account.id} ask={a} />
      ))}
    </section>
  );
}

function AskCard({ ask: { workspace: w, account: a } }: { ask: Ask }) {
  return (
    <div
      // As an invitation's: narrower than 36rem inside, the buttons take a line of their own.
      className="@container rounded-xl border border-blue/25 bg-blue-soft px-4 py-3.5"
    >
      <div className="flex flex-col gap-3.5 @xl:flex-row @xl:items-start @xl:gap-4">
        <div className="flex min-w-0 flex-1 items-start gap-3 @xl:gap-4">
          <span className="relative mt-0.5 shrink-0">
            <WorkspaceAvatar workspace={w} size={36} />
            <span
              aria-hidden="true"
              className="absolute -right-1.5 -bottom-1.5 flex size-5 items-center justify-center rounded-full bg-raised text-blue shadow-[0_0_0_2px_var(--c-canvas)]"
            >
              <UserRoundCog className="size-3" />
            </span>
          </span>
          <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
            <div className="text-[14.5px] text-ink">
              <span className="font-medium">{a.name}</span> asks to start agents for you in{" "}
              <Link
                to="/w/$slug"
                params={{ slug: w.slug }}
                search={{ tab: "service-accounts" }}
                className="font-medium underline-offset-2 hover:underline"
              >
                {w.name}
              </Link>
            </div>
            {a.description && (
              <p className="mt-1 text-[13px] leading-relaxed text-ink-2">“{a.description}”</p>
            )}
            <ul className="mt-2 flex flex-col gap-1 text-[12.5px] leading-relaxed text-ink-3">
              {allowingMeans(w.name).map((m) => (
                <li key={m.key} className="flex items-start gap-1.5">
                  <Check className="mt-[3px] size-3 shrink-0 text-ink-4" aria-hidden="true" />
                  <span>{m.text}</span>
                </li>
              ))}
            </ul>
            <div className="mt-2 flex flex-wrap items-center gap-x-1.5 text-[12px] text-ink-4">
              <span>A service account of {w.name}'s</span>
              {a.created_by && (
                <>
                  <span aria-hidden="true">·</span>
                  <span className="inline-flex items-center gap-1">
                    made by <Avatar user={a.created_by} size={14} /> {a.created_by.name}
                  </span>
                </>
              )}
              <span aria-hidden="true">·</span>
              <span className="whitespace-nowrap">{ago(a.created_at)}</span>
            </div>
          </div>
        </div>
        <div className="flex shrink-0 justify-end @xl:pt-0.5">
          <AnswerControl account={a} />
        </div>
      </div>
    </div>
  );
}

/**
 * What joining a workspace means for you, where it has service accounts: said on its invitation,
 * before you join. Where the instance asks people, you choose once you're in; where it doesn't, they
 * start sessions for you from the start.
 */
export function InvitationServiceAccounts({
  invitation: i,
  consent,
}: {
  invitation: G.WorkspaceInvitation;
  consent: boolean;
}) {
  const list = i.service_accounts ?? [];
  if (!list.length) return null;
  const names = list.map((s) => s.name);
  const named =
    names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
  const one = list.length === 1;
  return (
    <p className="mt-2 flex items-start gap-1.5 text-[12.5px] leading-relaxed text-ink-2">
      <UserRoundCog className="mt-[3px] size-3.5 shrink-0 text-ink-3" aria-hidden="true" />
      <span>
        {i.workspace.name} has {one ? "a service account" : "service accounts"}, {named}:{" "}
        {one ? "a program" : "programs"} of its own that {one ? "starts" : "start"} agents for its people, on
        their Claude subscription.{" "}
        {consent
          ? `${one ? "It starts" : "They start"} them for you only if you allow ${one ? "it" : "them"}: fctl asks you once you join.`
          : `This instance doesn't ask people: once you join, ${one ? "it starts" : "they start"} them for you too.`}
      </span>
    </p>
  );
}

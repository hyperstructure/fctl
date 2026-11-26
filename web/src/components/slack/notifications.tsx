// Settings → Notifications, in Slack: "Tell me in Slack", where they would go (each Slack workspace
// your workspaces work in, with your account there or the link to it, and whether its admins let
// notifications go there), and the direct message one comes as.

import { Check, TriangleAlert } from "lucide-react";
import { useId } from "react";
import { SwitchRow } from "@/components/settings/form";
import { SlackIcon, toast } from "@/components/ui";
import type * as G from "@/gen";
import { clickConfirmed, slackLinkUrl } from "@/lib/api";
import { cn } from "@/lib/cn";
import { useInstance } from "@/lib/queries";
import { SlackDmPreview } from "./preview";

/** "Acme and Side project", "Acme, Side project and Ops". */
function names(ws: G.SlackWorkspaceRef[]): string {
  const n = ws.map((w) => w.name);
  if (n.length <= 1) return n[0] ?? "";
  return `${n.slice(0, -1).join(", ")} and ${n[n.length - 1]}`;
}

/** Whether any of the places would get them now: linked there, and allowed. */
export const slackReaches = (places: G.SlackNotifyPlace[]) => places.some((p) => p.account && p.allowed);

export function SlackNotifications({
  n,
  onChange,
}: {
  n: G.Notifications;
  onChange: (b: G.UpdateNotificationsRequest) => void;
}) {
  const app = useInstance().data?.name ?? "fctl";
  const heading = useId();
  const places = n.slack;
  // Absent where the instance has no Slack app.
  if (!places) return null;
  const on = n.settings.slack;
  const reaches = slackReaches(places);
  return (
    // Its own width lays it out: the preview beside the switch where both fit.
    <section aria-labelledby={heading} className="@container">
      <h3 id={heading} className="flex items-center gap-2 text-[13.5px] font-medium text-ink">
        <SlackIcon className="size-3.5" />
        In Slack
      </h3>
      <div className="mt-2 grid items-start gap-x-8 gap-y-5 @xl:grid-cols-[minmax(0,1fr)_290px]">
        <div className="min-w-0">
          <div className="border-y border-line">
            <SwitchRow
              label="Tell me in Slack"
              checked={on}
              onChange={(v) => onChange({ slack: v })}
              hint={
                on
                  ? places.length === 0
                    ? "Your notifications come from fctl's app in Slack too, once one of your workspaces works in Slack."
                    : reaches
                      ? "The same notifications come to you as direct messages from fctl's app, where you linked your account, with the agent's question and its answers to click."
                      : "Nothing goes there until you link your account in a Slack workspace whose admins let notifications go there."
                  : "Off: your notifications come only to your browsers."
              }
            />
          </div>
          {places.length > 0 && (
            <ul className={cn("mt-3 flex flex-col gap-2", !on && "opacity-60")} aria-label="Where they go">
              {places.map((p) => (
                <Place key={p.team} p={p} />
              ))}
            </ul>
          )}
          {places.length === 0 && (
            <p className="mt-3 text-[12.5px] leading-relaxed text-ink-4">
              None of your workspaces works in Slack yet: its admins connect one under its Integrations.
            </p>
          )}
        </div>
        <SlackDmPreview app={app} className={cn("max-w-sm transition-opacity", !on && "opacity-60")} />
      </div>
    </section>
  );
}

/** A Slack workspace they'd come to: as whom, or the link to link your account; or why none comes. */
function Place({ p }: { p: G.SlackNotifyPlace }) {
  const href = slackLinkUrl({ team: p.team });
  return (
    <li className="flex gap-2.5 text-[12.5px] leading-relaxed">
      {p.account && p.allowed ? (
        <Check aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-accent" />
      ) : (
        <TriangleAlert aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-amber" />
      )}
      <span className="min-w-0 text-ink-3">
        {!p.allowed ? (
          <>
            None in <span className="font-medium text-ink-2">{p.team_name}</span>: {names(p.workspaces)}'s
            admins keep notifications out of Slack.
          </>
        ) : p.account ? (
          <>
            In <span className="font-medium text-ink-2">{p.team_name}</span>, to you as {p.account.name},
            about sessions in {names(p.workspaces)}.
          </>
        ) : (
          <>
            In <span className="font-medium text-ink-2">{p.team_name}</span>, about sessions in{" "}
            {names(p.workspaces)}, once fctl knows your account there.{" "}
            <a
              href={href}
              onClick={(e) => clickConfirmed(e, href)?.catch((err: Error) => toast(err.message, "error"))}
              className="text-ink-2 underline underline-offset-2 hover:text-ink"
            >
              Link your account
            </a>
          </>
        )}
      </span>
    </li>
  );
}

// Admin → Slack, for the instance's admins: the instance's Slack app as it stands (its credentials,
// how Slack's events reach it, Socket Mode's connection), how to create it from its manifest, the
// addresses Slack calls, the Slack workspaces it's in, and whether workspaces may know people by
// their verified address in Slack.

import { useMutation } from "@tanstack/react-query";
import { Check, CircleAlert, Minus } from "lucide-react";
import type { ReactNode } from "react";
import { SwitchRow, SwitchRows } from "@/components/settings/form";
import { Banner, CodeBox, Section, Skeleton, SlackIcon, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, queryClient, useInstance } from "@/lib/queries";
import { agoAfterVerb } from "@/lib/slack";
import { ManifestBox, SlackAppSetup, useAdminSlack } from "./setup";

/** A fact about the app: done, missing (amber, where it holds things up), or neither. */
function Fact({ state, children }: { state: "ok" | "warn" | "off"; children: ReactNode }) {
  return (
    <li className="flex gap-2.5 text-[13px] leading-relaxed">
      {state === "ok" ? (
        <Check aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-accent" />
      ) : state === "warn" ? (
        <CircleAlert aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-amber" />
      ) : (
        <Minus aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-ink-4" />
      )}
      <span
        className={cn("min-w-0 [overflow-wrap:anywhere]", state === "warn" ? "text-ink-2" : "text-ink-3")}
      >
        {children}
      </span>
    </li>
  );
}

/** How Slack's events reach the instance, as facts. */
function EventFacts({ a }: { a: G.AdminSlack }) {
  const facts: ReactNode[] = [];
  if (a.signing_secret)
    facts.push(
      <Fact key="http" state={a.reachable ? "ok" : "warn"}>
        {a.reachable
          ? "Slack's events, button clicks and commands come over HTTP, signed with SLACK_SIGNING_SECRET."
          : "SLACK_SIGNING_SECRET is set, but Slack can't reach this instance's address: its events come only over Socket Mode."}
      </Fact>,
    );
  if (a.socket_mode)
    facts.push(
      a.socket_connected ? (
        <Fact key="socket" state="ok">
          Socket Mode {a.socket_since ? `connected ${agoAfterVerb(a.socket_since)}` : "is connected"}: Slack
          sends its events over it.
        </Fact>
      ) : (
        <Fact key="socket" state="warn">
          Socket Mode isn't connected{a.socket_error ? `: ${a.socket_error}` : ""}. What people do in Slack
          waits until it is; fctl connects again by itself. Check SLACK_APP_TOKEN has connections:write.
        </Fact>
      ),
    );
  if (!a.signing_secret && !a.socket_mode)
    facts.push(
      <Fact key="none" state="warn">
        Nothing people do in Slack reaches fctl yet: set SLACK_SIGNING_SECRET, or SLACK_APP_TOKEN for Socket
        Mode.
      </Fact>,
    );
  return <>{facts}</>;
}

export function AdminSlackTab() {
  const admin = useAdminSlack();
  const name = useInstance().data?.name ?? "fctl";
  const save = useMutation({
    mutationFn: (on: boolean) => api.updateInstanceSettings({ slack_match_email: on }),
    onSuccess: (s) => {
      queryClient.setQueryData(keys.instanceSettings, s);
      void queryClient.invalidateQueries({ queryKey: keys.adminSlack });
      // Workspaces' Slack pages say whether their admins may turn it on.
      void queryClient.invalidateQueries({ queryKey: ["workspace"] });
      toast(
        s.slack_match_email
          ? "Workspaces' admins may let fctl know people by their verified address in Slack now"
          : "fctl knows only the people who link their Slack account now, in every workspace",
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  if (admin.isPending)
    return (
      <div className="flex flex-col gap-4">
        <Skeleton className="h-5 w-96 max-w-full" />
        <Skeleton className="h-32" />
        <Skeleton className="h-48" />
      </div>
    );
  if (admin.isError)
    return (
      <Banner tone="error">
        The Slack app didn't load: {admin.error.message}{" "}
        <button type="button" className="underline underline-offset-2" onClick={() => void admin.refetch()}>
          Try again
        </button>
      </Banner>
    );
  const a = admin.data;
  const teams = a.teams ?? [];
  const matchOn = save.isPending ? !!save.variables : a.match_email;
  // Its credentials are set, and Slack's events have a way in.
  const ready = a.configured && (a.signing_secret || a.socket_mode);
  return (
    <div>
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        One Slack app serves every workspace here: their admins add it to their Slack workspaces, where people
        then start and steer agents by mentioning it or writing to it. It reads only the conversations it's
        in, and fctl keeps each agent to its own thread and the channels its workspace makes readable.
      </p>
      <div className="mt-6 overflow-hidden rounded-xl border border-line bg-surface/60">
        <div className="flex items-center gap-3 px-4 py-3.5">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-lg border border-line bg-raised">
            <SlackIcon className="size-[18px]" />
          </span>
          <div className="min-w-0 flex-1">
            <p className="truncate text-[14.5px] font-medium text-ink">{name}'s Slack app</p>
            <p className="text-[12px] text-ink-4">
              {teams.length === 0
                ? "In no Slack workspace yet"
                : `In ${teams.length === 1 ? "1 Slack workspace" : `${teams.length} Slack workspaces`}`}
            </p>
          </div>
          <span
            className={cn(
              "inline-flex shrink-0 items-center gap-1.5 text-[12.5px]",
              a.configured ? "text-accent" : "text-ink-4",
            )}
          >
            {a.configured && <Check className="size-3.5" />}
            {a.configured ? "Set up" : "Not set up yet"}
          </span>
        </div>
        <ul className="flex flex-col gap-1.5 border-t border-line bg-sunken/40 px-4 py-3">
          <Fact state={a.configured ? "ok" : "off"}>
            {a.configured
              ? "SLACK_CLIENT_ID and SLACK_CLIENT_SECRET are set: workspaces' admins add the app to Slack, and people link their accounts."
              : "SLACK_CLIENT_ID and SLACK_CLIENT_SECRET aren't set: create the app below, then give the instance its credentials."}
          </Fact>
          <EventFacts a={a} />
        </ul>
      </div>
      {/* The sections under the app's card, the first without a line above it. */}
      <div className="mt-6">
        {/* Until the app is whole, how to finish it; then the manifest alone, for reference. */}
        {!ready && (
          <Section
            title={a.configured ? "Finish setting up the app" : "Create the Slack app"}
            description={
              a.configured
                ? "Slack's events have no way to reach this instance yet: give it the app's signing secret, or an app-level token for Socket Mode."
                : "A few minutes in Slack's settings, then its credentials go to this instance."
            }
          >
            <SlackAppSetup standalone />
          </Section>
        )}
        <Section
          title="The addresses Slack calls"
          description="The manifest has them already. They matter for an app set up by hand, or once this instance's address changes."
        >
          <div className="flex flex-col gap-4">
            <Address label="Event subscriptions: request URL" value={a.events_url} />
            <Address label="Interactivity: request URL" value={a.interactivity_url} />
            <Address label="Slash command /fctl: request URL" value={a.commands_url} />
            {a.redirect_urls.map((u, i) => (
              <Address key={u} label={i === 0 ? "OAuth: redirect URLs" : undefined} value={u} />
            ))}
            {a.socket_mode && (
              <p className="text-[12.5px] leading-relaxed text-ink-4">
                With Socket Mode on, Slack sends events, interactions and commands over its connection instead
                of the first three.
              </p>
            )}
          </div>
        </Section>
        {ready && (
          <Section
            title="The manifest"
            description="To make the app again (for another instance, say), or to compare with the app's own."
          >
            <ManifestBox app={a} />
          </Section>
        )}
        <Section
          title="Slack workspaces"
          description="Where the app is installed, and how many of this instance's workspaces work in each."
        >
          {teams.length === 0 ? (
            <p className="border-y border-line py-4 text-[13px] text-ink-4">
              None yet: a workspace's admins add the app to Slack from their workspace's Integrations tab.
            </p>
          ) : (
            <ul className="divide-y divide-line border-y border-line">
              {teams.map((t) => (
                <li key={t.team} className="flex items-center gap-3 py-3">
                  <SlackIcon className="size-4 shrink-0" />
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13.5px] text-ink">{t.name}</div>
                    <div className="truncate text-[12px] text-ink-4">
                      {[
                        t.domain && `${t.domain}.slack.com`,
                        t.workspaces === 0
                          ? "no workspace works in it"
                          : t.workspaces === 1
                            ? "1 workspace works in it"
                            : `${t.workspaces} workspaces work in it`,
                        `installed ${agoAfterVerb(t.installed_at)}`,
                      ]
                        .filter(Boolean)
                        .join(" · ")}
                    </div>
                  </div>
                  <span
                    className={cn(
                      "shrink-0 text-[12.5px]",
                      t.status === "active" ? "text-accent" : "text-amber",
                    )}
                  >
                    {t.status === "active" ? "Installed" : "Access revoked"}
                  </span>
                </li>
              ))}
            </ul>
          )}
        </Section>
        <Section
          title="People"
          description="How fctl knows who someone in Slack is: what they ask there starts sessions as them, on their Claude subscription."
        >
          <SwitchRows>
            <SwitchRow
              label="Workspaces may know people by their verified address"
              checked={matchOn}
              disabled={save.isPending}
              onChange={(v) => save.mutate(v)}
              hint={
                matchOn
                  ? "A workspace's admins may then let fctl count someone in Slack who hasn't linked their account as the member whose verified address their Slack account has."
                  : "Off: fctl knows only the people who link their Slack account, by signing in to Slack from fctl."
              }
            />
          </SwitchRows>
          <p className="mt-3 flex gap-2 text-[12.5px] leading-relaxed text-ink-3">
            <CircleAlert aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-amber" />
            <span>
              Only where people sign in to Slack and fctl through the same provider, such as your company's
              single sign-on: elsewhere, someone who gives their Slack account a member's address would start
              sessions as that member, on their Claude subscription.
            </span>
          </p>
        </Section>
      </div>
    </div>
  );
}

/** An address Slack calls, to copy. */
function Address({ label, value }: { label?: string; value: string }) {
  return (
    <div className="flex flex-col gap-1.5">
      {label && <span className="text-[12.5px] font-medium text-ink-2">{label}</span>}
      <CodeBox value={value} />
    </div>
  );
}

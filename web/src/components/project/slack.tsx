// A project's Settings → Slack, for its owners and maintainers: whether its agents read the public
// channels its workspace's admins made readable, and a channel it posts chosen events to through
// fctl's app (pull requests ready for review and failures, unless it chooses others). Only where its
// workspace works in a Slack workspace; it says who connects one otherwise.

import { useMutation } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Hash } from "lucide-react";
import { useId, useState } from "react";
import { SwitchRow, SwitchRows, UnsavedGuard, useDraft } from "@/components/settings/form";
import { ChannelPicker } from "@/components/slack/channels";
import { SlackEventPreview } from "@/components/slack/preview";
import { useWorkspaceSlack } from "@/components/slack/workspace";
import { Banner, Button, Field, Input, SaveBar, Section, Select, Skeleton, toast } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { placeOf } from "@/lib/project-page";
import { keys, queryClient, useInstance, useWorkspace } from "@/lib/queries";
import {
  DEFAULT_EVENT_KINDS,
  eventsSentence,
  eventsWhen,
  isChannelId,
  kindsOf,
  readableChannels,
  SLACK_EVENT_KINDS,
} from "@/lib/slack";

/** What the page edits: whether agents read the readable channels, and where events go. */
const slackOf = (s: G.ProjectSettings): G.SlackProject => ({
  read_channels: s.slack?.read_channels ?? true,
  events: s.slack?.events,
});

/** "#incidents and #deploys", "#a, #b and #c". */
function channelList(names: string[]): string {
  const n = names.map((x) => `#${x}`);
  if (n.length <= 1) return n[0] ?? "";
  return `${n.slice(0, -1).join(", ")} and ${n[n.length - 1]}`;
}

export function ProjectSlack({ project }: { project: G.Project }) {
  const { data: instance } = useInstance();
  const ref = project.workspace;
  const workspace = useWorkspace(ref?.slug).data;
  const slack = useWorkspaceSlack(ref?.id, !!instance?.features.slack);
  const { draft, set, setDraft, dirty, reset } = useDraft(slackOf(project.settings));
  const save = useMutation({
    mutationFn: () => {
      const e = draft.events;
      if (e && !isChannelId(e.channel))
        throw new ApiFailure(400, { code: "validation", field: "slack", message: "Choose a channel" });
      if (e && chosenKinds(e).length === 0)
        throw new ApiFailure(400, { code: "validation", field: "slack", message: "Choose an event" });
      return api.updateProject(project.id, { settings: { ...project.settings, slack: draft } });
    },
    onSuccess: (p) => {
      queryClient.setQueryData(keys.project(project.slug), p);
      setDraft(slackOf(p.settings));
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      if (ref) void queryClient.invalidateQueries({ queryKey: keys.workspaceSlack(ref.id) });
      const e = p.settings.slack?.events;
      toast(
        e ? `Saved: it ${lowerFirst(eventsSentence(e))}` : "Saved: sessions that start from now on take it",
      );
    },
  });
  const name = workspace?.name ?? ref?.name ?? "its workspace";
  const admin = workspace?.role === "owner" || workspace?.role === "admin";
  const description = "What the project's agents read in Slack, and the channel it tells of what happens.";
  if (!instance?.features.slack)
    return (
      <Section title="Slack" description={description}>
        <Banner tone="info">This instance has no Slack app yet: its admins set one up.</Banner>
      </Section>
    );
  if (slack.isPending)
    return (
      <Section title="Slack" description={description}>
        <div className="flex flex-col gap-3">
          <Skeleton className="h-16" />
          <Skeleton className="h-16" />
        </div>
      </Section>
    );
  if (slack.isError)
    return (
      <Section title="Slack" description={description}>
        <Banner tone="error">
          {name}'s Slack didn't load: {slack.error.message}{" "}
          <button type="button" className="underline underline-offset-2" onClick={() => void slack.refetch()}>
            Try again
          </button>
        </Banner>
      </Section>
    );
  const s = slack.data;
  const connections = s.connections.filter((c) => c.status === "active");
  if (connections.length === 0)
    return (
      <Section title="Slack" description={description}>
        <div className="rounded-xl border border-line bg-surface/60 p-4 text-[13.5px] leading-relaxed text-ink-2">
          <p className="font-medium text-ink">
            {s.connections.length > 0
              ? `Slack took fctl's access back in ${name}'s Slack workspace`
              : `${name} doesn't work in Slack yet`}
          </p>
          <p className="mt-1 text-ink-3">
            {admin ? "Add fctl to Slack" : `${name}'s admins add fctl to Slack`} under the workspace's
            Integrations: then people there start this project's agents from Slack, its agents read the
            channels made readable there, and it can post its events to a channel.
          </p>
          {ref && (
            <Link
              to="/w/$slug"
              params={{ slug: ref.slug }}
              search={{ tab: "integrations", integration: "slack" }}
              className="mt-3 inline-flex text-[13px] text-ink underline underline-offset-2"
            >
              {name}'s Integrations
            </Link>
          )}
        </div>
      </Section>
    );
  const err =
    save.error instanceof ApiFailure && save.error.field === "slack" ? save.error.message : undefined;
  const readable = connections.flatMap((c) => readableChannels(c).map((ch) => ch.name));
  return (
    <Section title="Slack" description={description}>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty) save.mutate();
        }}
      >
        <SwitchRows>
          <SwitchRow
            label="Agents read the readable channels"
            checked={draft.read_channels}
            onChange={(v) => set({ read_channels: v })}
            hint={
              !draft.read_channels
                ? "Off: agents read only the thread their session started from."
                : readable.length > 0
                  ? `Besides the thread their session started from, agents read ${channelList(readable)}: the channels ${name}'s admins made readable.`
                  : `Besides the thread their session started from, agents read the channels ${name}'s admins make readable. None is yet.`
            }
          />
          <Events
            project={project}
            workspaceName={name}
            connections={connections}
            events={draft.events}
            onChange={(events) => set({ events })}
            error={err}
          />
        </SwitchRows>
        <UnsavedGuard dirty={dirty && !save.isPending} page="Slack" at={(l) => placeOf(project, l)} />
        {(dirty || save.isPending) && (
          <SaveBar
            pending={save.isPending}
            disabled={!!draft.events && (!draft.events.channel || chosenKinds(draft.events).length === 0)}
            hint={
              draft.events && !draft.events.channel
                ? "Choose a channel first."
                : "Sessions that start from now on take it."
            }
            onReset={() => {
              reset();
              save.reset();
            }}
          />
        )}
        {save.error && !err && (
          <Banner tone="error" className="mt-4">
            {save.error.message}
          </Banner>
        )}
      </form>
    </Section>
  );
}

const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

/**
 * The events a draft posts: those ticked, none once all are unticked; the default ones where it never
 * chose, as stored without a list.
 */
const chosenKinds = (e: G.SlackEvents) => e.kinds ?? kindsOf(e);

/**
 * A channel the project posts chosen events to: whether it does, which Slack workspace and channel,
 * and which events, with what one looks like there.
 */
function Events({
  project,
  workspaceName,
  connections,
  events,
  onChange,
  error,
}: {
  project: G.Project;
  workspaceName: string;
  connections: G.SlackConnection[];
  events: G.SlackEvents | undefined;
  onChange: (e: G.SlackEvents | undefined) => void;
  error?: string;
}) {
  const app = useInstance().data?.name ?? "fctl";
  const connection = connections.find((c) => c.team === events?.team) ?? connections[0];
  const allowed = connection?.settings.project_events ?? false;
  const on = !!events;
  const kindsId = useId();
  if (!connection) return null;
  const kinds = events ? chosenKinds(events) : DEFAULT_EVENT_KINDS;
  const toggle = (type: string, v: boolean) =>
    events &&
    onChange({
      ...events,
      kinds: SLACK_EVENT_KINDS.map((k) => k.type).filter((t) => (t === type ? v : kinds.includes(t))),
    });
  return (
    <div>
      <SwitchRow
        label="Post events to a channel"
        checked={on}
        // Off is always a way out; on, only where the workspace's admins let projects post.
        disabled={!on && !allowed}
        onChange={(v) =>
          onChange(
            v
              ? { team: connection.team, channel: "", channel_name: "", kinds: DEFAULT_EVENT_KINDS }
              : undefined,
          )
        }
        hint={
          on
            ? events?.channel
              ? kinds.length === 0
                ? "Choose below when it posts."
                : `Posts to #${events.channel_name || events.channel} when ${eventsWhen(kinds)}.`
              : "fctl's app posts what you choose below to a channel, with a button that opens the session."
            : allowed
              ? "Off: the project posts nothing to Slack's channels. Its sessions' own threads still hear of them."
              : `${workspaceName}'s admins keep projects' events out of Slack: they turn them on under its Integrations.`
        }
      />
      {on && events && (
        <div className="flex animate-rise flex-col gap-5 pb-5">
          {!allowed && (
            <Banner tone="warn">
              {workspaceName}'s admins keep projects' events out of Slack, so nothing posts there until they
              turn them on under its Integrations.
            </Banner>
          )}
          {connections.length > 1 && (
            <Field label="Slack workspace">
              {(id) => (
                <span className="self-start">
                  <Select
                    id={id}
                    value={events.team}
                    options={connections.map((c) => ({ value: c.team, label: c.name }))}
                    onChange={(team) => onChange({ ...events, team, channel: "", channel_name: "" })}
                  />
                </span>
              )}
            </Field>
          )}
          <ChannelField
            project={project}
            connection={connection}
            events={events}
            onChange={onChange}
            error={error}
          />
          <fieldset aria-describedby={kindsId}>
            <legend className="text-[12.5px] font-medium text-ink-2">When</legend>
            <ul className="mt-2 grid gap-x-6 gap-y-2 sm:grid-cols-2">
              {SLACK_EVENT_KINDS.map((k) => (
                <li key={k.type}>
                  <label className="flex cursor-pointer items-start gap-2.5 text-[13.5px] text-ink select-none">
                    <input
                      type="checkbox"
                      checked={kinds.includes(k.type)}
                      onChange={(e) => toggle(k.type, e.target.checked)}
                      className="mt-[3px] size-4 shrink-0 accent-[var(--c-accent)]"
                    />
                    {k.label}
                  </label>
                </li>
              ))}
            </ul>
            <p id={kindsId} className="mt-2 text-[12.5px] text-ink-3">
              {kinds.length === 0
                ? "Choose at least one."
                : "Each posts a line of what happened, with a button that opens its session."}
            </p>
          </fieldset>
          {events.channel && kinds.length > 0 && (
            <SlackEventPreview
              app={app}
              channel={events.channel_name || events.channel}
              reference={`${project.slug}#12`}
              className="max-w-sm"
            />
          )}
        </div>
      )}
    </div>
  );
}

/**
 * The channel events go to: a public one found by searching Slack's channels (the project's
 * maintainers may, where the workspace lets projects post), or a private one by its ID, since only
 * its members see it.
 */
function ChannelField({
  project,
  connection,
  events,
  onChange,
  error,
}: {
  project: G.Project;
  connection: G.SlackConnection;
  events: G.SlackEvents;
  onChange: (e: G.SlackEvents) => void;
  error?: string;
}) {
  const [picking, setPicking] = useState(false);
  // A private channel, typed by its ID: where the one chosen isn't a public one found by name.
  const [typing, setTyping] = useState(false);
  const pick = (id: string, name: string) => onChange({ ...events, channel: id, channel_name: name });
  return (
    <div className="flex flex-col gap-4">
      <Field
        label="Channel"
        error={typing ? undefined : error}
        hint={
          typing ? undefined : "fctl's app joins a public channel if it isn't in it yet, which shows there."
        }
      >
        {(id) => (
          <div className="flex items-center gap-2">
            <span
              id={id}
              className={cn(
                "flex h-9.5 min-w-0 flex-1 items-center gap-1 rounded-md border border-line-strong bg-sunken/60 px-3 text-[14px]",
                events.channel && !typing ? "text-ink" : "text-ink-4",
              )}
            >
              {events.channel && !typing ? (
                <>
                  <Hash aria-hidden="true" className="size-3.5 shrink-0 text-ink-3" />
                  <span className="truncate">{events.channel_name || events.channel}</span>
                </>
              ) : typing ? (
                "A private channel, by its ID"
              ) : (
                "No channel yet"
              )}
            </span>
            <Button
              type="button"
              size="field"
              aria-label={
                events.channel && !typing
                  ? `Change the channel, #${events.channel_name}`
                  : "Choose the channel"
              }
              onClick={() => setPicking(true)}
            >
              {events.channel && !typing ? "Change" : "Choose"}
            </Button>
            <ChannelPicker
              workspace={project.workspace?.id ?? ""}
              team={connection.team}
              project={project.id}
              open={picking}
              onClose={() => setPicking(false)}
              title="Post events to a channel"
              description={`${project.name}'s events go there, as fctl's app.`}
              note="Private channels aren't listed: close this and give one's ID instead."
              onPick={(c) => {
                setTyping(false);
                pick(c.id, c.name);
              }}
            />
          </div>
        )}
      </Field>
      {typing ? (
        <Field
          label="Its ID"
          error={error}
          hint="In Slack, open the channel's details: About ends with its channel ID. Its members invite fctl's app first (/invite @fctl)."
        >
          {(id) => (
            <Input
              id={id}
              value={events.channel}
              onChange={(e) => {
                const v = e.target.value.trim().toUpperCase();
                onChange({ ...events, channel: v, channel_name: v });
              }}
              placeholder="C0123ABCD"
              aria-invalid={!!events.channel && !isChannelId(events.channel)}
              className="font-mono"
              autoCapitalize="characters"
              autoCorrect="off"
              spellCheck={false}
            />
          )}
        </Field>
      ) : (
        <button
          type="button"
          className="-mt-2 self-start text-[12.5px] text-ink-3 underline decoration-line-strong underline-offset-2 transition-colors hover:text-ink"
          onClick={() => {
            setTyping(true);
            onChange({ ...events, channel: "", channel_name: "" });
          }}
        >
          A private channel? Give its ID
        </button>
      )}
    </div>
  );
}

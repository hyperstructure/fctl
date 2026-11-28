// A workspace's Slack (Integrations → Slack): before anything is connected, what connecting a Slack
// workspace leads to, with a thread as Slack would show it, and Add to Slack for its admins (or, on an
// instance without a Slack app, how its admins create one). Once connected: the Slack workspace, what
// agents can see there in a sentence, and what the workspace lets people and agents do, grouped as
// people meet it: who starts sessions where, what agents read, what they post, who fctl knows, what
// Slack hears besides threads, and what transcripts keep. Members read it all; its admins change it.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  ArrowRight,
  Check,
  Eye,
  EyeOff,
  Hash,
  Lock,
  Plus,
  RotateCcw,
  Trash2,
  UserRound,
  X,
} from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { SwitchRow, SwitchRows } from "@/components/settings/form";
import {
  Banner,
  Button,
  DangerZone,
  Dialog,
  IconButton,
  Section,
  Select,
  type SelectOption,
  Skeleton,
  SlackIcon,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { clickConfirmed, slackLinkUrl, workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { goTo } from "@/lib/history";
import { keys, queryClient, useInstance, useProjects } from "@/lib/queries";
import {
  agoAfterVerb,
  connectOutcome,
  eventsSentence,
  mappedChannels,
  readableChannels,
  type SlackOutcome,
  settingSaved,
} from "@/lib/slack";
import { ChannelPicker } from "./channels";
import { SlackThreadPreview } from "./preview";
import { SlackAppSetup } from "./setup";

const isAdmin = (w: G.Workspace) => w.role === "owner" || w.role === "admin";

/** The workspace's Slack, as everyone in it reads it: `GET /workspaces/{id}/slack`. */
export const useWorkspaceSlack = (id: string | undefined, enabled = true) =>
  useQuery({
    queryKey: keys.workspaceSlack(id ?? ""),
    queryFn: () => workspaces.slack(id ?? ""),
    enabled: !!id && enabled,
  });

export function SlackTab({ w, returned, team }: { w: G.Workspace; returned?: SlackOutcome; team?: string }) {
  const admin = isAdmin(w);
  const slack = useWorkspaceSlack(w.id);
  const connect = useMutation({
    mutationFn: (team?: string) => workspaces.connectSlack(w.id, team),
    // On to Slack, which brings the browser back here.
    onSuccess: (link) => goTo(link.url),
    onError: (e) => toast(e.message, "error"),
  });
  const connecting = connect.isPending || connect.isSuccess;
  if (slack.isPending) return <Loading />;
  if (slack.isError)
    return (
      <Banner tone="error">
        Slack didn't load: {slack.error.message}{" "}
        <button type="button" className="underline underline-offset-2" onClick={() => void slack.refetch()}>
          Try again
        </button>
      </Banner>
    );
  const s = slack.data;
  // The Slack workspace a sign-in just connected: the one the way back names, else the newest.
  const connected =
    s.connections.find((c) => c.team === team) ??
    [...s.connections].sort((a, b) => b.created_at - a.created_at)[0];
  const outcome = returned && connectOutcome(returned, connected?.name);
  return (
    <div className="flex flex-col gap-10">
      {outcome && (
        <Banner tone={outcome.tone} className="-mb-4">
          {outcome.text}
        </Banner>
      )}
      {!s.configured ? (
        <NoApp w={w} s={s} />
      ) : s.connections.length === 0 ? (
        <Intro w={w} s={s} admin={admin} connecting={connecting} onConnect={(t) => connect.mutate(t)} />
      ) : (
        <Connected w={w} s={s} admin={admin} connecting={connecting} onConnect={(t) => connect.mutate(t)} />
      )}
    </div>
  );
}

/** While it loads: the intro's shape, so that nothing jumps when it comes. */
function Loading() {
  return (
    <div className="grid items-start gap-x-12 gap-y-8 md:grid-cols-[minmax(0,1fr)_340px]" aria-busy="true">
      <div className="flex flex-col gap-3">
        <Skeleton className="h-8 w-72 max-w-full" />
        <Skeleton className="h-4 w-full max-w-xl" />
        <Skeleton className="h-4 w-4/5 max-w-lg" />
        <Skeleton className="mt-4 h-4 w-3/4" />
        <Skeleton className="h-4 w-2/3" />
        <Skeleton className="h-4 w-3/4" />
        <Skeleton className="mt-4 h-11 w-40" />
      </div>
      <Skeleton className="h-80 max-md:hidden" />
    </div>
  );
}

/** The headline, the same before and after connecting: what Slack is for here. */
function Headline({ id, children }: { id: string; children: ReactNode }) {
  return (
    <div>
      <h2
        id={id}
        className="font-display text-[26px] leading-[1.15] font-medium tracking-[-0.018em] text-ink sm:text-[28px]"
      >
        Start agents from Slack
      </h2>
      <p className="mt-2 max-w-2xl text-[15px] leading-relaxed text-ink-3">{children}</p>
    </div>
  );
}

/** "Web", "Web and Docs", "Web, Docs and API", "its 5 projects". */
function projectWords(names: string[]): string | undefined {
  if (names.length === 0) return undefined;
  if (names.length > 3) return `its ${names.length} projects`;
  return names.length === 1 ? names[0] : `${names.slice(0, -1).join(", ")} and ${names[names.length - 1]}`;
}

/** The workspace's projects the caller sees, in their order. */
function useWorkspaceProjects(w: G.Workspace) {
  return (useProjects().data?.projects ?? []).filter((p) => p.workspace?.id === w.id && !p.archived_at);
}

/** What the instance's admins have to do before Slack's events reach fctl, where they haven't. */
function NoEvents({ s, className }: { s: G.WorkspaceSlack; className?: string }) {
  if (s.events) return null;
  return (
    <Banner tone="warn" className={className}>
      Nothing people do in Slack reaches fctl yet: this instance's admins set SLACK_SIGNING_SECRET, or
      SLACK_APP_TOKEN for Socket Mode.
      {s.instance_admin && (
        <>
          {" "}
          <Link to="/admin" search={{ tab: "slack" }} className="underline underline-offset-2">
            Admin → Slack
          </Link>
        </>
      )}
    </Banner>
  );
}

/** Socket Mode, where events come over it, while it isn't connected. */
function SocketDown({ s, className }: { s: G.WorkspaceSlack; className?: string }) {
  if (!s.socket_mode || s.socket_connected !== false) return null;
  return (
    <Banner tone="warn" className={className}>
      fctl isn't connected to Slack's Socket Mode right now, so what people ask in Slack waits until it is. It
      connects again by itself.
      {s.instance_admin && (
        <>
          {" "}
          <Link to="/admin" search={{ tab: "slack" }} className="underline underline-offset-2">
            See why in Admin → Slack
          </Link>
        </>
      )}
    </Banner>
  );
}

// ───────────────────────────── no app ─────────────────────────────

/**
 * An instance without a Slack app: its admins create one here, beside the thread it leads to;
 * everyone else is told who does.
 */
function NoApp({ w, s }: { w: G.Workspace; s: G.WorkspaceSlack }) {
  const heading = useId();
  const app = useInstance().data?.name ?? "fctl";
  const first = useWorkspaceProjects(w)[0];
  return (
    <section
      aria-labelledby={heading}
      className="grid animate-rise items-start gap-x-12 gap-y-10 md:grid-cols-[minmax(0,1fr)_340px]"
    >
      <div className="min-w-0">
        <Headline id={heading}>
          This instance has no Slack app yet.{" "}
          {s.instance_admin
            ? `Create it once, for every workspace here: then ${w.name}'s admins add it to your Slack workspace, and people there start agents by mentioning it or writing to it.`
            : "Its admins create one; then this workspace's admins add it to your Slack workspace."}
        </Headline>
        {s.instance_admin && (
          <div className="mt-8">
            <SlackAppSetup />
          </div>
        )}
      </div>
      <SlackThreadPreview
        app={app}
        project={first?.name ?? "Web"}
        slug={first?.slug ?? "web"}
        className="max-md:hidden md:sticky md:top-8"
      />
    </section>
  );
}

// ───────────────────────────── before connecting ─────────────────────────────

/**
 * Before anything is connected: what connecting leads to, in this workspace's words, with a thread
 * as Slack shows it, the three things people ask (who it acts as, what agents read, what they never
 * reach) and one action, or who can act.
 */
function Intro({
  w,
  s,
  admin,
  connecting,
  onConnect,
}: {
  w: G.Workspace;
  s: G.WorkspaceSlack;
  admin: boolean;
  connecting: boolean;
  /** Off to Slack to install fctl's app: in `team` alone, when it names one. */
  onConnect: (team?: string) => void;
}) {
  const heading = useId();
  const projects = useWorkspaceProjects(w);
  const app = useInstance().data?.name ?? "fctl";
  const where = projectWords(projects.map((p) => p.name));
  const first = projects[0];
  const points: { icon: ReactNode; text: string }[] = [
    {
      icon: <UserRound />,
      text: "Each session runs as the person who asked, on their Claude subscription. fctl knows people once they link their Slack account, in two clicks the first time they ask.",
    },
    {
      icon: <Eye />,
      text: `Agents read the thread they started from, in a channel, a private channel or a direct message, and the public channels ${w.name}'s admins make readable.`,
    },
    {
      icon: <EyeOff />,
      text: "They never reach other messages of private channels, anyone's direct messages with fctl, or channels shared with other organizations.",
    },
  ];
  return (
    <section
      aria-labelledby={heading}
      className="grid animate-rise items-center gap-x-12 gap-y-10 md:grid-cols-[minmax(0,1fr)_340px]"
    >
      <div>
        <Headline id={heading}>
          Connect a Slack workspace, and people there start agents
          {where ? ` in ${where}` : ` in ${w.name}'s projects`} by mentioning @{app} in a channel, or writing
          to it directly. The agent works in the thread: its card there shows how it's going, its questions
          and replies come there, and a reply there steers it.
        </Headline>
        <ul className="mt-6 flex max-w-xl flex-col gap-3">
          {points.map((p) => (
            <li key={p.text} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
              <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-3 [&_svg]:size-4">
                {p.icon}
              </span>
              {p.text}
            </li>
          ))}
        </ul>
        <NoEvents s={s} className="mt-6 max-w-xl" />
        {admin ? (
          <div className="mt-8 flex flex-wrap items-center gap-x-4 gap-y-3">
            <Button
              variant="primary"
              size="lg"
              icon={<SlackIcon className="size-4" />}
              loading={connecting}
              onClick={() => onConnect()}
            >
              Add to Slack
            </Button>
            <span className="text-[12.5px] leading-snug text-ink-4">
              Slack asks you to sign in and allow fctl's app.
              <br className="max-sm:hidden" /> One of its admins may have to approve it.
            </span>
          </div>
        ) : (
          <p className="mt-8 max-w-xl text-[13px] leading-relaxed text-ink-4">
            {w.name}'s admins add it to Slack
            {w.primary ? ` (${w.primary.name} is its primary admin)` : ""}: ask one who may add apps to your
            Slack workspace too.
          </p>
        )}
      </div>
      <SlackThreadPreview
        app={app}
        project={first?.name ?? "Web"}
        slug={first?.slug ?? "web"}
        className="max-md:max-w-sm"
      />
    </section>
  );
}

// ───────────────────────────── connected ─────────────────────────────

function Connected({
  w,
  s,
  admin,
  connecting,
  onConnect,
}: {
  w: G.Workspace;
  s: G.WorkspaceSlack;
  admin: boolean;
  connecting: boolean;
  /** Off to Slack to install fctl's app: in `team` alone, when it names one. */
  onConnect: (team?: string) => void;
}) {
  const heading = useId();
  const app = useInstance().data?.name ?? "fctl";
  const [chosen, setChosen] = useState<string>();
  const c = s.connections.find((x) => x.team === chosen) ?? s.connections[0];
  const active = s.connections.filter((x) => x.status === "active");
  if (!c) return null;
  const several = s.connections.length > 1;
  return (
    <>
      <section aria-labelledby={heading} className="animate-rise">
        <Headline id={heading}>
          {active.length === 0
            ? `Slack took fctl's access back, so nothing people ask there reaches ${w.name} until ${admin ? "you connect it" : "its admins connect it"} again.`
            : `People in ${active.length === 1 ? active[0]?.name : "these Slack workspaces"} start ${w.name}'s agents by mentioning @${c.app_name ?? app} in a channel, or writing to it directly. Each session runs as whoever asked, on their Claude subscription, and works in the thread.`}
        </Headline>
        <NoEvents s={s} className="mt-5 max-w-2xl" />
        <SocketDown s={s} className="mt-5 max-w-2xl" />
        <ul className="mt-6 flex flex-col gap-2.5">
          {s.connections.map((x) => (
            <ConnectionCard
              key={x.team}
              w={w}
              s={s}
              c={x}
              admin={admin}
              connecting={connecting}
              onConnect={onConnect}
            />
          ))}
        </ul>
      </section>
      {c.status === "active" && (
        <div>
          {several && (
            <div className="mb-2 flex flex-wrap items-center gap-x-3 gap-y-2">
              <span className="text-[13px] text-ink-3">What {w.name} does in</span>
              <Select
                size="sm"
                aria-label="The Slack workspace whose choices show"
                value={c.team}
                options={s.connections.map((x) => ({ value: x.team, label: x.name }))}
                onChange={setChosen}
              />
            </div>
          )}
          {!admin && (
            <p className="mb-2 text-[13px] text-ink-4">
              {w.name}'s admins change these
              {w.primary ? ` (${w.primary.name} is its primary admin)` : ""}.
            </p>
          )}
          <Choices key={c.team} w={w} s={s} c={c} admin={admin} />
        </div>
      )}
      {admin && (
        <section className="grid items-center gap-x-8 gap-y-4 md:grid-cols-[minmax(0,1fr)_auto]">
          <div>
            <h3 className="text-[13.5px] font-semibold text-ink">Connect another Slack workspace</h3>
            <p className="mt-1 max-w-2xl text-[13px] leading-relaxed text-ink-3">
              You don't need one unless people in another Slack workspace ask {w.name}'s agents for work too.
              Slack asks you to sign in there and allow fctl's app.
            </p>
          </div>
          <div>
            <Button
              icon={<SlackIcon className="size-3.5" />}
              loading={connecting}
              onClick={() => onConnect()}
            >
              Add to Slack
            </Button>
          </div>
        </section>
      )}
      {admin && <Disconnect w={w} c={c} />}
    </>
  );
}

/**
 * A Slack workspace it works in: its name and address, how it stands, who connected it and how many
 * of the workspace's people fctl knows there; what agents can see there, in a sentence; and whether
 * fctl knows you there.
 */
function ConnectionCard({
  w,
  s,
  c,
  admin,
  connecting,
  onConnect,
}: {
  w: G.Workspace;
  s: G.WorkspaceSlack;
  c: G.SlackConnection;
  admin: boolean;
  connecting: boolean;
  /** Off to Slack to install fctl's app: in `team` alone, when it names one. */
  onConnect: (team?: string) => void;
}) {
  const revoked = c.status === "revoked";
  const linked = (s.linked_teams ?? []).includes(c.team);
  const app = c.app_name ?? "fctl";
  return (
    <li className="overflow-hidden rounded-xl border border-line bg-surface/60">
      <div className="flex items-start gap-3.5 px-4 py-3.5">
        <span className="mt-0.5 flex size-9 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-line bg-raised">
          {c.icon_url ? (
            <img src={c.icon_url} alt="" className="size-full object-cover" referrerPolicy="no-referrer" />
          ) : (
            <SlackIcon className="size-[18px]" />
          )}
        </span>
        <div className="min-w-0 flex-1">
          {/* How it stands beside its name, on a phone too. */}
          <div className="flex items-center justify-between gap-3">
            {c.domain ? (
              <a
                href={`https://${c.domain}.slack.com`}
                target="_blank"
                rel="noreferrer"
                className="min-w-0 truncate text-[14.5px] font-medium text-ink hover:underline"
              >
                {c.name}
              </a>
            ) : (
              <span className="min-w-0 truncate text-[14.5px] font-medium text-ink">{c.name}</span>
            )}
            <div className="-my-1 flex shrink-0 items-center gap-1">
              {!revoked ? (
                <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
                  <Check className="size-3.5" /> Connected
                </span>
              ) : admin ? (
                <Button
                  size="sm"
                  icon={<RotateCcw className="size-3.5" />}
                  loading={connecting}
                  onClick={() => onConnect(c.team)}
                >
                  Connect again
                </Button>
              ) : (
                <span className="text-[12.5px] text-amber">Access revoked</span>
              )}
            </div>
          </div>
          <div className="mt-0.5 text-[12px] leading-relaxed text-ink-4 [overflow-wrap:anywhere]">
            {[
              c.domain && `${c.domain}.slack.com`,
              `as @${app}`,
              c.connected_by && `connected by ${c.connected_by.name} ${agoAfterVerb(c.created_at)}`,
              c.linked === 1
                ? "1 person here linked their account"
                : `${c.linked} people here linked their accounts`,
            ]
              .filter(Boolean)
              .join(" · ")}
          </div>
          {revoked && (
            <p className="mt-2 text-[12.5px] leading-relaxed text-ink-3">
              Slack took fctl's access back: its app was removed there, or its token revoked. Until{" "}
              {admin ? "you connect it" : `${w.name}'s admins connect it`} again, mentions and messages there
              start nothing, and the sessions they started carry on without posting there.
            </p>
          )}
        </div>
      </div>
      {!revoked && (
        <div className="flex flex-col gap-2 border-t border-line bg-sunken/40 px-4 py-3">
          <p className="flex gap-2.5 text-[14px] leading-relaxed text-ink">
            <Eye aria-hidden="true" className="mt-[3px] size-4 shrink-0 text-accent" />
            <span className="min-w-0">{c.reach}</span>
          </p>
          {s.socket_mode && s.socket_connected && (
            <p className="flex items-center gap-2.5 text-[12px] text-ink-4">
              <span aria-hidden="true" className="ml-[5px] size-1.5 shrink-0 rounded-full bg-accent" />
              Slack's events come over Socket Mode, connected now
            </p>
          )}
          <Yours team={c.team} name={c.name} linked={linked} />
        </div>
      )}
    </li>
  );
}

/** Whether fctl knows the reader in a Slack workspace: what they ask there runs as them. */
function Yours({ team, name, linked }: { team: string; name: string; linked: boolean }) {
  if (linked)
    return (
      <p className="flex gap-2.5 text-[12.5px] leading-relaxed text-ink-3">
        <Check aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-accent" />
        <span className="min-w-0">
          fctl knows your Slack account in {name}: what you ask fctl there starts sessions as you.{" "}
          <Link to="/settings" search={{ tab: "slack" }} className="text-ink-2 underline underline-offset-2">
            Settings
          </Link>
        </span>
      </p>
    );
  const href = slackLinkUrl({ team });
  return (
    <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-2 rounded-lg border border-blue/25 bg-blue-soft px-3 py-2.5">
      <p className="min-w-0 flex-1 basis-64 text-[12.5px] leading-relaxed text-ink-2">
        <span className="font-medium text-ink">Link your Slack account.</span> fctl starts sessions only for
        people it knows in {name}, as them. It takes a Slack sign-in.
      </p>
      <a
        href={href}
        onClick={(e) => clickConfirmed(e, href)?.catch((err: Error) => toast(err.message, "error"))}
        className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-sm border border-line-strong bg-raised px-2.5 text-[12.5px] font-medium text-ink transition-colors hover:border-ink-4"
      >
        <SlackIcon className="size-3.5" />
        Link your Slack account
      </a>
    </div>
  );
}

// ───────────────────────────── its choices ─────────────────────────────

/** Changes a Slack workspace's choices at once, the switch moving before the server answers. */
function useUpdate(w: G.Workspace) {
  const key = keys.workspaceSlack(w.id);
  return useMutation({
    mutationFn: ({ team, b }: { team: string; b: G.UpdateSlackRequest }) =>
      workspaces.updateSlack(w.id, team, b),
    onMutate: ({ team, b }) => {
      queryClient.setQueryData<G.WorkspaceSlack>(
        key,
        (old) =>
          old && {
            ...old,
            connections: old.connections.map((c) =>
              c.team === team ? { ...c, settings: { ...c.settings, ...withoutUndefined(b) } } : c,
            ),
          },
      );
    },
    onSuccess: (s, { b }) => {
      queryClient.setQueryData(key, s);
      toast(settingSaved(b));
    },
    onError: (e) => {
      toast(e.message, "error");
      void queryClient.invalidateQueries({ queryKey: key });
    },
  });
}

const withoutUndefined = <T extends object>(o: T): Partial<T> =>
  Object.fromEntries(Object.entries(o).filter(([, v]) => v !== undefined)) as Partial<T>;

/** What a change to a channel comes to, for its toast. */
function channelSaved(name: string, b: G.SetSlackChannelRequest, project?: string): string {
  if (b.readable === true) return `Agents read #${name} now: fctl joined it, which shows there`;
  if (b.readable === false) return `Agents no longer read #${name}`;
  if (b.project === "") return `Mentions in #${name} ask which project now`;
  return `Mentions in #${name} start sessions in ${project ?? "the project"} now`;
}

/** Sets what the workspace does with a channel. */
function useChannel(w: G.Workspace) {
  return useMutation({
    mutationFn: (v: {
      team: string;
      channel: string;
      name: string;
      b: G.SetSlackChannelRequest;
      project?: string;
    }) => workspaces.setSlackChannel(w.id, v.team, v.channel, v.b),
    onSuccess: (s, v) => {
      queryClient.setQueryData(keys.workspaceSlack(w.id), s);
      // Projects show what they read and post.
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      toast(channelSaved(v.name, v.b, v.project));
    },
    onError: (e) => toast(e.message, "error"),
  });
}

/** What the workspace lets people and agents do in a Slack workspace, in sections. */
function Choices({
  w,
  s,
  c,
  admin,
}: {
  w: G.Workspace;
  s: G.WorkspaceSlack;
  c: G.SlackConnection;
  admin: boolean;
}) {
  const update = useUpdate(w);
  const set = (b: G.UpdateSlackRequest) => update.mutate({ team: c.team, b });
  const st = c.settings;
  return (
    <div>
      <Section
        title="Starting sessions"
        description="Who starts sessions from Slack, and in which project. Each runs as whoever asked, on their Claude subscription, once fctl knows them."
      >
        <Settings>
          <Setting
            admin={admin}
            label="Mentions in channels"
            checked={st.channels}
            onChange={(v) => set({ channels: v })}
            hint={
              st.channels
                ? "In the channels fctl is in, mentioning it, /fctl and Send to fctl… start a session as whoever asked."
                : "Off: mentions, /fctl and Send to fctl… start nothing. Replies in threads that started sessions still reach them."
            }
          />
          <Setting
            admin={admin}
            label="Direct messages"
            checked={st.direct_messages}
            onChange={(v) => set({ direct_messages: v })}
            hint={
              st.direct_messages
                ? "Writing to fctl in its messages starts a session, in a thread only its writer sees; so do fctl's Home tab and its Start an fctl session shortcut."
                : "Off: fctl's messages, its Home tab and its shortcut start nothing."
            }
          />
        </Settings>
        <Mappings w={w} c={c} admin={admin} />
      </Section>
      <Section
        title="What agents read"
        description="Every agent reads the thread its session started from. The public channels made readable here, agents read too: fctl joins them, which shows there."
      >
        <Readable w={w} c={c} admin={admin} />
        <Settings className="mt-5">
          <Setting
            admin={admin}
            label="Search the readable channels"
            checked={st.search}
            onChange={(v) => set({ search: v })}
            hint={
              st.search
                ? "Agents search them for what they need, besides reading their history."
                : "Off: agents read the readable channels' history and the messages linked to them, without searching."
            }
          />
        </Settings>
        <ProjectsRead w={w} c={c} />
      </Section>
      <Section
        title="What agents post"
        description="fctl posts each session's card in its thread and keeps it current, with the agent's questions and the last message of each turn."
      >
        <Settings>
          <Setting
            admin={admin}
            label="Agents' own replies"
            checked={st.agent_replies}
            onChange={(v) => set({ agent_replies: v })}
            hint={
              st.agent_replies
                ? "Agents also post in their own thread when they choose, with fctl slack reply: never anywhere else."
                : "Off: only fctl posts in sessions' threads."
            }
          />
        </Settings>
      </Section>
      <Section
        title="People"
        description={`fctl starts sessions only for people it knows in ${c.name}, as them.`}
      >
        <p className="text-[13.5px] leading-relaxed text-ink-2">
          {c.linked === 0
            ? `Nobody in ${w.name} has linked their Slack account in ${c.name} yet: each person links theirs once, from the message fctl sends them the first time they ask, or in Settings.`
            : `${c.linked} of ${w.name}'s ${w.members} ${w.members === 1 ? "member" : "members"} linked their Slack account in ${c.name}. Others link theirs from the message fctl sends them the first time they ask, or in Settings.`}
        </p>
        {s.match_email ? (
          <Settings className="mt-4">
            <Setting
              admin={admin}
              label="Know people by their verified address"
              checked={st.match_email}
              onChange={(v) => set({ match_email: v })}
              hint={
                st.match_email
                  ? `Someone who hasn't linked their account counts as the member of ${w.name} whose address their Slack account has. Leave it on only where people sign in to Slack and fctl through the same provider.`
                  : "Off: fctl knows only the people who linked their Slack account."
              }
            />
          </Settings>
        ) : (
          <p className="mt-3 text-[12.5px] leading-relaxed text-ink-4">
            fctl knows only the people who linked their account: this instance's admins don't let workspaces
            know people by their address in Slack.
          </p>
        )}
      </Section>
      <Section
        title="Notifications and events"
        description="What fctl tells people and channels in Slack, besides the threads that started sessions."
      >
        <Settings>
          <Setting
            admin={admin}
            label="Notifications in Slack"
            checked={st.notifications}
            onChange={(v) => set({ notifications: v })}
            hint={
              st.notifications
                ? "People who turn on Tell me in Slack, in Settings → Notifications, get theirs as direct messages from fctl."
                : `Off: notifications about ${w.name}'s sessions come only to people's browsers.`
            }
          />
          <Setting
            admin={admin}
            label="Projects' events"
            checked={st.project_events}
            onChange={(v) => set({ project_events: v })}
            hint={
              st.project_events
                ? "Projects post the events they choose to a channel, in their settings under Slack."
                : "Off: projects post nothing to channels."
            }
          />
        </Settings>
        <ProjectEvents c={c} on={st.project_events} />
      </Section>
      <Section
        title="Transcripts"
        description="What a session's transcript keeps of Slack: the messages that reached its agent, and what it read."
      >
        <Transcripts w={w} value={st.transcripts} admin={admin} onChange={(v) => set({ transcripts: v })} />
      </Section>
    </div>
  );
}

/** Switches one after another, between hairlines. */
function Settings({ children, className }: { children: ReactNode; className?: string }) {
  return <SwitchRows className={className}>{children}</SwitchRows>;
}

/** A switch for its admins; for everyone else, what it is set to, in a word. */
function Setting({
  admin,
  label,
  hint,
  checked,
  onChange,
}: {
  admin: boolean;
  label: string;
  hint: string;
  checked: boolean;
  onChange: (v: boolean) => void;
}) {
  if (admin) return <SwitchRow label={label} hint={hint} checked={checked} onChange={onChange} />;
  return (
    <div className="flex items-start justify-between gap-6 py-3.5">
      <div className="min-w-0">
        <span className="block text-[13.5px] leading-5 text-ink">{label}</span>
        <span className="mt-0.5 block text-[12.5px] leading-relaxed text-ink-4">{hint}</span>
      </div>
      <span className={cn("mt-0.5 shrink-0 text-[12.5px]", checked ? "text-accent" : "text-ink-4")}>
        {checked ? "On" : "Off"}
      </span>
    </div>
  );
}

/** A channel's name, as Slack shows it: `#`, or a lock for a private one. */
function ChannelName({ c }: { c: Pick<G.SlackChannel, "name" | "private"> }) {
  return (
    <span className="inline-flex min-w-0 items-center gap-0.5 font-medium text-ink">
      {c.private ? (
        <Lock aria-hidden="true" className="size-3 shrink-0 text-ink-3" />
      ) : (
        <Hash aria-hidden="true" className="size-3.5 shrink-0 text-ink-3" />
      )}
      {c.private && <span className="sr-only">the private channel </span>}
      <span className="truncate">{c.name}</span>
    </span>
  );
}

/**
 * The channels whose mentions start sessions in a project, each a sentence with a choice: "Mentions
 * in #web-team start sessions in [Web]". Elsewhere, fctl asks whoever mentions it which project,
 * unless they start sessions in one alone.
 */
function Mappings({ w, c, admin }: { w: G.Workspace; c: G.SlackConnection; admin: boolean }) {
  const mapped = mappedChannels(c);
  const channel = useChannel(w);
  const [picking, setPicking] = useState(false);
  // A channel picked, whose project is still to choose.
  const [adding, setAdding] = useState<G.SlackChannelChoice | null>(null);
  const projects = c.projects ?? [];
  const options: SelectOption[] = projects.map((p) => ({ value: p.id, label: p.name }));
  const choose = (ch: { id: string; name: string }, project: string) =>
    channel.mutate(
      {
        team: c.team,
        channel: ch.id,
        name: ch.name,
        b: { name: ch.name, project },
        project: projects.find((p) => p.id === project)?.name,
      },
      { onSuccess: () => setAdding(null) },
    );
  return (
    <div className="mt-6">
      <h3 className="text-[13.5px] font-medium text-ink">Which project</h3>
      <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
        A mention in a channel chosen here starts a session in its project. Elsewhere, fctl asks whoever wrote
        which project, unless they start sessions in one alone.
      </p>
      {mapped.length > 0 || adding ? (
        <ul className="mt-3 divide-y divide-line border-y border-line">
          {mapped.map((m) => (
            <li key={m.id} className="flex items-center gap-2 py-2">
              <p className="flex min-w-0 flex-1 flex-wrap items-center gap-x-1.5 gap-y-1 text-[13.5px] text-ink-2">
                <span className="inline-flex min-w-0 items-center gap-1.5">
                  Mentions in <ChannelName c={m} />
                </span>{" "}
                <span className="inline-flex items-center gap-1.5">
                  start sessions in
                  {admin ? (
                    <Select
                      size="sm"
                      aria-label={`The project mentions in #${m.name} start sessions in`}
                      value={m.project?.id}
                      options={options}
                      onChange={(p) => choose(m, p)}
                    />
                  ) : (
                    <span className="font-medium text-ink">{m.project?.name}</span>
                  )}
                </span>
              </p>
              {admin && (
                <IconButton
                  type="button"
                  label={`Stop starting sessions in ${m.project?.name ?? "a project"} from #${m.name}`}
                  onClick={() =>
                    channel.mutate({ team: c.team, channel: m.id, name: m.name, b: { project: "" } })
                  }
                >
                  <Trash2 className="size-3.5" />
                </IconButton>
              )}
            </li>
          ))}
          {adding && (
            <li className="flex animate-rise items-center gap-2 py-2">
              <p className="flex min-w-0 flex-1 flex-wrap items-center gap-x-1.5 gap-y-1 text-[13.5px] text-ink-2">
                <span className="inline-flex min-w-0 items-center gap-1.5">
                  Mentions in <ChannelName c={adding} />
                </span>{" "}
                <span className="inline-flex items-center gap-1.5">
                  start sessions in
                  <Select
                    size="sm"
                    aria-label={`The project mentions in #${adding.name} start sessions in`}
                    value={undefined}
                    placeholder="Choose a project"
                    options={options}
                    disabled={channel.isPending}
                    onChange={(p) => choose(adding, p)}
                  />
                </span>
              </p>
              <IconButton type="button" label="Don't add it" onClick={() => setAdding(null)}>
                <X className="size-3.5" />
              </IconButton>
            </li>
          )}
        </ul>
      ) : (
        <p className="mt-3 border-y border-line py-3 text-[13px] text-ink-4">
          No channel has a project of its own yet
          {admin ? ": add one, and its mentions start sessions there." : "."}
        </p>
      )}
      {admin && (
        <>
          <Button
            size="sm"
            variant="ghost"
            className="mt-2 -ml-2.5"
            icon={<Plus className="size-3.5" />}
            onClick={() => setPicking(true)}
            disabled={projects.length === 0}
          >
            Add a channel
          </Button>
          {projects.length === 0 && (
            <p className="mt-1 text-[12.5px] text-ink-4">
              {w.name} has no project yet for a channel to start sessions in.
            </p>
          )}
          <ChannelPicker
            workspace={w.id}
            team={c.team}
            open={picking}
            onClose={() => setPicking(false)}
            title="Choose a channel"
            description="Its mentions start sessions in the project you choose next. fctl joins it, which shows there."
            note="Private channels aren't listed: invite fctl there, mention it, and tick Remember for the channel as you choose the project."
            taken={(ch) => {
              const m = mapped.find((x) => x.id === ch.id);
              return m ? `Starts sessions in ${m.project?.name ?? "a project"}` : undefined;
            }}
            onPick={(ch) => setAdding(ch)}
          />
        </>
      )}
    </div>
  );
}

/** The public channels agents read, as chips; its admins add and take them away. */
function Readable({ w, c, admin }: { w: G.Workspace; c: G.SlackConnection; admin: boolean }) {
  const readable = readableChannels(c);
  const channel = useChannel(w);
  const [picking, setPicking] = useState(false);
  return (
    <div>
      <h3 className="text-[13.5px] font-medium text-ink">Readable channels</h3>
      {readable.length === 0 && !admin ? (
        <p className="mt-1.5 text-[13px] text-ink-4">None: agents read only their own thread.</p>
      ) : (
        <ul aria-label="Readable channels" className="mt-2 flex flex-wrap gap-1.5">
          {readable.map((ch) => (
            <li
              key={ch.id}
              className={cn(
                "inline-flex h-7 items-center gap-1 rounded-full border border-line bg-surface pl-2 text-[12.5px] text-ink-2",
                admin ? "pr-0.5" : "pr-2.5",
              )}
            >
              <Hash aria-hidden="true" className="size-3 text-ink-4" />
              {ch.name}
              {admin && (
                <button
                  type="button"
                  aria-label={`Stop agents reading #${ch.name}`}
                  title={`Stop agents reading #${ch.name}`}
                  disabled={channel.isPending}
                  onClick={() =>
                    channel.mutate({ team: c.team, channel: ch.id, name: ch.name, b: { readable: false } })
                  }
                  className="ml-0.5 flex size-6 items-center justify-center rounded-full text-ink-4 transition-colors hover:bg-hover hover:text-ink"
                >
                  <X className="size-3" />
                </button>
              )}
            </li>
          ))}
          {admin && (
            <li>
              <button
                type="button"
                onClick={() => setPicking(true)}
                className="inline-flex h-7 items-center gap-1 rounded-full border border-dashed border-line-strong px-2.5 text-[12.5px] text-ink-3 transition-colors hover:border-solid hover:bg-hover hover:text-ink"
              >
                <Plus className="size-3" aria-hidden="true" />
                {readable.length === 0 ? "Make a channel readable" : "Add a channel"}
              </button>
            </li>
          )}
        </ul>
      )}
      {admin && (
        <ChannelPicker
          workspace={w.id}
          team={c.team}
          open={picking}
          onClose={() => setPicking(false)}
          title="Make a channel readable"
          description="Agents of the projects that read Slack read its messages and threads. fctl joins it, which shows there."
          note="Private channels aren't listed: there, agents read their own thread alone."
          taken={(ch) => (readable.some((x) => x.id === ch.id) ? "Readable already" : undefined)}
          onPick={(ch) =>
            channel.mutate({
              team: c.team,
              channel: ch.id,
              name: ch.name,
              b: { name: ch.name, readable: true },
            })
          }
        />
      )}
    </div>
  );
}

/** Each project's choice of whether its agents read the readable channels, a click from its settings. */
function ProjectsRead({ w, c }: { w: G.Workspace; c: G.SlackConnection }) {
  const projects = c.projects ?? [];
  const mine = useProjects().data?.projects;
  const [all, setAll] = useState(false);
  const list = useId();
  if (projects.length === 0) return null;
  // Those that read none first: they're the exception. A long list folds after a few.
  const ordered = [...projects].sort((a, b) => Number(a.read_channels) - Number(b.read_channels));
  const folds = ordered.length > FOLD_AFTER + 2;
  const shown = folds && !all ? ordered.slice(0, FOLD_AFTER) : ordered;
  const manages = (id: string) => {
    const role = mine?.find((p) => p.id === id)?.role;
    return role === "owner" || role === "maintainer";
  };
  return (
    <div className="mt-6">
      <h3 className="text-[13.5px] font-medium text-ink">Each project chooses</h3>
      <p className="mt-1 text-[12.5px] leading-relaxed text-ink-3">
        Whether its agents read the readable channels: in its settings, under Slack.
      </p>
      <ul id={list} className="mt-2 divide-y divide-line border-y border-line">
        {shown.map((p) => (
          <li key={p.id} className="flex items-center gap-3 py-2 text-[13px]">
            <span className="min-w-0 flex-1 truncate text-ink">{p.name}</span>
            <span className={cn("shrink-0", p.read_channels ? "text-ink-2" : "text-ink-4")}>
              {p.read_channels ? "Reads them" : "Reads its threads alone"}
            </span>
            {manages(p.id) ? (
              <Link
                to="/p/$slug"
                params={{ slug: p.slug }}
                search={{ tab: "slack" }}
                aria-label={`${p.name}'s Slack settings`}
                className="shrink-0 rounded p-1 text-ink-4 transition-colors hover:text-ink"
              >
                <ArrowRight className="size-3.5" />
              </Link>
            ) : (
              <span className="w-[22px] shrink-0" />
            )}
          </li>
        ))}
      </ul>
      {folds && (
        <button
          type="button"
          aria-expanded={all}
          aria-controls={list}
          onClick={() => setAll(!all)}
          className="mt-2 text-[12.5px] text-ink-3 transition-colors hover:text-ink"
        >
          {all ? "Show fewer" : `Show ${ordered.length - FOLD_AFTER} more`}
        </button>
      )}
      {w.projects > projects.length && (
        <p className="mt-2 text-[12px] text-ink-4">
          And {w.projects - projects.length === 1 ? "1 project" : `${w.projects - projects.length} projects`}{" "}
          you aren't a member of: their maintainers choose.
        </p>
      )}
    </div>
  );
}

/** How many projects the list shows before it folds the rest behind a button. */
const FOLD_AFTER = 6;

/** The projects that post their events to a channel, each in a sentence. */
function ProjectEvents({ c, on }: { c: G.SlackConnection; on: boolean }) {
  const posting = (c.projects ?? []).filter((p) => p.events);
  if (posting.length === 0) return null;
  return (
    <ul className={cn("mt-4 flex flex-col gap-1.5", !on && "opacity-60")}>
      {posting.map((p) => (
        <li key={p.id} className="flex gap-2 text-[12.5px] leading-relaxed text-ink-3">
          <Hash aria-hidden="true" className="mt-[3px] size-3.5 shrink-0 text-ink-4" />
          <span className="min-w-0">
            <span className="text-ink-2">{p.name}</span> {p.events && lowerFirst(eventsSentence(p.events))}
            {!on && " Not while projects' events are off."}
          </span>
        </li>
      ))}
    </ul>
  );
}

const lowerFirst = (s: string) => s.charAt(0).toLowerCase() + s.slice(1);

/** Whether transcripts keep what Slack's messages said once their session ends. */
function Transcripts({
  w,
  value,
  admin,
  onChange,
}: {
  w: G.Workspace;
  value: G.SlackTranscripts;
  admin: boolean;
  onChange: (v: G.SlackTranscripts) => void;
}) {
  const options: SelectOption<G.SlackTranscripts>[] = [
    {
      value: "keep",
      label: "Keep them",
      description: "As anything else in a transcript: what agents read and the messages that reached them.",
    },
    {
      value: "drop",
      label: "Drop them once a session ends",
      description: "fctl keeps who wrote each message and where, and drops what it said.",
    },
  ];
  const id = useId();
  return (
    <div className="flex flex-col gap-2">
      <label htmlFor={id} className="text-[12.5px] font-medium text-ink-2">
        What Slack's messages said
      </label>
      {admin ? (
        <span className="self-start">
          <Select id={id} value={value} options={options} onChange={onChange} />
        </span>
      ) : (
        <p id={id} className="text-[13.5px] text-ink">
          {options.find((o) => o.value === value)?.label}
        </p>
      )}
      <p className="text-[12.5px] leading-relaxed text-ink-3">
        {value === "drop"
          ? `Within about an hour after a session of ${w.name}'s stops, fails or is archived, its transcript keeps who wrote each Slack message and where, and drops what they said and what its agent read.`
          : `Transcripts keep what Slack's messages said and what agents read there, for as long as ${w.name} keeps its sessions.`}
      </p>
    </div>
  );
}

/** Disconnecting a Slack workspace, behind its danger zone and a question. */
function Disconnect({ w, c }: { w: G.Workspace; c: G.SlackConnection }) {
  const [asking, setAsking] = useState(false);
  const disconnect = useMutation({
    mutationFn: () => workspaces.disconnectSlack(w.id, c.team),
    onSuccess: (s) => {
      queryClient.setQueryData(keys.workspaceSlack(w.id), s);
      void queryClient.invalidateQueries({ queryKey: keys.mySlack });
      setAsking(false);
      toast(`Disconnected ${c.name}: what people ask there starts nothing now`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const consequences = `${w.name}'s people can't start agents from ${c.name} any more, and the choices of its channels go. Sessions it started carry on, without posting there. Once no workspace works in ${c.name}, fctl's app removes itself from it.`;
  return (
    <DangerZone
      title={`Disconnect ${c.name}`}
      description={consequences}
      action="Disconnect…"
      onAction={() => setAsking(true)}
    >
      <Dialog
        open={asking}
        onClose={() => setAsking(false)}
        title={`Stop working in ${c.name}?`}
        description={consequences}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setAsking(false)}>
            Cancel
          </Button>
          <Button variant="danger" loading={disconnect.isPending} onClick={() => disconnect.mutate()}>
            Disconnect
          </Button>
        </div>
      </Dialog>
    </DangerZone>
  );
}

// A project's ticket tracker: where its agents read and write tickets (none, GitHub Issues or
// Linear), and for Linear, the team whose issues people delegate to fctl there, how a ticket
// becomes a session, and the sessions tickets started lately. The project's maintainers choose;
// everyone in it sees what was chosen, and links their own Linear account here.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import {
  ArrowUpRight,
  AtSign,
  Ban,
  Check,
  Compass,
  GitPullRequestArrow,
  MessagesSquare,
  Play,
  RotateCcw,
} from "lucide-react";
import { type ReactNode, useId, useMemo, useState } from "react";
import { StateDot } from "@/components/session/ticket";
import {
  Avatar,
  Banner,
  Button,
  Field,
  GithubIcon,
  Input,
  LinearIcon,
  Logo,
  Select,
  type SelectOption,
  Skeleton,
  StatusDot,
  Switch,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api, linearLinkUrl, workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { agoShort, stamp } from "@/lib/format";
import { keys, queryClient, useInstance } from "@/lib/queries";
import { type LinearOutcome, linearOutcome, stateTone, TRACKERS, trackerOf } from "@/lib/tickets";
import { useWorkspaceIndex } from "@/lib/workspaces";

const DEFAULT_EXPLORE = "explore";

/** The choice of every repository, or every issue of a team, in a select (an empty value reads as none). */
const ALL = "all";

export function TicketsTab({ project, returned }: { project: G.Project; returned?: LinearOutcome }) {
  const manage = project.role === "owner" || project.role === "maintainer";
  const status = useQuery({
    queryKey: keys.projectTickets(project.id),
    queryFn: () => api.projectTickets(project.id),
  });
  const { data: instance } = useInstance();
  const index = useWorkspaceIndex();
  const workspace = project.workspace ? index.get(project.workspace.id) : undefined;
  const admin = workspace?.role === "owner" || workspace?.role === "admin";
  const linear = useQuery({
    queryKey: keys.workspaceLinear(project.workspace?.id ?? ""),
    queryFn: () => workspaces.linear(project.workspace?.id ?? ""),
    enabled: !!project.workspace && !!instance?.features.linear,
  });
  const saved = project.settings.tickets ?? {};
  // Back from Linear, Linear is chosen again, as it was when the browser left for it: it can't have been
  // saved, since saving it takes a team.
  const [draft, setDraft] = useState<G.Tickets>(() =>
    returned && manage && trackerOf(project.settings) !== "linear" ? { ...saved, tracker: "linear" } : saved,
  );
  // A save, or another tab's, brings the draft back to what is stored.
  const stored = JSON.stringify(project.settings.tickets);
  const [was, setWas] = useState(stored);
  if (stored !== was) {
    setWas(stored);
    setDraft(project.settings.tickets ?? {});
  }
  const tracker = draft.tracker ?? "github";
  const dirty = JSON.stringify(normalized(draft)) !== JSON.stringify(normalized(saved));
  const save = useMutation({
    mutationFn: () =>
      api.updateProject(project.id, { settings: { ...project.settings, tickets: normalized(draft) } }),
    onSuccess: (p) => {
      queryClient.setQueryData(keys.project(project.slug), p);
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      void queryClient.invalidateQueries({ queryKey: keys.projectTickets(project.id) });
      toast("Saved: agents take it from their next session, and their GitHub tokens within the hour");
    },
  });
  const outcome = returned && linearOutcome(returned);
  const s = status.data;
  return (
    <div className="flex flex-col gap-10">
      {outcome && (
        <Banner tone={outcome.tone} className="-mb-4">
          {outcome.text}
        </Banner>
      )}
      <section aria-labelledby="tracker-heading" className="animate-rise">
        <h2
          id="tracker-heading"
          className="font-display text-[26px] leading-[1.15] font-medium tracking-[-0.018em] text-ink sm:text-[28px]"
        >
          Where agents track their work
        </h2>
        <p className="mt-2 max-w-2xl text-[14.5px] leading-relaxed text-ink-3">
          The project's ticket tracker is where its agents read what a ticket asks, file what they find and
          answer the people there. Agents reach this one alone: fctl holds its credentials, and their GitHub
          tokens reach issues only where it says.
        </p>
        <TrackerPicker
          value={tracker}
          onChange={(t) => setDraft((d) => ({ ...d, tracker: t }))}
          disabled={!manage}
          linear={!!instance?.features.linear}
          className="mt-6"
        />
        {!manage && (
          <p className="mt-3 text-[12.5px] text-ink-4">
            The project's owners and maintainers choose its tracker.
          </p>
        )}
        <div key={tracker} className="mt-6 animate-rise">
          {tracker === "none" && <NoneDetails />}
          {tracker === "github" && (
            <GithubDetails
              project={project}
              status={s}
              draft={draft}
              onChange={setDraft}
              disabled={!manage}
              error={fieldError(save.error, "settings.tickets.repo")}
            />
          )}
          {tracker === "linear" && (
            <LinearDetails
              project={project}
              instance={instance}
              connections={linear.data}
              loading={linear.isPending && !!instance?.features.linear}
              admin={admin}
              workspaceName={workspace?.name ?? project.workspace?.name ?? "the workspace"}
              draft={draft}
              onChange={setDraft}
              disabled={!manage}
              error={save.error}
            />
          )}
        </div>
        {manage && (dirty || save.isPending) && (
          <SaveBar
            pending={save.isPending}
            disabled={tracker === "linear" && !draft.linear?.team}
            hint={
              tracker === "linear" && !draft.linear?.team
                ? "Choose the team first."
                : "Running sessions take it within the hour."
            }
            onSave={() => save.mutate()}
            onReset={() => {
              setDraft(saved);
              save.reset();
            }}
          />
        )}
        {save.error && !fieldError(save.error, "settings.tickets.repo") && (
          <Banner tone="error" className="mt-4">
            {save.error.message}
          </Banner>
        )}
        {!dirty && s?.problem && (
          <Banner tone="warn" className="mt-4">
            {s.problem}
          </Banner>
        )}
      </section>
      {!dirty && trackerOf(project.settings) === "linear" && project.settings.tickets?.linear && (
        <Delegating project={project} status={s} />
      )}
      <Recent project={project} status={s} loading={status.isPending} />
    </div>
  );
}

/** What a draft comes to as settings: Linear's choice is kept while another tracker is chosen. */
function normalized(t: G.Tickets): G.Tickets {
  const out: G.Tickets = { tracker: t.tracker ?? "github" };
  if (t.repo) out.repo = t.repo;
  if (t.linear) out.linear = t.linear;
  return out;
}

function fieldError(error: Error | null, field: string): string | undefined {
  return error instanceof ApiFailure && error.field?.startsWith(field) ? error.message : undefined;
}

// ───────────────────────────── the choice ─────────────────────────────

const TRACKER_ICON: Record<G.Tracker, ReactNode> = {
  none: <Ban className="size-[17px]" />,
  github: <GithubIcon className="size-[17px]" />,
  linear: <LinearIcon className="size-[16px]" />,
};

/** The three trackers as cards to choose from: radios, which the arrow keys move through. */
function TrackerPicker({
  value,
  onChange,
  disabled,
  linear,
  className,
}: {
  value: G.Tracker;
  onChange: (t: G.Tracker) => void;
  disabled: boolean;
  /** The instance has a Linear app. */
  linear: boolean;
  className?: string;
}) {
  const name = useId();
  return (
    <fieldset className={cn("grid gap-2.5 sm:grid-cols-3", className)}>
      <legend className="sr-only">Ticket tracker</legend>
      {TRACKERS.map((t) => {
        const checked = value === t.value;
        const unavailable = t.value === "linear" && !linear && !checked;
        const off = (disabled && !checked) || unavailable;
        return (
          <label
            key={t.value}
            className={cn(
              "group relative flex items-start gap-3 rounded-xl border p-3.5 text-left transition-[background-color,border-color,box-shadow] duration-150 has-[:focus-visible]:shadow-[0_0_0_3px_var(--c-accent-soft)]",
              checked
                ? "border-accent/55 bg-accent-soft/40 shadow-[inset_0_0_0_1px_var(--c-accent-soft)]"
                : "border-line bg-surface/60",
              !checked && !off && "cursor-pointer hover:border-line-strong hover:bg-hover",
              off && "opacity-55",
              disabled && checked && "cursor-default",
            )}
          >
            <input
              type="radio"
              name={name}
              value={t.value}
              checked={checked}
              disabled={off}
              onChange={() => onChange(t.value)}
              className="sr-only"
            />
            <span
              aria-hidden="true"
              className={cn(
                "flex size-8 shrink-0 items-center justify-center rounded-lg border transition-colors",
                checked ? "border-accent/40 bg-raised text-ink" : "border-line bg-raised/70 text-ink-3",
              )}
            >
              {TRACKER_ICON[t.value]}
            </span>
            <span className="min-w-0 flex-1">
              <span className="flex items-center justify-between gap-2 text-[14px] font-medium text-ink">
                {t.label}
                <span
                  aria-hidden="true"
                  className={cn(
                    "flex size-4 shrink-0 items-center justify-center rounded-full border transition-colors",
                    checked ? "border-highlight bg-highlight text-highlight-ink" : "border-line-strong",
                  )}
                >
                  {checked && <Check className="size-2.5" strokeWidth={3} />}
                </span>
              </span>
              <span className="mt-1 block text-[12.5px] leading-snug text-ink-3">
                {unavailable ? "This instance has no Linear app yet." : t.summary}
              </span>
            </span>
          </label>
        );
      })}
    </fieldset>
  );
}

/** A row of what a choice comes to: an icon, and a sentence. */
function Point({ icon, children }: { icon: ReactNode; children: ReactNode }) {
  return (
    <li className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
      <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-4 [&_svg]:size-4">
        {icon}
      </span>
      <span className="min-w-0">{children}</span>
    </li>
  );
}

function NoneDetails() {
  return (
    <ul className="flex max-w-2xl flex-col gap-3">
      <Point icon={<Ban />}>
        Agents read and write no tickets, on GitHub or anywhere else, and leave issue references out of their
        commits and pull requests.
      </Point>
      <Point icon={<GithubIcon />}>
        Their GitHub tokens carry no issues permission, so GitHub refuses them the issues of private
        repositories. A public repository's issues are open to anyone: there, what agents are told keeps them
        away.
      </Point>
      <Point icon={<MessagesSquare />}>
        Pull requests stay theirs to open, comment on and label, as the project's GitHub access allows.
      </Point>
    </ul>
  );
}

// ───────────────────────────── GitHub ─────────────────────────────

function GithubDetails({
  project,
  status,
  draft,
  onChange,
  disabled,
  error,
}: {
  project: G.Project;
  status?: G.ProjectTickets;
  draft: G.Tickets;
  onChange: (t: G.Tickets) => void;
  disabled: boolean;
  error?: string;
}) {
  const writable = status?.writable ?? (project.repo ? [project.repo] : []);
  const name = (r: G.RepoRef) => `${r.owner}/${r.name}`;
  const options: SelectOption[] = [
    {
      value: ALL,
      label: writable.length === 1 ? `${name(writable[0] as G.RepoRef)}` : "Every repository agents write to",
      description:
        writable.length > 1
          ? writable.map(name).join(", ")
          : "With gh, as agents always could. They also read the issues of repositories they only read.",
      icon: <GithubIcon className="size-3.5" />,
    },
    ...(writable.length > 1
      ? writable.map((r) => ({
          value: name(r),
          label: `Only ${name(r)}`,
          description: "Agents reach no other repository's issues.",
          icon: <GithubIcon className="size-3.5" />,
        }))
      : []),
  ];
  if (draft.repo && !options.some((o) => o.value === name(draft.repo as G.RepoRef)))
    options.push({
      value: name(draft.repo),
      label: `Only ${name(draft.repo)}`,
      description: "No longer writable",
    });
  const one = draft.repo;
  if (writable.length === 0)
    return (
      <Banner tone="info" className="max-w-2xl">
        The project has no repository yet, so its agents have no GitHub issues to reach. Link one under{" "}
        <Link
          to="/p/$slug"
          params={{ slug: project.slug }}
          search={{ tab: "repositories" }}
          className="underline underline-offset-2"
        >
          Repositories
        </Link>
        .
      </Banner>
    );
  return (
    <div className="grid gap-6 md:grid-cols-[minmax(0,1fr)_minmax(0,1.1fr)] md:gap-10">
      <Field
        label="Issues of"
        error={error}
        hint={
          writable.length > 1
            ? "One repository keeps agents to its issues: GitHub's tokens hold one set of permissions for all of them, so fctl reaches that one's for agents."
            : "The repository agents write to."
        }
      >
        {(id) => (
          <Select
            id={id}
            value={one ? name(one) : ALL}
            onChange={(v) => {
              const [owner, repo] = v === ALL ? [] : v.split("/");
              onChange({ ...draft, repo: owner && repo ? { owner, name: repo } : undefined });
            }}
            options={options}
            disabled={disabled || writable.length < 2}
            className="w-full"
          />
        )}
      </Field>
      <ul className="flex flex-col gap-3 md:pt-6">
        {one && writable.length > 1 ? (
          <>
            <Point icon={<Logo withWord={false} className="[&_svg]:size-4" />}>
              Agents read, open and comment on {name(one)}'s issues with <Code>fctl ticket</Code>, through
              fctl, which holds a token for that repository alone.
            </Point>
            <Point icon={<Ban />}>Their own tokens reach no issues at all.</Point>
          </>
        ) : (
          <>
            <Point icon={<GithubIcon />}>
              Agents read, open and comment on issues with <Code>gh issue</Code>, and write{" "}
              <Code>Fixes #12</Code> in a pull request that does what an issue asks.
            </Point>
            <Point icon={<Ban />}>Agents that only read on GitHub read issues alone.</Point>
          </>
        )}
      </ul>
    </div>
  );
}

function Code({ children }: { children: ReactNode }) {
  return <code className="rounded bg-sunken px-1 py-px font-mono text-[12px] text-ink-2">{children}</code>;
}

// ───────────────────────────── Linear ─────────────────────────────

function LinearDetails({
  project,
  instance,
  connections,
  loading,
  admin,
  workspaceName,
  draft,
  onChange,
  disabled,
  error,
}: {
  project: G.Project;
  instance?: G.InstanceInfo;
  connections?: G.WorkspaceLinear;
  loading: boolean;
  admin: boolean;
  workspaceName: string;
  draft: G.Tickets;
  onChange: (t: G.Tickets) => void;
  disabled: boolean;
  error: Error | null;
}) {
  const connect = useMutation({
    mutationFn: () => workspaces.connectLinear(project.workspace?.id ?? "", { project: project.slug }),
    // On to Linear, which brings the browser back here.
    onSuccess: (link) => window.location.assign(link.url),
    onError: (e) => toast(e.message, "error"),
  });
  if (instance && !instance.features.linear)
    return (
      <Banner tone="info" className="max-w-2xl">
        This instance has no Linear app yet. Its admins set one up: the self-hosting guide says how.
      </Banner>
    );
  if (loading) return <Skeleton className="h-28" />;
  const all = connections?.connections ?? [];
  const chosen =
    all.find((c) => c.organization === draft.linear?.organization) ?? (all.length === 1 ? all[0] : undefined);
  if (all.length === 0)
    return (
      <ConnectCard
        admin={admin}
        workspaceName={workspaceName}
        pending={connect.isPending || connect.isSuccess}
        onConnect={() => connect.mutate()}
      />
    );
  return (
    <div className="flex flex-col gap-6">
      <div className="flex flex-wrap items-center gap-3 rounded-xl border border-line bg-surface/60 px-4 py-3">
        <span className="flex size-8 shrink-0 items-center justify-center rounded-lg border border-line bg-raised text-ink">
          <LinearIcon className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          {all.length > 1 ? (
            <Select
              value={chosen?.organization}
              onChange={(v) =>
                onChange({
                  ...draft,
                  linear: {
                    organization: v,
                    team: "",
                    move_started: draft.linear?.move_started,
                    move_review: draft.linear?.move_review,
                    move_done: draft.linear?.move_done,
                  },
                })
              }
              options={all.map((c) => ({
                value: c.organization,
                label: c.name,
                description: `linear.app/${c.url_key}`,
              }))}
              placeholder="Choose a Linear workspace"
              size="sm"
              disabled={disabled}
              aria-label="Linear workspace"
            />
          ) : (
            <p className="text-[14px] text-ink">{chosen?.name}</p>
          )}
          {chosen && (
            <p className="mt-0.5 truncate text-[12px] text-ink-4">
              <a
                href={`https://linear.app/${chosen.url_key}`}
                target="_blank"
                rel="noreferrer"
                className="transition-colors hover:text-ink"
              >
                linear.app/{chosen.url_key}
              </a>
              {chosen.connected_by && ` · connected by ${chosen.connected_by.name}`}
            </p>
          )}
        </div>
        {chosen?.status === "revoked" ? (
          admin ? (
            <Button
              size="sm"
              icon={<RotateCcw className="size-3.5" />}
              loading={connect.isPending || connect.isSuccess}
              onClick={() => connect.mutate()}
            >
              Connect again
            </Button>
          ) : (
            <span className="text-[12.5px] text-amber">Access revoked</span>
          )
        ) : (
          <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
            <Check className="size-3.5" /> Connected
          </span>
        )}
      </div>
      {chosen && (
        <LinearChoice
          project={project}
          connection={chosen}
          draft={draft}
          onChange={onChange}
          disabled={disabled}
          error={error}
        />
      )}
    </div>
  );
}

/** For a workspace that works in no Linear workspace yet: connecting one, or who does. */
function ConnectCard({
  admin,
  workspaceName,
  pending,
  onConnect,
}: {
  admin: boolean;
  workspaceName: string;
  pending: boolean;
  onConnect: () => void;
}) {
  return (
    <div className="grid items-center gap-6 rounded-xl border border-line bg-surface/60 p-5 md:grid-cols-[minmax(0,1fr)_auto]">
      <div>
        <p className="flex items-center gap-2 text-[14px] font-semibold text-ink">
          <LinearIcon className="size-4" /> Connect a Linear workspace
        </p>
        <p className="mt-1.5 text-[13.5px] leading-relaxed text-ink-3">
          {admin ? (
            <>
              Installing fctl's app in a Linear workspace takes one of its admins. Linear then lists fctl as
              an agent that people there assign issues to and mention, and {workspaceName}'s projects choose
              the teams they take tickets from.
            </>
          ) : (
            <>
              {workspaceName}'s admins connect Linear: installing fctl's app there takes an admin of both. Ask
              one of them, then choose the team here.
            </>
          )}
        </p>
      </div>
      {admin && (
        <Button
          variant="primary"
          icon={<LinearIcon className="size-3.5" />}
          loading={pending}
          onClick={onConnect}
        >
          Connect Linear
        </Button>
      )}
    </div>
  );
}

/** The team, the Linear project, and what fctl does with what is delegated. */
function LinearChoice({
  project,
  connection,
  draft,
  onChange,
  disabled,
  error,
}: {
  project: G.Project;
  connection: G.LinearConnection;
  draft: G.Tickets;
  onChange: (t: G.Tickets) => void;
  disabled: boolean;
  error: Error | null;
}) {
  const workspace = project.workspace?.id ?? "";
  const teams = useQuery({
    queryKey: keys.linearTeams(workspace, connection.organization),
    queryFn: () => workspaces.linearTeams(workspace, connection.organization),
    enabled: connection.status === "active",
    staleTime: 60_000,
  });
  const current = draft.linear?.organization === connection.organization ? draft.linear : undefined;
  const team = teams.data?.teams.find((t) => t.id === current?.team);
  const set = (l: Partial<G.LinearTickets>) =>
    onChange({
      ...draft,
      linear: {
        organization: connection.organization,
        team: current?.team ?? "",
        ...current,
        ...l,
      },
    });
  const teamOptions: SelectOption[] = (teams.data?.teams ?? []).map((t) => ({
    value: t.id,
    label: t.name,
    short: t.key,
    description: t.key,
    icon: (
      <span
        aria-hidden="true"
        className="size-2.5 rounded-[3px]"
        style={{ backgroundColor: t.color ?? "var(--c-ink-4)" }}
      />
    ),
  }));
  if (current?.team && !team && current.team_name)
    teamOptions.push({
      value: current.team,
      label: current.team_name,
      description: "fctl's app no longer reaches it",
    });
  const projectOptions: SelectOption[] = [
    { value: ALL, label: team ? `Every issue of ${team.name}` : "Every issue of the team" },
    ...(team?.projects ?? []).map((p) => ({
      value: p.id,
      label: p.name,
      description: "Only this project's issues",
    })),
  ];
  const label = current?.explore_label ?? DEFAULT_EXPLORE;
  const err = (f: string) => fieldError(error, f);
  return (
    <div className="flex flex-col gap-6">
      <div className="grid items-start gap-4 sm:grid-cols-2">
        <Field label="Team" error={err("settings.tickets.linear.team") ?? err("settings.tickets.linear")}>
          {(id) =>
            teams.isPending && connection.status === "active" ? (
              <Skeleton className="h-9.5" />
            ) : (
              <Select
                id={id}
                value={current?.team || undefined}
                onChange={(v) => {
                  const t = teams.data?.teams.find((x) => x.id === v);
                  set({
                    team: v,
                    team_key: t?.key,
                    team_name: t?.name,
                    project: undefined,
                    project_name: undefined,
                  });
                }}
                options={teamOptions}
                placeholder="Choose a team"
                disabled={disabled || connection.status !== "active"}
                className="w-full"
              />
            )
          }
        </Field>
        <Field
          label="Issues of"
          hint={team && team.projects.length === 0 ? `${team.name} has no open projects.` : undefined}
          error={err("settings.tickets.linear.project")}
        >
          {(id) => (
            <Select
              id={id}
              value={current?.project ?? ALL}
              onChange={(v) => {
                const p = team?.projects.find((x) => x.id === v);
                set({ project: v === ALL ? undefined : v, project_name: p?.name });
              }}
              options={projectOptions}
              disabled={disabled || !team}
              className="w-full"
            />
          )}
        </Field>
      </div>
      {teams.error && <Banner tone="error">{teams.error.message}</Banner>}
      <TicketMoves current={current} team={team} set={set} disabled={disabled || !current?.team} />
      <Field
        label="Label that asks for an exploration"
        error={err("settings.tickets.linear.explore_label")}
        hint={
          label.trim() ? (
            <>
              A ticket labelled <span className="font-mono text-[12px] text-ink-2">{label.trim()}</span> gets
              an exploration: the agent finds out what it needs and replies with a plan, without changing
              code, until someone replies to go ahead.
            </>
          ) : (
            "No label asks for one: every delegated ticket gets the work done."
          )
        }
      >
        {(id) => (
          <span className="block w-full sm:w-60">
            <Input
              id={id}
              value={label}
              onChange={(e) => set({ explore_label: e.target.value })}
              placeholder="No label"
              disabled={disabled || !current?.team}
              autoComplete="off"
              spellCheck={false}
            />
          </span>
        )}
      </Field>
    </div>
  );
}

// ───────────────────────────── saving ─────────────────────────────

function SaveBar({
  pending,
  disabled,
  hint,
  onSave,
  onReset,
}: {
  pending: boolean;
  disabled: boolean;
  hint: string;
  onSave: () => void;
  onReset: () => void;
}) {
  return (
    <div className="sticky bottom-4 z-10 mt-8 flex animate-rise flex-wrap items-center justify-between gap-3 rounded-xl border border-line-strong bg-surface/95 px-4 py-3 shadow-[0_18px_50px_-24px_rgb(0_0_0/0.45)] backdrop-blur max-sm:bottom-2">
      <span className="text-[12.5px] text-ink-3">{hint}</span>
      <div className="flex items-center gap-2">
        <Button variant="ghost" onClick={onReset} disabled={pending}>
          Cancel
        </Button>
        <Button variant="primary" loading={pending} disabled={disabled} onClick={onSave}>
          Save
        </Button>
      </div>
    </div>
  );
}

// ───────────────────────────── delegating ─────────────────────────────

/** How a Linear ticket becomes a session, with a picture of it in Linear, and the caller's own link. */
function Delegating({ project, status }: { project: G.Project; status?: G.ProjectTickets }) {
  const l = project.settings.tickets?.linear;
  const team = l?.team_key ?? l?.team_name ?? "the team";
  // As people find fctl's app in Linear: the instance's Linear app may go by another name.
  const app = status?.linear?.app_name ?? "fctl";
  const handle = status?.linear?.app_handle ?? "fctl";
  const review = l?.move_review !== false;
  const done = l?.move_done !== false;
  const moves =
    review && done
      ? " The ticket moves to In Review when its pull request is up, and to Done once a pull request that fixes it merges."
      : review
        ? " The ticket moves to In Review when its pull request is up."
        : done
          ? " The ticket moves to Done once a pull request that fixes it merges."
          : "";
  const steps: { icon: ReactNode; text: ReactNode }[] = [
    {
      icon: <AtSign />,
      text: (
        <>
          In Linear, assign one of {team}'s issues to <span className="font-medium text-ink">{app}</span>, or
          mention <span className="font-mono text-[12.5px] text-ink">@{handle}</span> in a comment on one.
        </>
      ),
    },
    {
      icon: <Play />,
      text: (
        <>
          fctl starts a session in {project.name} for whoever asked, on their Claude subscription
          {l?.move_started !== false ? ", and starts the ticket" : ""}.
        </>
      ),
    },
    {
      icon: <GitPullRequestArrow />,
      text: `Its steps show in the ticket as it works, its pull requests link to it, and each of its replies lands in the ticket's thread.${moves}`,
    },
    {
      icon: <MessagesSquare />,
      text: "Reply in the thread to steer it, or press stop there to interrupt it. Only people fctl knows in Linear reach it.",
    },
  ];
  return (
    <section aria-labelledby="delegating-heading" className="animate-rise">
      <h2 id="delegating-heading" className="text-[14px] font-semibold text-ink">
        Delegating a ticket
      </h2>
      <div className="mt-4 grid items-start gap-x-12 gap-y-8 md:grid-cols-[minmax(0,1fr)_320px]">
        <div>
          <ol className="flex flex-col gap-3.5">
            {steps.map((s, i) => (
              <li key={i} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
                <span
                  aria-hidden="true"
                  className="flex size-6 shrink-0 items-center justify-center rounded-full border border-line bg-raised text-ink-3 [&_svg]:size-3.5"
                >
                  {s.icon}
                </span>
                <span className="min-w-0 pt-0.5">{s.text}</span>
              </li>
            ))}
          </ol>
          <YourAccount project={project} status={status} />
        </div>
        <DelegatedTicket team={team} project={project.slug} app={app} />
      </div>
    </section>
  );
}

/** Whether fctl knows the caller in the Linear workspace: what they delegate runs as them once it does. */
function YourAccount({ project, status }: { project: G.Project; status?: G.ProjectTickets }) {
  if (!status?.linear || status.linked === undefined) return null;
  const name = status.linear.name;
  if (status.linked)
    return (
      <p className="mt-6 flex items-start gap-2 text-[13px] leading-relaxed text-ink-3">
        <Check className="mt-[3px] size-3.5 shrink-0 text-accent" />
        <span>
          fctl knows your Linear account in {name}: issues you delegate there start sessions as you.{" "}
          <Link
            to="/settings"
            search={{ linked: undefined, error: undefined }}
            hash="linear"
            className="text-ink-2 underline underline-offset-2"
          >
            Settings
          </Link>
        </span>
      </p>
    );
  return (
    <div className="mt-6 flex flex-col gap-3 rounded-xl border border-blue/25 bg-blue-soft/60 p-4 sm:flex-row sm:items-center">
      <p className="min-w-0 flex-1 text-[13px] leading-relaxed text-ink-2">
        <span className="font-medium text-ink">Link your Linear account.</span> fctl starts sessions only for
        people it knows in {name}, as them. It takes a Linear sign-in, which fctl forgets at once.
      </p>
      <a href={linearLinkUrl(project.slug)} className="shrink-0">
        <Button icon={<LinearIcon className="size-3.5" />}>Link Linear account</Button>
      </a>
    </div>
  );
}

/** A switch of [`TicketMoves`]: the setting, the state it moves tickets to, and when. */
type Move = {
  key: "move_started" | "move_review" | "move_done";
  kind: G.StateKind;
  state?: string;
  when: string;
  hint: string;
};

/**
 * The states fctl moves a project's tickets to as their sessions work, each with a switch: the team's
 * first started one as a session starts, its review one when a pull request is up, and its first
 * completed one once they merged (`fctl_domain::tickets::next_state`).
 */
function TicketMoves({
  current,
  team,
  set,
  disabled,
}: {
  current?: G.LinearTickets;
  team?: G.LinearTeam;
  set: (l: Partial<G.LinearTickets>) => void;
  disabled: boolean;
}) {
  const id = useId();
  const moves: Move[] = [
    {
      key: "move_started",
      kind: "started",
      state: team?.start_state ?? "In Progress",
      when: "as its session starts",
      hint: "A delegated ticket at once, as Linear asks of agents; one that mentions fctl once its agent changes something.",
    },
    {
      key: "move_review",
      kind: "started",
      state: team?.review_state ?? "In Review",
      when: "when its pull request is up",
      hint: "When a turn of its session ends with a pull request open for review.",
    },
    {
      key: "move_done",
      kind: "completed",
      state: team?.done_state ?? "Done",
      when: "once its pull requests merge",
      hint: "Once one that completes it (Fixes, not Part of) merged and none is open. Its agent moves a ticket that needs no pull request.",
    },
  ];
  return (
    <fieldset aria-describedby={`${id}-hint`} className="flex min-w-0 flex-col gap-1.5">
      <legend className="mb-1.5 text-[12.5px] font-medium text-ink-2">Ticket states</legend>
      <p id={`${id}-hint`} className="text-[12.5px] leading-relaxed text-ink-3">
        fctl moves the tickets its sessions work on through {team ? `${team.name}'s` : "the team's"} workflow,
        never back and never out of done or canceled. Turn off what Linear's GitHub integration does for the
        team already.
      </p>
      <ul className="mt-1.5 divide-y divide-line overflow-hidden rounded-lg border border-line">
        {moves.map((m) => {
          const on = current?.[m.key] ?? true;
          return (
            <li key={m.key} className="flex items-center gap-3 px-3.5 py-2.5">
              <span className="min-w-0 flex-1">
                <span className="flex flex-wrap items-center gap-x-1.5 text-[13.5px] text-ink-2">
                  <span
                    className={cn(
                      "inline-flex items-center gap-1 font-medium transition-colors duration-200",
                      on ? stateTone(m.kind) : "text-ink-4",
                    )}
                  >
                    <StateDot kind={m.kind} />
                    {m.state}
                  </span>
                  {m.when}
                </span>
                <span className="block text-[12.5px] leading-relaxed text-ink-4">{m.hint}</span>
              </span>
              <Switch
                checked={on}
                onChange={(v) => set({ [m.key]: v })}
                label={`Move a ticket to ${m.state} ${m.when}`}
                disabled={disabled}
              />
            </li>
          );
        })}
      </ul>
    </fieldset>
  );
}

/**
 * What a delegated ticket looks like in Linear: fctl's session on it, a step, a pull request, a reply.
 * The workspace's Linear tab shows it too, before Linear is connected.
 */
export function DelegatedTicket({ team, project, app }: { team: string; project: string; app: string }) {
  const key = `${team === "the team" ? "ENG" : team}-142`;
  return (
    <div aria-hidden="true" className="relative select-none max-md:max-w-sm">
      <div className="overflow-hidden rounded-xl border border-line bg-surface shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]">
        <div className="flex items-center gap-2 border-b border-line px-3.5 py-2.5 text-[11.5px] text-ink-3">
          <LinearIcon className="size-3.5 text-ink-2" />
          <span className="font-mono">{key}</span>
          <span className="inline-flex items-center gap-1 text-blue">
            <StateDot kind="started" />
            In Review
          </span>
          <span className="ml-auto inline-flex items-center gap-1 rounded-full border border-line px-1.5 py-px">
            <Logo withWord={false} className="[&_svg]:size-3" />
            <span className="max-w-24 truncate">{app}</span>
          </span>
        </div>
        <div className="px-3.5 pt-3 pb-1">
          <p className="text-[13.5px] font-semibold text-ink">Fix the login redirect loop</p>
          <p className="mt-1 text-[12px] leading-relaxed text-ink-3">
            Signing in on Safari sends people back to /login.
          </p>
        </div>
        <ol className="flex flex-col gap-2.5 px-3.5 py-3 text-[12px]">
          <li className="flex gap-2 text-ink-3">
            <Logo withWord={false} className="mt-px shrink-0 [&_svg]:size-3.5" />
            <span className="italic">Working on this in fctl session {project}#12, as Ada.</span>
          </li>
          <li className="flex items-center gap-2 text-ink-3">
            <GitPullRequestArrow className="size-3.5 shrink-0 text-pr-open" />
            <span>
              Opened a pull request <span className="font-mono text-ink-2">web#45</span>
            </span>
          </li>
          <li className="rounded-lg border border-line bg-raised/60 px-3 py-2 leading-relaxed text-ink-2">
            The session cookie lost its SameSite setting on Safari: I set it again and added a test. Fixes{" "}
            <span className="whitespace-nowrap">{key}.</span>
          </li>
          <li className="flex gap-2 text-ink-3">
            <span className="mt-px flex size-3.5 shrink-0 items-center justify-center rounded-full bg-pink/80 text-[8px] font-semibold text-white">
              A
            </span>
            <span>Thanks! Ship it.</span>
          </li>
        </ol>
      </div>
      <div className="mt-3 flex items-center gap-2.5 rounded-xl border border-line bg-surface px-3.5 py-2.5 text-[12px] shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]">
        <Compass className="size-3.5 text-blue" />
        <span className="font-semibold text-ink">explore</span>
        <span className="truncate text-ink-3">asks for a plan first</span>
      </div>
    </div>
  );
}

// ───────────────────────────── lately ─────────────────────────────

function Recent({
  project,
  status,
  loading,
}: {
  project: G.Project;
  status?: G.ProjectTickets;
  loading: boolean;
}) {
  const heading = useId();
  const runs = status?.recent ?? [];
  const linear = trackerOf(project.settings) === "linear";
  const items = useMemo(() => runs.slice(0, 10), [runs]);
  if (loading) return <Skeleton className="h-24" />;
  if (items.length === 0 && !linear) return null;
  return (
    <section aria-labelledby={heading}>
      <h2 id={heading} className="mb-2 text-[14px] font-semibold text-ink">
        Tickets lately
      </h2>
      {items.length === 0 ? (
        <p className="border-t border-line py-6 text-[13.5px] text-ink-4">
          No ticket has started a session yet. The first one you delegate shows here.
        </p>
      ) : (
        <ul className="border-t border-line">
          {items.map((r) => (
            <li
              key={r.session.id}
              className="group relative isolate flex items-center gap-3 border-b border-line py-2.5 transition-colors hover:bg-hover/30"
            >
              <StatusDot status={r.status} />
              <a
                href={r.ticket.url}
                target="_blank"
                rel="noreferrer"
                title={r.ticket.title ? `${r.ticket.key}: ${r.ticket.title}` : r.ticket.key}
                className="relative z-[2] inline-flex w-[5.5rem] shrink-0 items-center gap-1 font-mono text-[12px] text-ink-3 transition-colors hover:text-ink max-sm:w-auto"
              >
                {r.ticket.tracker === "linear" ? (
                  <LinearIcon className="size-3 shrink-0" />
                ) : (
                  <GithubIcon className="size-3 shrink-0" />
                )}
                <span className="truncate">{r.ticket.key}</span>
              </a>
              <Link
                to="/s/$id"
                params={{ id: r.session.id }}
                className="min-w-0 flex-1 truncate text-[14px] text-ink outline-none after:absolute after:inset-0 after:z-[1] focus-visible:after:outline-2 focus-visible:after:outline-offset-2 focus-visible:after:outline-accent"
              >
                {r.session.title}
              </Link>
              <span className="shrink-0 font-mono text-[12px] text-ink-4 max-sm:hidden">
                #{r.session.number}
              </span>
              {r.by && <Avatar user={r.by} size={20} />}
              <time
                dateTime={new Date(r.at).toISOString()}
                title={stamp(r.at)}
                className="w-10 shrink-0 text-right text-[12px] text-ink-4 tabular-nums"
              >
                {agoShort(r.at)}
              </time>
              <ArrowUpRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-opacity group-hover:opacity-100 max-sm:hidden" />
            </li>
          ))}
        </ul>
      )}
    </section>
  );
}

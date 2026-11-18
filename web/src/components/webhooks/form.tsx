// Adding or changing a webhook: where it posts, what it hears of (every event, or those chosen, from
// ready sets or one by one), a workspace's projects, and whose sessions; with what it will post said
// in a sentence as it's chosen (describeWebhook), as the CLI says it.

import { useMutation } from "@tanstack/react-query";
import { Check, Minus } from "lucide-react";
import { type ReactNode, useId, useMemo, useState } from "react";
import { ProjectMark } from "@/components/project/project-icon";
import { Banner, Button, Field, Input } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, queryClient, useInstance } from "@/lib/queries";
import { describeWebhook, hostOf, ORIGINS, PRESETS, presetOf, urlProblem } from "@/lib/webhooks";

/** Whose webhooks: a workspace's (for all its projects, or some), or a project's own. */
export type HookScope =
  | { kind: "workspace"; workspace: G.Workspace }
  | { kind: "project"; project: G.Project };

export const scopeId = (s: HookScope) => (s.kind === "workspace" ? s.workspace.id : s.project.id);
export const scopeName = (s: HookScope) => (s.kind === "workspace" ? s.workspace.name : s.project.name);

/** What a change to webhooks touches: the lists that hold them, and the webhook. */
export function refreshWebhooks(id?: string) {
  void queryClient.invalidateQueries({ queryKey: ["webhooks"] });
  if (id) void queryClient.invalidateQueries({ queryKey: keys.webhook(id) });
}

export function WebhookForm({
  scope,
  webhook,
  catalog,
  groups,
  projects,
  onDone,
}: {
  scope: HookScope;
  /** The webhook it changes; none adds one. */
  webhook?: G.Webhook;
  catalog: G.WebhookEventInfo[];
  groups: G.WebhookEventGroup[];
  /** The workspace's projects, which a workspace's webhook may be narrowed to. */
  projects: G.Project[];
  onDone: (result?: { created?: G.CreatedWebhook; saved?: G.Webhook }) => void;
}) {
  const { data: instance } = useInstance();
  const dev = !!instance?.features.local_webhooks;
  const [url, setUrl] = useState(webhook?.url ?? "");
  const [name, setName] = useState(webhook?.name ?? "");
  const [every, setEvery] = useState(webhook ? webhook.events.length === 0 : false);
  const [events, setEvents] = useState<string[]>(webhook?.events ?? PRESETS[0]?.events ?? []);
  const [someProjects, setSomeProjects] = useState((webhook?.projects.length ?? 0) > 0);
  const [chosenProjects, setChosenProjects] = useState<string[]>(webhook?.projects ?? []);
  const [startedBy, setStartedBy] = useState<G.WebhookOrigin[]>(webhook?.started_by ?? []);
  const [touched, setTouched] = useState(false);
  const workspaceHook = scope.kind === "workspace" && !webhook?.project;

  const chosen = every ? [] : events;
  const sessionsChosen = every || catalog.some((e) => e.sessions && events.includes(e.type));
  const projectNames = useMemo(
    () => (someProjects ? projects.filter((p) => chosenProjects.includes(p.id)).map((p) => p.name) : []),
    [someProjects, projects, chosenProjects],
  );
  const problem = urlProblem(url, dev);
  const missing = !url.trim()
    ? "Give the address fctl posts to."
    : !every && events.length === 0
      ? "Choose an event, or every event."
      : workspaceHook && someProjects && chosenProjects.length === 0
        ? "Choose a project, or every project."
        : undefined;
  const save = useMutation({
    mutationFn: async () => {
      const body = {
        url: url.trim(),
        name: name.trim(),
        events: chosen,
        projects: workspaceHook && someProjects ? chosenProjects : [],
        started_by: sessionsChosen ? startedBy : [],
      };
      if (webhook) return { saved: await api.updateWebhook(webhook.id, body) };
      const created =
        scope.kind === "workspace"
          ? await api.createWorkspaceWebhook(scope.workspace.id, { ...body, name: body.name || undefined })
          : await api.createProjectWebhook(scope.project.id, { ...body, name: body.name || undefined });
      return { created };
    },
    onSuccess: (r) => {
      refreshWebhooks(webhook?.id ?? r.created?.webhook.id);
      onDone(r);
    },
  });
  const failure = save.error instanceof ApiFailure ? save.error : undefined;
  const fieldError = (f: string) => (failure?.field === f ? failure.message : undefined);
  const ready = !problem && !missing && !save.isPending;
  const sentence =
    url.trim() && !problem
      ? describeWebhook(url.trim(), chosen, projectNames, sessionsChosen ? startedBy : [], catalog)
      : undefined;

  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        setTouched(true);
        if (ready) save.mutate();
      }}
      className={cn(
        "flex animate-rise scroll-mt-24 flex-col gap-6 rounded-2xl border border-line bg-surface/70 p-4 sm:p-5",
        !webhook && "shadow-[0_24px_60px_-40px_rgb(0_0_0/0.5)]",
      )}
      aria-label={webhook ? `Change ${webhook.name || hostOf(webhook.url)}` : "New webhook"}
    >
      <div className="flex flex-col gap-4">
        <input
          value={name}
          onChange={(e) => setName(e.target.value)}
          placeholder={url && !problem ? hostOf(url.trim()) : "Name it"}
          aria-label="Name"
          maxLength={80}
          className="w-full border-b border-transparent bg-transparent pb-1 font-display text-[20px] leading-snug font-medium tracking-[-0.01em] text-ink outline-none transition-colors placeholder:text-ink-4 hover:border-line focus:border-line-strong"
        />
        {fieldError("name") && <p className="-mt-3 text-[12.5px] text-danger">{fieldError("name")}</p>}
        <Field
          label="Address"
          error={fieldError("url") ?? ((touched || url.includes("://")) && problem ? problem : undefined)}
          hint={
            dev
              ? "fctl posts each event there as JSON, signed with its secret. This development instance also takes http:// and local addresses."
              : "fctl posts each event there as JSON, signed with its secret: an https:// address on the public internet."
          }
        >
          {(id) => (
            <Input
              id={id}
              type="url"
              inputMode="url"
              autoComplete="off"
              spellCheck={false}
              autoFocus={!webhook}
              placeholder="https://hooks.example.com/fctl"
              value={url}
              onChange={(e) => setUrl(e.target.value)}
              onBlur={() => setTouched(true)}
              className="font-mono text-[13px]"
            />
          )}
        </Field>
      </div>

      <EventsField
        catalog={catalog}
        groups={groups}
        every={every}
        events={events}
        onEvery={setEvery}
        onEvents={setEvents}
        error={fieldError("events")}
      />

      {workspaceHook && projects.length > 0 && (
        <ProjectsField
          projects={projects}
          some={someProjects}
          chosen={chosenProjects}
          onSome={setSomeProjects}
          onChosen={setChosenProjects}
          workspace={scope.kind === "workspace" ? scope.workspace.name : ""}
          error={fieldError("projects")}
        />
      )}

      {sessionsChosen && <StartedByField value={startedBy} onChange={setStartedBy} />}

      <div
        className="rounded-xl border border-line bg-sunken/60 px-4 py-3 text-[13.5px] leading-relaxed text-ink-2"
        aria-live="polite"
      >
        {sentence ?? <span className="text-ink-4">What it posts shows here once it has an address.</span>}
      </div>

      {save.error && !failure?.field && <Banner tone="error">{save.error.message}</Banner>}

      <div className="flex flex-wrap items-center justify-between gap-x-4 gap-y-3 border-t border-line pt-4">
        <p className="min-w-0 text-[12.5px] leading-snug text-ink-3">
          {touched && missing
            ? missing
            : webhook
              ? "Changes reach new events within half a minute."
              : "Adding it shows its signing secret, once."}
        </p>
        <div className="flex gap-2 max-sm:ml-auto">
          <Button variant="ghost" onClick={() => onDone()}>
            Cancel
          </Button>
          <Button type="submit" variant="primary" loading={save.isPending} disabled={touched && !ready}>
            {webhook ? "Save" : "Add webhook"}
          </Button>
        </div>
      </div>
    </form>
  );
}

/** A label above a choice, as Field has it, for choices that aren't one control. */
function Legend({ children, hint }: { children: ReactNode; hint?: ReactNode }) {
  return (
    <div className="mb-2.5">
      <legend className="text-[12.5px] font-medium text-ink-2">{children}</legend>
      {hint && <p className="mt-0.5 text-[12.5px] text-ink-3">{hint}</p>}
    </div>
  );
}

/** Two choices in a pill, as the schedules' form has its ways to run. */
function Choice<T extends string>({
  value,
  options,
  onChange,
  label,
}: {
  value: T;
  options: { value: T; label: string }[];
  onChange: (v: T) => void;
  label: string;
}) {
  return (
    <div
      role="radiogroup"
      aria-label={label}
      className="inline-flex rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)]"
    >
      {options.map((o) => (
        // biome-ignore lint/a11y/useSemanticElements: a segment of the pill, as a radio of the group.
        <button
          key={o.value}
          type="button"
          role="radio"
          aria-checked={value === o.value}
          onClick={() => onChange(o.value)}
          className={cn(
            "h-7 rounded-full px-3.5 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
            value === o.value
              ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25),inset_0_0_0_1px_var(--c-line-strong)]"
              : "text-ink-3 hover:text-ink",
          )}
        >
          {o.label}
        </button>
      ))}
    </div>
  );
}

/** A box that is ticked, not, or (for a group) partly ticked. */
function Tick({ on, some }: { on: boolean; some?: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "flex size-4 shrink-0 items-center justify-center rounded-[5px] border transition-colors duration-150",
        on || some ? "border-highlight bg-highlight text-highlight-ink" : "border-line-strong bg-sunken",
      )}
    >
      {on ? (
        <Check className="size-3" strokeWidth={3} />
      ) : some ? (
        <Minus className="size-3" strokeWidth={3} />
      ) : null}
    </span>
  );
}

function EventsField({
  catalog,
  groups,
  every,
  events,
  onEvery,
  onEvents,
  error,
}: {
  catalog: G.WebhookEventInfo[];
  groups: G.WebhookEventGroup[];
  every: boolean;
  events: string[];
  onEvery: (v: boolean) => void;
  onEvents: (v: string[]) => void;
  error?: string;
}) {
  const preset = presetOf(events);
  const toggle = (type: string) =>
    onEvents(events.includes(type) ? events.filter((e) => e !== type) : [...events, type]);
  return (
    <fieldset>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <legend className="text-[12.5px] font-medium text-ink-2">Events</legend>
        <Choice
          label="Which events"
          value={every ? "every" : "some"}
          onChange={(v) => onEvery(v === "every")}
          options={[
            { value: "some", label: "Choose events" },
            { value: "every", label: "Every event" },
          ]}
        />
      </div>
      {every ? (
        <p className="max-w-xl text-[13px] leading-relaxed text-ink-3">
          All {catalog.length} events, and those a later version of fctl adds: sessions, agents' pull
          requests, reviews and schedules. A session's status changes often, so expect many requests.
        </p>
      ) : (
        <div className="flex flex-col gap-4">
          <div className="-mx-4 flex items-center gap-2 overflow-x-auto px-4 [scrollbar-width:none] sm:-mx-5 sm:px-5">
            <span className="shrink-0 text-[12.5px] text-ink-4">Start from</span>
            {PRESETS.map((p) => (
              <button
                key={p.key}
                type="button"
                aria-pressed={preset?.key === p.key}
                title={p.line}
                onClick={() => onEvents(p.events)}
                className={cn(
                  "inline-flex h-7 shrink-0 items-center rounded-full border px-2.5 text-[12.5px] transition-colors duration-150",
                  preset?.key === p.key
                    ? "border-line-strong bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                    : "border-line text-ink-3 hover:bg-hover hover:text-ink",
                )}
              >
                {p.label}
              </button>
            ))}
          </div>
          <div className="grid gap-x-6 gap-y-5 md:grid-cols-2">
            {groups.map((g) => {
              const of = catalog.filter((e) => e.group === g.id);
              const on = of.filter((e) => events.includes(e.type)).length;
              const id = `events-${g.id}`;
              return (
                <fieldset key={g.id} aria-labelledby={id} className="min-w-0">
                  <button
                    type="button"
                    onClick={() =>
                      onEvents(
                        on === of.length
                          ? events.filter((e) => !of.some((x) => x.type === e))
                          : [
                              ...events.filter((e) => !of.some((x) => x.type === e)),
                              ...of.map((x) => x.type),
                            ],
                      )
                    }
                    aria-pressed={on === of.length}
                    className="mb-1 flex w-full items-center gap-2.5 rounded-md py-1 text-left"
                  >
                    <Tick on={on === of.length} some={on > 0} />
                    <span id={id} className="text-[13px] font-semibold text-ink">
                      {g.label}
                    </span>
                    <span className="text-[12px] text-ink-4 tabular-nums">
                      {on} of {of.length}
                    </span>
                  </button>
                  <ul className="flex flex-col">
                    {of.map((e) => {
                      const checked = events.includes(e.type);
                      return (
                        <li key={e.type}>
                          {/* biome-ignore lint/a11y/useSemanticElements: a row of the list, its tick drawn, as a checkbox. */}
                          <button
                            type="button"
                            role="checkbox"
                            aria-checked={checked}
                            onClick={() => toggle(e.type)}
                            className="group flex w-full items-start gap-2.5 rounded-md py-1.5 pr-2 pl-0 text-left transition-colors hover:bg-hover/40 pointer-coarse:py-2"
                          >
                            <span className="mt-0.5">
                              <Tick on={checked} />
                            </span>
                            <span className="min-w-0">
                              <span className="block text-[13.5px] text-ink">{e.label}</span>
                              <span className="block text-[12px] leading-snug text-ink-3">
                                {e.description}
                              </span>
                              <span className="mt-0.5 block font-mono text-[11px] text-ink-4">{e.type}</span>
                            </span>
                          </button>
                        </li>
                      );
                    })}
                  </ul>
                </fieldset>
              );
            })}
          </div>
        </div>
      )}
      {error && <p className="mt-2 text-[12.5px] text-danger">{error}</p>}
    </fieldset>
  );
}

function ProjectsField({
  projects,
  some,
  chosen,
  onSome,
  onChosen,
  workspace,
  error,
}: {
  projects: G.Project[];
  some: boolean;
  chosen: string[];
  onSome: (v: boolean) => void;
  onChosen: (v: string[]) => void;
  workspace: string;
  error?: string;
}) {
  const listId = useId();
  return (
    <fieldset>
      <div className="mb-3 flex flex-wrap items-center justify-between gap-x-4 gap-y-2">
        <legend className="text-[12.5px] font-medium text-ink-2">Projects</legend>
        <Choice
          label="Which projects"
          value={some ? "some" : "every"}
          onChange={(v) => onSome(v === "some")}
          options={[
            { value: "every", label: "Every project" },
            { value: "some", label: "Some projects" },
          ]}
        />
      </div>
      {some ? (
        <ul id={listId} className="flex flex-col rounded-xl border border-line">
          {projects.map((p) => {
            const on = chosen.includes(p.id);
            return (
              <li key={p.id} className="border-b border-line last:border-b-0">
                {/* biome-ignore lint/a11y/useSemanticElements: a row of the list, its tick drawn, as a checkbox. */}
                <button
                  type="button"
                  role="checkbox"
                  aria-checked={on}
                  onClick={() => onChosen(on ? chosen.filter((c) => c !== p.id) : [...chosen, p.id])}
                  className="flex w-full items-center gap-3 px-3 py-2 text-left transition-colors hover:bg-hover/40 pointer-coarse:py-2.5"
                >
                  <Tick on={on} />
                  <ProjectMark project={p} size={20} />
                  <span className="min-w-0 flex-1 truncate text-[13.5px] text-ink">{p.name}</span>
                  <span className="font-mono text-[11.5px] text-ink-4">{p.slug}</span>
                </button>
              </li>
            );
          })}
        </ul>
      ) : (
        <p className="text-[13px] leading-relaxed text-ink-3">
          The events of all of {workspace}'s projects, those made later too.
        </p>
      )}
      {error && <p className="mt-2 text-[12.5px] text-danger">{error}</p>}
    </fieldset>
  );
}

function StartedByField({
  value,
  onChange,
}: {
  value: G.WebhookOrigin[];
  onChange: (v: G.WebhookOrigin[]) => void;
}) {
  return (
    <fieldset>
      <Legend
        hint={
          value.length === 0
            ? "Sessions' events, and their pull requests', of sessions however they started. Choose some to hear of those alone."
            : "Sessions' events, and their pull requests', of these sessions alone. Reviews' and schedules' events come whoever started them."
        }
      >
        Sessions started by
      </Legend>
      <div className="flex flex-wrap gap-2">
        {ORIGINS.map((o) => {
          const on = value.includes(o.value);
          return (
            <button
              key={o.value}
              type="button"
              aria-pressed={on}
              title={o.line}
              onClick={() =>
                onChange(
                  on
                    ? value.filter((v) => v !== o.value)
                    : ORIGINS.map((x) => x.value).filter((v) => v === o.value || value.includes(v)),
                )
              }
              className={cn(
                "inline-flex h-8 items-center gap-2 rounded-full border px-3 text-[13px] transition-colors duration-150",
                on
                  ? "border-line-strong bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                  : "border-line text-ink-3 hover:bg-hover hover:text-ink",
              )}
            >
              {on && <Check className="size-3.5 text-ink-2" aria-hidden="true" />}
              {o.label}
            </button>
          );
        })}
      </div>
    </fieldset>
  );
}

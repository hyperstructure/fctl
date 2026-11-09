// A project's Settings tab: its pages, listed beside the one open where both fit, else the list
// alone (a phone), which opens one with the way back to it. General, Repositories and Members;
// Agents, Pull requests and Tickets; Sandbox and Secrets. Owners and maintainers see them all;
// everyone else in the project reads Repositories, Members and Tickets. Each page that edits saves
// on its own, once something changed, and asks before someone leaves it with changes unsaved. Which
// pages there are, who sees them and what the list says of each: lib/project-page.ts.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  Bot,
  Box,
  ChevronLeft,
  FolderGit2,
  GitPullRequest,
  KeyRound,
  type LucideIcon,
  SlidersHorizontal,
  Ticket,
  Users,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { GithubAccessIcon } from "@/components/github-access";
import { ProjectIconPicker } from "@/components/project/icon-picker";
import { Members } from "@/components/project/members";
import { ProjectIcon } from "@/components/project/project-icon";
import { Repositories } from "@/components/project/repositories";
import { Secrets } from "@/components/project/secrets";
import { TicketsSettings } from "@/components/project/tickets";
import { SwitchRow, SwitchRows, sameDraft, UnsavedGuard, useDraft } from "@/components/settings/form";
import { type NavEntry, type NavLinkProps, SettingsList, SettingsNav } from "@/components/settings/nav";
import {
  Banner,
  Button,
  DangerZone,
  Dialog,
  EffortIcon,
  Field,
  Input,
  SaveBar,
  Section,
  SectionLayout,
  Select,
  type SelectOption,
  Textarea,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { showHex } from "@/lib/color";
import { cpus, memory } from "@/lib/format";
import { GITHUB_ACCESS } from "@/lib/github-access";
import {
  type ImageSource,
  imageSource,
  isDevcontainer,
  remember,
  type Typed,
  withSource,
} from "@/lib/images";
import { ALIASES, chosenModel, EFFORTS, type ModelChoice, modelChoice } from "@/lib/models";
import { areDefaults, shownInstructions } from "@/lib/pr-instructions";
import { ICON_COLORS, iconDef, iconOf } from "@/lib/project-icons";
import {
  isSettingsPage,
  type ProjectSettingsPage,
  type ProjectView,
  placeOf,
  SETTINGS_GROUPS,
  settingsGists,
  settingsLabel,
  settingsPagesFor,
} from "@/lib/project-page";
import { DAY_CHOICES, DEFAULT_DAYS, lasts } from "@/lib/published";
import { keys, queryClient, useInstance, useMe, useRunners } from "@/lib/queries";
import type { Gist } from "@/lib/settings";
import type { LinearOutcome } from "@/lib/tickets";

const ICONS: Record<ProjectSettingsPage, LucideIcon> = {
  // Not the gear: that's the Settings tab right above.
  general: SlidersHorizontal,
  repositories: FolderGit2,
  members: Users,
  agents: Bot,
  "pull-requests": GitPullRequest,
  tickets: Ticket,
  sandbox: Box,
  secrets: KeyRound,
};

/** Settings' pages in their groups, as the lists show them: those someone in the project sees. */
function entries(pages: ProjectSettingsPage[]): NavEntry<ProjectSettingsPage>[][] {
  return SETTINGS_GROUPS.map((group) =>
    group
      .filter((e) => pages.includes(e.page))
      .map((e) => {
        const Icon = ICONS[e.page];
        return {
          key: e.page,
          label: e.label,
          icon: (className: string) => <Icon className={className} aria-hidden="true" />,
        };
      }),
  );
}

export function ProjectSettings({
  project,
  view,
  linear,
}: {
  project: G.Project;
  /** What the address asks for: Settings itself, or one of its pages. */
  view: ProjectView;
  /** Back from Linear, to Tickets. */
  linear?: LinearOutcome;
}) {
  const pages = settingsPagesFor(project.role);
  // The page the address names; Settings itself shows the first beside the list where both fit.
  const asked = isSettingsPage(view) ? view : undefined;
  const open = asked ?? pages[0];
  const groups = entries(pages);
  const link = (page: ProjectSettingsPage, props: NavLinkProps) => (
    <Link to="/p/$slug" params={{ slug: project.slug }} search={{ tab: page }} {...props} />
  );
  if (!open) return null;
  return (
    // Its own width (a container query) decides how it shows, as Settings' does (routes/settings.tsx):
    // the list of pages beside the page open where both fit; else the list, or the page it opened
    // with the way back to it.
    <div className="@container">
      {asked && (
        <Link
          to="/p/$slug"
          params={{ slug: project.slug }}
          search={{ tab: "settings" }}
          className="-mt-2 mb-4 -ml-1.5 inline-flex h-8 items-center gap-0.5 rounded-md pr-2 text-[14px] text-ink-3 transition-colors hover:text-ink @2xl:hidden"
        >
          <ChevronLeft className="size-[18px]" />
          Settings
        </Link>
      )}
      <div className="@2xl:grid @2xl:grid-cols-[180px_minmax(0,1fr)] @2xl:items-start @2xl:gap-10 @4xl:grid-cols-[200px_minmax(0,1fr)] @4xl:gap-14">
        <SettingsNav
          label={`${project.name}'s settings`}
          groups={groups}
          current={open}
          link={link}
          className="@max-2xl:hidden"
        />
        {!asked && <PageList project={project} groups={groups} link={link} className="@2xl:hidden" />}
        <div className={cn("min-w-0", !asked && "@max-2xl:hidden")}>
          <SectionLayout value="subpage">
            {/* Another page, or another project's, starts afresh. */}
            <SettingsBody key={`${project.id}:${open}`} page={open} project={project} linear={linear} />
          </SectionLayout>
        </div>
      </div>
    </div>
  );
}

/** Settings on a phone: the pages in their groups, each with what it says now. Each opens on its own. */
function PageList({
  project,
  groups,
  link,
  className,
}: {
  project: G.Project;
  groups: NavEntry<ProjectSettingsPage>[][];
  link: (page: ProjectSettingsPage, props: NavLinkProps) => ReactNode;
  className?: string;
}) {
  const manage = project.role === "owner" || project.role === "maintainer";
  const members = useQuery({ queryKey: keys.members(project.id), queryFn: () => api.members(project.id) });
  const secrets = useQuery({
    queryKey: keys.secrets(project.id),
    queryFn: () => api.secrets(project.id),
    enabled: manage,
  });
  const said = settingsGists(project, {
    members: members.data?.members.length,
    secrets: secrets.data?.secrets.length,
  });
  const gists: Partial<Record<ProjectSettingsPage, Gist>> = Object.fromEntries(
    Object.entries(said).flatMap(([page, text]) => (text ? [[page, { text }]] : [])),
  );
  return (
    <SettingsList
      label={`${project.name}'s settings`}
      groups={groups}
      gists={gists}
      link={link}
      className={className}
    />
  );
}

function SettingsBody({
  page,
  project,
  linear,
}: {
  page: ProjectSettingsPage;
  project: G.Project;
  linear?: LinearOutcome;
}) {
  switch (page) {
    case "general":
      return <General project={project} />;
    case "repositories":
      return <Repositories project={project} />;
    case "members":
      return <Members project={project} />;
    case "agents":
      return <Agents project={project} />;
    case "pull-requests":
      return <PullRequests project={project} />;
    case "tickets":
      return <TicketsSettings project={project} returned={linear} />;
    case "sandbox":
      return <Sandbox project={project} />;
    case "secrets":
      return <Secrets project={project} />;
  }
}

/**
 * Saves what a page changed, as `body` has it when saving, and shows the project as the control
 * plane has it now. A check of the page's own fails in `body` as the control plane's would, with
 * the field it names. `saved` hands the page what was stored, which becomes its draft: the control
 * plane tidies what it stores (it trims text), and a draft that kept what was typed would still
 * differ from it, with nothing left to save.
 */
function useSave(
  project: G.Project,
  done: string,
  body: () => G.UpdateProjectRequest,
  saved: (p: G.Project) => void,
) {
  return useMutation({
    mutationFn: () => api.updateProject(project.id, body()),
    onSuccess: (p) => {
      queryClient.setQueryData(keys.project(project.slug), p);
      saved(p);
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      toast(done);
    },
  });
}

/** The field a failed save names, if any. */
const failedField = (e: Error | null) => (e as { field?: string } | null)?.field;

/**
 * The end of a page's form: the save bar while something changed or saves, and what the control
 * plane said of no field the page shows.
 */
function PageFoot({
  project,
  page,
  dirty,
  save,
  hint,
  fields,
  onReset,
}: {
  project: G.Project;
  page: ProjectSettingsPage;
  dirty: boolean;
  save: { isPending: boolean; error: Error | null; reset: () => void };
  hint: string;
  /** The fields the page shows its errors under. */
  fields: string[];
  onReset: () => void;
}) {
  const field = failedField(save.error);
  return (
    <>
      <UnsavedGuard
        dirty={dirty && !save.isPending}
        page={settingsLabel(page)}
        at={(l) => placeOf(project, l)}
      />
      {(dirty || save.isPending) && (
        <SaveBar
          pending={save.isPending}
          hint={hint}
          onReset={() => {
            onReset();
            save.reset();
          }}
        />
      )}
      {save.error && !(field && fields.includes(field)) && (
        <Banner tone="error" className="mt-4">
          {save.error.message}
        </Banner>
      )}
    </>
  );
}

/** Parts of a page, each with its heading, under what the page starts with. */
function Parts({ children }: { children: ReactNode }) {
  return (
    <SectionLayout value="part">
      <div className="mt-14 flex flex-col">{children}</div>
    </SectionLayout>
  );
}

// ───────────────────────────── general ─────────────────────────────

/** What the General page edits. */
const generalFields = (p: G.Project) => ({
  name: p.name,
  description: p.description ?? "",
  default_branch: p.default_branch,
});

/** Settings → General: its name, description, icon and default branch, and for owners deleting it. */
function General({ project }: { project: G.Project }) {
  const { draft, set, setDraft, dirty, reset } = useDraft(generalFields(project));
  const save = useSave(
    project,
    "Saved",
    () => draft,
    (p) => setDraft(generalFields(p)),
  );
  const err = (f: string) => (failedField(save.error) === f ? save.error?.message : undefined);
  const branch = draft.default_branch.trim();
  return (
    <Section
      title="General"
      description="How people find the project, and the branch its sessions start from."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty) save.mutate();
        }}
      >
        <div className="flex flex-col gap-5">
          <Field label="Name" error={err("name")}>
            {(id) => <Input id={id} value={draft.name} onChange={(e) => set({ name: e.target.value })} />}
          </Field>
          <Field label="Description" hint="Shown to people, not to agents." error={err("description")}>
            {(id) => (
              <Input
                id={id}
                value={draft.description}
                onChange={(e) => set({ description: e.target.value })}
              />
            )}
          </Field>
          <IconField project={project} />
          <Field
            label="Default branch"
            hint="Sessions start from it unless told otherwise."
            error={err("default_branch")}
          >
            {(id) => (
              <Input
                id={id}
                value={draft.default_branch}
                onChange={(e) => set({ default_branch: e.target.value })}
                className="font-mono"
                autoCapitalize="off"
                autoCorrect="off"
                spellCheck={false}
              />
            )}
          </Field>
        </div>
        <PageFoot
          project={project}
          page="general"
          dirty={dirty}
          save={save}
          hint={
            branch && branch !== project.default_branch
              ? `Sessions that start from now on start from ${branch}.`
              : "Everyone in the project sees it once it's saved."
          }
          fields={["name", "description", "default_branch"]}
          onReset={reset}
        />
      </form>
      {project.role === "owner" && <ProjectDangerZone project={project} />}
    </Section>
  );
}

// ───────────────────────────── agents ─────────────────────────────

/** What the Agents page edits. */
const agentSettings = (s: G.ProjectSettings) => ({
  /** The model as chosen in its list: Model ID before one is typed sets none yet. */
  choice: modelChoice(s.model),
  instructions: s.instructions,
  model: s.model,
  effort: s.effort,
  github_access: s.github_access,
  remote_control: s.remote_control,
  claude_ai_connectors: s.claude_ai_connectors,
  project_memory: s.project_memory,
  retrospectives: s.retrospectives,
});

/**
 * Settings → Agents: what Claude is told, the model and effort it works with, what it may do on
 * GitHub, the Claude apps, and what agents leave the agents after them.
 */
function Agents({ project }: { project: G.Project }) {
  // The model's choice is part of the draft, so that it goes back with the rest of it: after a save,
  // Cancel, or someone's change elsewhere.
  const { draft: s, set, setDraft, dirty, reset } = useDraft(agentSettings(project.settings));
  const model = s.choice;
  /** The model ID typed last, so that choosing Model ID again brings it back. */
  const typedModel = useRef<string>(undefined);
  const save = useSave(
    project,
    "Saved: sessions that start from now on take it",
    () => {
      const { choice, ...settings } = s;
      if (choice === "id" && !settings.model?.trim())
        throw new ApiFailure(400, {
          code: "validation",
          field: "model",
          message: "Type a model ID, or choose another model",
        });
      return { settings: { ...project.settings, ...settings } };
    },
    (p) => setDraft(agentSettings(p.settings)),
  );
  const err = (f: string) => (failedField(save.error) === f ? save.error?.message : undefined);
  return (
    <Section title="Agents" description="What Claude is told and how it works in this project.">
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty) save.mutate();
        }}
      >
        <Field
          label="Instructions"
          hint="Appended to Claude's system prompt in every session. CLAUDE.md in the repository is read too."
          error={err("settings.instructions")}
        >
          {(id) => (
            <Textarea
              id={id}
              value={s.instructions ?? ""}
              onChange={(e) => set({ instructions: e.target.value })}
              minRows={4}
              placeholder="Run `npm test` before you commit. Keep components small."
            />
          )}
        </Field>
        <Parts>
          <Section
            title="Model"
            description="What sessions run with, unless whoever starts one chooses another."
          >
            <div className="flex flex-col gap-4">
              <div className="grid grid-cols-2 items-start gap-4">
                <Field label="Model">
                  {(id) => (
                    <Select
                      id={id}
                      value={model}
                      onChange={(v) => {
                        if (model === "id") typedModel.current = s.model;
                        set({ choice: v, model: chosenModel(v, typedModel.current) });
                      }}
                      options={MODELS}
                      className="w-full"
                    />
                  )}
                </Field>
                <Field label="Effort" error={err("effort")}>
                  {(id) => (
                    <Select
                      id={id}
                      value={s.effort ?? ""}
                      onChange={(v) => set({ effort: v || undefined })}
                      options={EFFORT_LEVELS}
                      className="w-full"
                    />
                  )}
                </Field>
              </div>
              {model === "id" && (
                <Field label="Model ID" hint="Passed to Claude Code as --model." error={err("model")}>
                  {(id) => (
                    <Input
                      id={id}
                      value={s.model ?? ""}
                      onChange={(e) => set({ model: e.target.value || undefined })}
                      className="font-mono"
                      placeholder="claude-opus-5-5"
                      autoComplete="off"
                      autoCapitalize="off"
                      autoCorrect="off"
                      spellCheck={false}
                    />
                  )}
                </Field>
              )}
            </div>
          </Section>
          <Section
            title="Access"
            description="What agents may do on GitHub and use of a Claude account, and where people follow them."
          >
            <Field
              label="Agents on GitHub"
              hint="The most its sessions' agents may do there, and what they do unless a session asks for less. Running sessions get no more from their next GitHub token on, within the hour."
            >
              {(id) => (
                <Select
                  id={id}
                  value={s.github_access ?? "push"}
                  onChange={(v) => set({ github_access: v })}
                  options={GITHUB_ACCESS.map((l) => ({
                    value: l.value,
                    label: l.label,
                    icon: <GithubAccessIcon access={l.value} />,
                    description: l.description,
                  }))}
                  className="w-full"
                />
              )}
            </Field>
            <SwitchRows className="mt-6">
              <SwitchRow
                label="claude.ai connectors"
                hint="Agents can use the connectors of the person who starts a session (mail, files, docs) without asking. Only with a full Claude login."
                checked={s.claude_ai_connectors}
                onChange={(v) => set({ claude_ai_connectors: v })}
              />
              <SwitchRow
                label="Remote Control"
                hint="Follow sessions in claude.ai/code and the Claude apps (needs a full Claude login)."
                checked={s.remote_control}
                onChange={(v) => set({ remote_control: v })}
              />
            </SwitchRows>
          </Section>
          <Section title="Memory" description="What agents leave for the agents and people after them.">
            <SwitchRows>
              <SwitchRow
                label="Project memory"
                hint="Each session's agent starts with the notes earlier agents and people left, adds what a future agent should know, and marks notes that turned out wrong stale. People read and change them under Memory. Off, agents neither read nor add notes, and the ones there stay."
                checked={s.project_memory}
                onChange={(v) => set({ project_memory: v })}
              />
              <SwitchRow
                label="Retrospectives"
                hint="As they finish a piece of work, agents record what they worked on, the approach they took, what got in the way and what a future agent should know. People read them on the session's page and under Memory."
                checked={s.retrospectives}
                onChange={(v) => set({ retrospectives: v })}
              />
            </SwitchRows>
          </Section>
        </Parts>
        <PageFoot
          project={project}
          page="agents"
          dirty={dirty}
          save={save}
          hint="Sessions that start from now on take it."
          fields={["settings.instructions", "effort", "model"]}
          onReset={reset}
        />
      </form>
    </Section>
  );
}

// ───────────────────────────── pull requests ─────────────────────────────

/** What the Pull requests page edits. */
const pullRequestSettings = (s: G.ProjectSettings) => ({
  auto_pull_request: s.auto_pull_request,
  pull_request_descriptions: s.pull_request_descriptions,
  pull_request_instructions: s.pull_request_instructions,
  credit_people: s.credit_people,
  credit_claude: s.credit_claude,
  publish_screenshots: s.publish_screenshots,
  screenshot_days: s.screenshot_days,
  follow_ups: s.follow_ups ?? NO_FOLLOW_UPS,
});

/**
 * Settings → Pull requests: whether agents open one when they're done and what they write in it,
 * who their commits credit, the screenshots they publish for it, and what brings them back to it.
 */
function PullRequests({ project }: { project: G.Project }) {
  const { data: instance } = useInstance();
  const { data: me } = useMe();
  const { draft: s, set, setDraft, dirty, reset } = useDraft(pullRequestSettings(project.settings));
  const save = useSave(
    project,
    "Saved",
    () => ({ settings: { ...project.settings, ...s } }),
    (p) => setDraft(pullRequestSettings(p.settings)),
  );
  const err = (f: string) => (failedField(save.error) === f ? save.error?.message : undefined);
  const followUps = s.follow_ups;
  const follow = (f: Partial<G.FollowUps>) => set({ follow_ups: { ...followUps, ...f } });
  return (
    <Section
      title="Pull requests"
      description="What agents put in the pull requests they open, and what brings an agent back to one."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty) save.mutate();
        }}
      >
        <div className="flex flex-col gap-5">
          <SwitchRows>
            <SwitchRow
              label="Open a pull request when done"
              hint="Agents are asked to open one with gh when the task is complete."
              checked={s.auto_pull_request}
              onChange={(v) => set({ auto_pull_request: v })}
            />
          </SwitchRows>
          <Field
            label="Descriptions"
            hint="How much agents write when they open a pull request. They keep the description up to date as they commit more."
          >
            {(id) => (
              <Select
                id={id}
                value={s.pull_request_descriptions ?? "balanced"}
                onChange={(v) => set({ pull_request_descriptions: v })}
                options={DESCRIPTIONS}
                className="w-full"
              />
            )}
          </Field>
          <Field
            label="Rules for pull requests and issues"
            hint="What agents that open pull requests are told about handling them and the issues they work on: fctl's rules until you change them, or none when you empty them."
            error={err("settings.pull_request_instructions")}
            action={
              !areDefaults(s.pull_request_instructions) && (
                <button
                  type="button"
                  className="text-[12px] text-ink-3 hover:text-ink"
                  onClick={() => set({ pull_request_instructions: null })}
                >
                  Use fctl's rules
                </button>
              )
            }
          >
            {(id) => (
              <Textarea
                id={id}
                value={shownInstructions(s.pull_request_instructions)}
                onChange={(e) => set({ pull_request_instructions: e.target.value })}
                minRows={4}
                maxRows={20}
                placeholder="Nothing: agents follow the repository's own instructions."
              />
            )}
          </Field>
        </div>
        <Parts>
          <Section title="Commits" description="Who the commits agents make credit as co-authors.">
            <SwitchRows>
              <SwitchRow
                label="Credit people in commits"
                hint="Commits name the person who started the session as a co-author, with the commit email in their profile."
                checked={s.credit_people}
                onChange={(v) => set({ credit_people: v })}
              />
              <SwitchRow
                label="Credit Claude in commits"
                hint="Commits name Claude as a co-author, Claude Code's default. Off: they leave out its trailer and the claude.ai session link."
                checked={s.credit_claude}
                onChange={(v) => set({ credit_claude: v })}
              />
            </SwitchRows>
          </Section>
          <Section title="Screenshots">
            <SwitchRows>
              <SwitchRow
                label="Publish screenshots"
                hint="Agents can publish screenshots at links anyone who has one can open, so that pull requests show them: GitHub takes no uploads from a session. Off takes them offline."
                checked={s.publish_screenshots}
                onChange={(v) => set({ publish_screenshots: v })}
                more={
                  instance &&
                  !instance.features.publish_screenshots && (
                    <p className="mt-1 text-[12.5px] text-amber">
                      This instance's admins haven't allowed it yet
                      {me && me.user.role !== "member" ? (
                        <>
                          {" "}
                          (
                          <Link
                            to="/admin"
                            search={{ tab: "settings" }}
                            className="underline underline-offset-2"
                          >
                            Admin → Settings
                          </Link>
                          )
                        </>
                      ) : null}
                      .
                    </p>
                  )
                }
              />
            </SwitchRows>
            {s.publish_screenshots && (
              <Field
                label="Screenshot links work for"
                hint="Then they stop working, and the screenshots are deleted."
                error={err("settings.screenshot_days")}
                className="mt-5"
              >
                {(id) => (
                  <Select
                    id={id}
                    value={String(s.screenshot_days ?? DEFAULT_DAYS)}
                    onChange={(v) => set({ screenshot_days: Number(v) })}
                    options={dayOptions(s.screenshot_days)}
                    className="w-full"
                  />
                )}
              </Field>
            )}
          </Section>
          <Section
            title="Follow-ups"
            description="What brings an agent back to a pull request it opened. It carries on in its session, on the Claude subscription the session ran on."
          >
            {instance && !instance.features.github_webhook && (
              <p className="mb-4 text-[12.5px] leading-relaxed text-amber">
                {instance.features.github
                  ? "This instance takes no GitHub webhooks yet, so nothing brings agents back. Its admins set GITHUB_WEBHOOK_SECRET."
                  : "This instance has no GitHub App, so its agents open no pull requests to come back to."}
              </p>
            )}
            <SwitchRows>
              <SwitchRow
                label="Comments that mention the agent"
                hint={
                  <>
                    People who can write to the repository write <Address>@fctl</Address> or{" "}
                    <Address>@agent</Address> in a comment, a review or a comment on a line, or start a
                    comment with <Address>fctl:</Address> or <Address>agent:</Address>. Apps and everyone else
                    are ignored.
                  </>
                }
                checked={followUps.comments}
                onChange={(v) => follow({ comments: v })}
              />
              <SwitchRow
                label="Failed checks"
                hint="When a check of the pull request's newest commit fails, the agent fixes what its changes broke and pushes: up to three times in a row while nobody messages it."
                checked={followUps.checks}
                onChange={(v) => follow({ checks: v })}
              />
              <SwitchRow
                label="Merge conflicts"
                hint="When the pull request conflicts with its base branch, the agent merges the base branch in, resolves the conflicts and pushes: up to three times in a row while nobody messages it."
                checked={followUps.conflicts}
                onChange={(v) => follow({ conflicts: v })}
              />
              <SwitchRow
                label="Merged or closed"
                hint="The agent does what's left, such as noting the merge in the issue it fixed."
                checked={followUps.closed}
                onChange={(v) => follow({ closed: v })}
              />
            </SwitchRows>
          </Section>
        </Parts>
        <PageFoot
          project={project}
          page="pull-requests"
          dirty={dirty}
          save={save}
          hint={
            sameFollowUps(followUps, project.settings.follow_ups ?? NO_FOLLOW_UPS)
              ? "Sessions that start from now on take it."
              : "Follow-ups change at once, for pull requests agents opened already too."
          }
          fields={["settings.pull_request_instructions", "settings.screenshot_days"]}
          onReset={reset}
        />
      </form>
    </Section>
  );
}

const sameFollowUps = (a: G.FollowUps, b: G.FollowUps) =>
  a.comments === b.comments && a.checks === b.checks && a.conflicts === b.conflicts && a.closed === b.closed;

// ───────────────────────────── sandbox ─────────────────────────────

/** `KEY=value` per line, as the Environment field shows variables. */
const envText = (env: Record<string, string>) =>
  Object.entries(env)
    .map(([k, v]) => `${k}=${v}`)
    .join("\n");

/** The variables the Environment field's lines set: blank lines and `#` comments set none. */
function parseEnv(text: string): Record<string, string> {
  const env: Record<string, string> = {};
  for (const line of text.split("\n")) {
    const t = line.trim();
    if (!t || t.startsWith("#")) continue;
    const i = t.indexOf("=");
    if (i > 0) env[t.slice(0, i).trim()] = t.slice(i + 1);
  }
  return env;
}

/** What the Sandbox page edits; the environment as its field's text. */
const sandboxSettings = (s: G.ProjectSettings) => ({
  /** Where sessions start from, as chosen: an image from a registry, before one is typed, sets none yet. */
  source: imageSource(s),
  setup_script: s.setup_script,
  package_caches: s.package_caches,
  image: s.image,
  image_build: s.image_build ?? null,
  cpus: s.cpus,
  memory_mb: s.memory_mb,
  idle_timeout_minutes: s.idle_timeout_minutes,
  env: envText(s.env),
});

/**
 * Settings → Sandbox: what each session's sandbox runs after cloning, the image it starts from, its
 * size, how long it waits for a message, and its environment.
 */
function Sandbox({ project }: { project: G.Project }) {
  const saved = sandboxSettings(project.settings);
  const { draft: s, set, setDraft, dirty: changed, reset } = useDraft(saved);
  // Lines that set the same variables are no change: a blank line, a comment.
  const dirty =
    changed && (sameEnv(s.env, saved.env) ? !sameDraft({ ...s, env: "" }, { ...saved, env: "" }) : true);
  // The image's source is part of the draft, so that it goes back with the rest of it: after a save,
  // Cancel, or someone's change elsewhere.
  const source = s.source;
  /** What was typed for each image source, so that switching back brings it back. */
  const typed = useRef<Typed>({});
  const save = useSave(
    project,
    "Saved: sessions that start from now on take it",
    () => {
      const { source: from, env, ...settings } = s;
      if (from === "image" && !settings.image?.trim())
        throw new ApiFailure(400, {
          code: "validation",
          field: "image",
          message: "Name the image, or choose where else sessions start from",
        });
      return { settings: { ...project.settings, ...settings, env: parseEnv(env) } };
    },
    (p) => setDraft(sandboxSettings(p.settings)),
  );
  const err = (f: string) => (failedField(save.error) === f ? save.error?.message : undefined);
  const number = (v: string) => (v ? Number(v) : undefined);
  return (
    <Section
      title="Sandbox"
      description="What each session's sandbox starts from and runs before the agent works, how large it is, and how long it waits."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty) save.mutate();
        }}
      >
        <div className="flex flex-col gap-5">
          <Field
            label="Setup script"
            hint="Runs in the repository after cloning."
            error={err("settings.setup_script")}
          >
            {(id) => (
              <Textarea
                id={id}
                value={s.setup_script ?? ""}
                onChange={(e) => set({ setup_script: e.target.value })}
                minRows={3}
                className="font-mono text-[12.5px]"
                placeholder={"npm ci\nnpx playwright install chromium"}
              />
            )}
          </Field>
          <SwitchRows>
            <SwitchRow
              label="Package caches"
              hint="Sessions on the same runner share what npm, pnpm, Yarn, Bun, Cargo, uv, pip, Go and Playwright download, so setup scripts fetch only what's new. Each runner keeps up to 10 GiB for the project, or what its owner sets, and empties them past that. Reviews and Lima runners don't use them."
              checked={s.package_caches}
              onChange={(v) => set({ package_caches: v })}
            />
          </SwitchRows>
        </div>
        <Parts>
          <Section
            title="Image"
            description="Apple container and Docker runners make sandboxes from it. Lima runners clone the VM their host prepared, and the process driver uses none."
          >
            <div className="flex flex-col gap-4">
              <Field label="Sessions start from">
                {(id) => (
                  <Select
                    id={id}
                    value={source}
                    onChange={(v) => {
                      const now = { ...project.settings, ...s, env: {} };
                      typed.current = remember(now, typed.current);
                      const next = withSource(now, v, typed.current);
                      set({ source: v, image: next.image, image_build: next.image_build ?? null });
                    }}
                    options={imageSources(project)}
                    className="w-full"
                  />
                )}
              </Field>
              {source === "image" && (
                <Field
                  label="Image reference"
                  hint="It needs git and Claude Code 2.1.101 or later. Runners run it when their owners allow it (allowed_images in host.toml); a session no runner allows stays queued and says so."
                  error={err("image")}
                >
                  {(id) => (
                    <Input
                      id={id}
                      value={s.image ?? ""}
                      onChange={(e) => set({ image: e.target.value || undefined })}
                      className="font-mono"
                      placeholder="ghcr.io/acme/agent:1"
                      autoCapitalize="off"
                      autoCorrect="off"
                      spellCheck={false}
                    />
                  )}
                </Field>
              )}
              {source === "build" && s.image_build && (
                <BuildFields
                  project={project}
                  build={s.image_build}
                  onChange={(b) => set({ image_build: b })}
                  error={err("settings.image_build")}
                />
              )}
            </div>
          </Section>
          <Section title="Size" description="Empty takes the runner's default.">
            <div className="flex flex-col gap-3">
              <div className="grid grid-cols-2 items-start gap-4">
                <Field label="CPUs" error={err("settings.cpus")}>
                  {(id) => (
                    <Input
                      id={id}
                      type="number"
                      inputMode="numeric"
                      min={1}
                      max={64}
                      value={s.cpus ?? ""}
                      onChange={(e) => set({ cpus: number(e.target.value) })}
                      placeholder="The runner's"
                    />
                  )}
                </Field>
                <Field label="Memory (MiB)" error={err("settings.memory_mb")}>
                  {(id) => (
                    <Input
                      id={id}
                      type="number"
                      inputMode="numeric"
                      min={512}
                      step={512}
                      value={s.memory_mb ?? ""}
                      onChange={(e) => set({ memory_mb: number(e.target.value) })}
                      placeholder="The runner's"
                    />
                  )}
                </Field>
              </div>
              <LargestSandbox />
            </div>
          </Section>
          <Section
            title="Idle sessions"
            description="A session that waits for a message longer than this stops, and its runner takes other work. A message resumes it. Work its agent left running in the background keeps it up for up to an hour after the turn, or for this long when that is longer."
          >
            <Field
              label="Idle timeout (minutes)"
              error={err("settings.idle_timeout_minutes")}
              className="max-w-[16rem]"
            >
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  inputMode="numeric"
                  min={1}
                  value={s.idle_timeout_minutes ?? ""}
                  onChange={(e) => set({ idle_timeout_minutes: number(e.target.value) })}
                  placeholder="10"
                />
              )}
            </Field>
          </Section>
          <Section
            title="Environment"
            description={
              <>
                Variables every sandbox has. They aren't secret: credentials go in{" "}
                <Link
                  to="/p/$slug"
                  params={{ slug: project.slug }}
                  search={{ tab: "secrets" }}
                  className="text-ink-2 underline underline-offset-2 hover:text-ink"
                >
                  Secrets
                </Link>
                .
              </>
            }
          >
            <Field label="Variables" hint="KEY=value, one per line." error={err("settings.env")}>
              {(id) => (
                <Textarea
                  id={id}
                  value={s.env}
                  onChange={(e) => set({ env: e.target.value })}
                  minRows={2}
                  className="font-mono text-[12.5px]"
                  placeholder="NODE_ENV=development"
                  autoCapitalize="off"
                  autoCorrect="off"
                  spellCheck={false}
                />
              )}
            </Field>
          </Section>
        </Parts>
        <PageFoot
          project={project}
          page="sandbox"
          dirty={dirty}
          save={save}
          hint="Sessions that start from now on take it."
          fields={[
            "settings.setup_script",
            "image",
            "settings.image_build",
            "settings.cpus",
            "settings.memory_mb",
            "settings.idle_timeout_minutes",
            "settings.env",
          ]}
          onReset={reset}
        />
      </form>
    </Section>
  );
}

/** Whether two texts of the Environment field set the same variables. */
const sameEnv = (a: string, b: string) => sameDraft(parseEnv(a), parseEnv(b));

// ───────────────────────────── the pages' parts ─────────────────────────────

/** The models a project can choose; the empty value leaves it to Claude. */
const MODELS: SelectOption<ModelChoice>[] = [
  { value: "", label: "Claude's default" },
  ...ALIASES,
  { value: "id", label: "Model ID" },
];

/** Claude's effort levels; the empty value leaves it to Claude. */
const EFFORT_LEVELS: SelectOption[] = [
  { value: "", label: "Claude's default", icon: <EffortIcon level="" /> },
  ...EFFORTS.map((e) => ({ ...e, icon: <EffortIcon level={e.value} /> })),
];

/** How much agents write in the descriptions of their pull requests; balanced unless chosen. */
const DESCRIPTIONS: SelectOption<G.Verbosity>[] = [
  {
    value: "brief",
    label: "Brief",
    description: "A few sentences: what changed and why, and how it was verified.",
  },
  {
    value: "balanced",
    label: "Balanced",
    description:
      "A short summary of what changed and why, and the decisions and assumptions worth confirming.",
  },
  {
    value: "full",
    label: "Full",
    description:
      "Everything: how it works, the choices made and the alternatives, how it was verified in detail.",
  },
];

/** What brings agents back to their pull requests where a project says nothing of it. */
const NO_FOLLOW_UPS: G.FollowUps = { comments: false, checks: false, closed: false, conflicts: false };

/** A word that addresses agents on GitHub, as people write it. */
function Address({ children }: { children: string }) {
  return <code className="font-mono text-[11.5px] text-ink-2">{children}</code>;
}

/** How long screenshot links may work, with a project's own choice among them. */
function dayOptions(current?: number): SelectOption[] {
  const days = [...new Set([...DAY_CHOICES, current ?? DEFAULT_DAYS])].sort((a, b) => a - b);
  return days.map((d) => ({ value: String(d), label: lasts(d) }));
}

/** The most CPUs and memory any enabled runner gives a sandbox, when runners say. */
function LargestSandbox() {
  const runners = useRunners();
  const limits = (runners.data?.runners ?? [])
    .filter((r) => r.status !== "disabled")
    .flatMap((r) => (r.sandbox ? [r.sandbox.max] : []));
  if (limits.length === 0) return null;
  const most = {
    cpus: Math.max(...limits.map((l) => l.cpus)),
    memory_mb: Math.max(...limits.map((l) => l.memory_mb)),
  };
  return (
    <p className="text-[12.5px] leading-relaxed text-ink-3">
      Runners give a sandbox at most {cpus(most.cpus)} and {memory(most.memory_mb)}; a session that asks for
      more stays queued.
    </p>
  );
}

/** Where a project's sessions can start from: a build only with a repository. */
function imageSources(project: G.Project): SelectOption<ImageSource>[] {
  return [
    {
      value: "default",
      label: "Each runner's own",
      description: "ghcr.io/hyperstructure/sandbox, unless the runner's owner chose another",
    },
    { value: "image", label: "An image from a registry", description: "On runners that allow it" },
    {
      value: "build",
      label: "Built from the repository",
      description: project.repo
        ? "A Dockerfile or devcontainer.json on its default branch"
        : "Link a repository first",
      disabled: !project.repo,
    },
  ];
}

/** The file a project's image builds from, and the directory a Dockerfile builds in. */
function BuildFields({
  project,
  build,
  onChange,
  error,
}: {
  project: G.Project;
  build: G.ImageBuild;
  onChange: (b: G.ImageBuild) => void;
  error?: string;
}) {
  const repo = project.repo ? `${project.repo.owner}/${project.repo.name}` : "the repository";
  const devcontainer = isDevcontainer(build.file);
  return (
    <div className="flex flex-col gap-1.5">
      <div className="grid gap-4 sm:grid-cols-2">
        <Field label="File" error={error}>
          {(id) => (
            <Input
              id={id}
              value={build.file}
              onChange={(e) =>
                onChange({
                  file: e.target.value,
                  context: isDevcontainer(e.target.value) ? undefined : build.context,
                })
              }
              className="font-mono"
              placeholder="Dockerfile"
            />
          )}
        </Field>
        {!devcontainer && (
          <Field label="Directory" hint="Where it builds. Empty: the repository's root.">
            {(id) => (
              <Input
                id={id}
                value={build.context ?? ""}
                onChange={(e) => onChange({ ...build, context: e.target.value || undefined })}
                className="font-mono"
                placeholder="."
              />
            )}
          </Field>
        )}
      </div>
      <p className="text-[12.5px] text-ink-3">
        Runners whose owners allow builds from {repo} (allowed_builds in host.toml) build it from{" "}
        {project.default_branch}, keep it, and build it again when the files it comes from change.
        {devcontainer ? " The devcontainer.json names its own Dockerfile and directory." : ""}
      </p>
    </div>
  );
}

/** The project's icon in its settings: saved as it's chosen, apart from the form. */
function IconField({ project }: { project: G.Project }) {
  const [picking, setPicking] = useState(false);
  const icon = iconOf(project);
  return (
    <div>
      <span className="mb-1.5 block text-[12.5px] font-medium text-ink-2">Icon</span>
      <div className="flex items-center gap-3">
        <ProjectIcon icon={icon} tile={38} />
        <span className="min-w-0 flex-1 text-[13px] text-ink-3">
          {icon.chosen ? (
            <>
              <span className="text-ink-2">{iconDef(icon.name)?.label ?? icon.name}</span> ·{" "}
              {icon.hex ? (
                <span className="font-mono text-[12px]">{showHex(icon.hex)}</span>
              ) : (
                ICON_COLORS[icon.color].label.toLowerCase()
              )}
              , {icon.style}
            </>
          ) : (
            "The one that fits its name, until someone chooses one"
          )}
        </span>
        <Button type="button" size="field" onClick={() => setPicking(true)}>
          {icon.chosen ? "Change" : "Choose"}
        </Button>
      </div>
      <ProjectIconPicker project={project} open={picking} onClose={() => setPicking(false)} />
    </div>
  );
}

function ProjectDangerZone({ project }: { project: G.Project }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="mt-10">
      <DangerZone
        title="Delete this project"
        description={`Deletes the project with its sessions, their transcripts, its secrets and its members' access.${project.repo ? " The repository on GitHub stays." : ""} This cannot be undone.`}
        action="Delete project…"
        onAction={() => setOpen(true)}
      >
        <DeleteProject project={project} open={open} onClose={() => setOpen(false)} />
      </DangerZone>
    </div>
  );
}

function DeleteProject({
  project,
  open,
  onClose,
}: {
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const navigate = useNavigate();
  const [typed, setTyped] = useState("");
  const remove = useMutation({
    mutationFn: () => api.deleteProject(project.id),
    onSuccess: () => {
      queryClient.removeQueries({ queryKey: keys.project(project.slug) });
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      void queryClient.invalidateQueries({ queryKey: ["sessions"] });
      toast(`Deleted ${project.name}`);
      void navigate({ to: "/" });
    },
  });
  useEffect(() => {
    if (!open) {
      setTyped("");
      remove.reset();
    }
  }, [open, remove.reset]);
  const running = project.active_sessions;
  const sessions = project.total_sessions;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Delete ${project.name}?`}
      description="This deletes the project for good. It cannot be undone."
    >
      {running > 0 ? (
        <div className="flex flex-col gap-5">
          <Banner tone="warn">
            Stop the project's {running === 1 ? "running session" : `${running} running sessions`} first.
          </Banner>
          <div className="flex justify-end">
            <Button onClick={onClose}>Close</Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-5"
          onSubmit={(e) => {
            e.preventDefault();
            if (typed.trim() === project.slug) remove.mutate();
          }}
        >
          <ul className="flex list-disc flex-col gap-1.5 pl-5 text-[13.5px] leading-relaxed text-ink-2 marker:text-ink-4">
            <li>
              {sessions === 0
                ? "It has no sessions."
                : sessions === 1
                  ? "Its session and transcript are deleted."
                  : `Its ${sessions} sessions and their transcripts are deleted.`}
            </li>
            <li>Its secrets and settings are deleted, and its members lose access.</li>
            <li>
              {project.repo ? (
                <>
                  The repository{" "}
                  <span className="font-mono text-ink">
                    {project.repo.owner}/{project.repo.name}
                  </span>{" "}
                  on GitHub is not touched.
                </>
              ) : (
                "Nothing outside fctl is touched."
              )}
            </li>
          </ul>
          <Field
            label={
              <>
                Type <span className="font-mono text-ink">{project.slug}</span> to confirm
              </>
            }
            error={remove.error?.message}
          >
            {(id) => (
              <Input
                id={id}
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
              />
            )}
          </Field>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="danger"
              loading={remove.isPending}
              disabled={typed.trim() !== project.slug}
            >
              Delete project
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

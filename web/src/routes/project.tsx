// A project: its sessions, repositories, settings, people and secrets.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { ArrowRight, Building2, GitBranch, KeyRound, Plus, Trash2 } from "lucide-react";
import { type ReactNode, useEffect, useId, useRef, useState } from "react";
import { GithubAccessIcon } from "@/components/github-access";
import { MemoryTab } from "@/components/project/memory";
import { RetrospectivesTab } from "@/components/project/retrospectives";
import { ReviewsTab } from "@/components/project/reviews";
import { ScheduleButton, SchedulesTab } from "@/components/project/schedules";
import { TicketsTab } from "@/components/project/tickets";
import {
  AttachButton,
  DraftTray,
  DropHint,
  useAttachments,
  useFileDrop,
} from "@/components/session/attachments";
import { PullRequestLinks, ScheduledMark, SessionLink, TicketMark } from "@/components/session/row";
import {
  StartButton,
  StartChoices,
  useKeepDraft,
  useStartChoice,
  useStartSession,
} from "@/components/session/start";
import {
  Avatar,
  Banner,
  Button,
  DangerZone,
  Dialog,
  EffortIcon,
  Empty,
  Field,
  GithubIcon,
  Input,
  SecretInput,
  Section,
  Select,
  type SelectOption,
  Skeleton,
  StatusDot,
  Switch,
  Textarea,
  TextareaControls,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { NoRunnerNote } from "@/components/workspace/no-runners";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { filesFromPaste } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { draftId, forgetDraft, loadText } from "@/lib/draft";
import { ago, agoShort, cpus, memory, stamp } from "@/lib/format";
import { GITHUB_ACCESS } from "@/lib/github-access";
import {
  type ImageSource,
  imageSource,
  isDevcontainer,
  remember,
  type Typed,
  withSource,
} from "@/lib/images";
import { ALIASES, asked, chosenModel, EFFORTS, type ModelChoice, modelChoice } from "@/lib/models";
import { areDefaults, shownInstructions } from "@/lib/pr-instructions";
import { DAY_CHOICES, DEFAULT_DAYS, lasts } from "@/lib/published";
import {
  keys,
  queryClient,
  useInstance,
  useMe,
  useProject,
  useProjectRepos,
  useRunners,
  useSessions,
} from "@/lib/queries";
import { repoLabel, repoSummary } from "@/lib/repos";
import { lastMoved, sections } from "@/lib/sessions";
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

type Tab =
  | "sessions"
  | "tickets"
  | "schedules"
  | "reviews"
  | "memory"
  | "retrospectives"
  | "repositories"
  | "settings"
  | "members"
  | "secrets";

/** Everyone in the project sees these; owners and maintainers the rest. */
const OPEN_TABS: Tab[] = [
  "sessions",
  "tickets",
  "schedules",
  "reviews",
  "memory",
  "retrospectives",
  "repositories",
  "members",
];

export function ProjectPage() {
  const { slug } = useParams({ from: "/app/p/$slug" });
  const { tab = "sessions", schedule, linear } = useSearch({ from: "/app/p/$slug" });
  const project = useProject(slug);
  const p = project.data;
  const manage = p?.role === "owner" || p?.role === "maintainer";
  const repos = p && repoSummary(p);
  const workspace = freshest(useWorkspaceIndex(), p?.workspace);
  if (project.isError)
    return (
      <div className="mx-auto max-w-xl px-6 py-24">
        <Banner tone="error">This project does not exist, or you cannot see it.</Banner>
      </div>
    );
  return (
    <div className="mx-auto max-w-[920px] px-6 pt-12 pb-24">
      {p ? (
        <header className="animate-rise">
          {workspace && (
            <Link
              to="/w/$slug"
              params={{ slug: workspace.slug }}
              className="mb-3 inline-flex max-w-full items-center gap-2 text-[13px] text-ink-3 transition-colors hover:text-ink"
            >
              <WorkspaceAvatar workspace={workspace} size={18} />
              <span className="truncate">{workspace.name}</span>
            </Link>
          )}
          {/* A name or a link with no spaces wraps on a phone instead of widening the page. */}
          <h1 className="font-display text-[38px] leading-[1.1] font-medium tracking-[-0.022em] [overflow-wrap:anywhere]">
            {p.name}
          </h1>
          {p.description && (
            <p className="mt-2 max-w-2xl text-[15px] leading-relaxed text-ink-3 [overflow-wrap:anywhere]">
              {p.description}
            </p>
          )}
          <div className="mt-3 flex flex-wrap items-center gap-x-4 gap-y-1 text-[12.5px] text-ink-3">
            {repos ? (
              <span className="inline-flex items-center gap-1.5">
                {p.repo && (
                  <a
                    href={`https://github.com/${p.repo.owner}/${p.repo.name}`}
                    target="_blank"
                    rel="noreferrer"
                    className="inline-flex items-center gap-1.5 font-mono text-ink-2 transition-colors hover:text-ink"
                  >
                    <GithubIcon className="size-3.5" />
                    {repos.name}
                  </a>
                )}
                {repos.more && (
                  <Link
                    to="/p/$slug"
                    params={{ slug }}
                    search={{ tab: "repositories" }}
                    className="transition-colors hover:text-ink"
                  >
                    {p.repo ? repos.more : repoLabel(p)}
                  </Link>
                )}
              </span>
            ) : (
              <span>No repository</span>
            )}
            <span className="inline-flex items-center gap-1.5 font-mono">
              <GitBranch className="size-3.5 text-ink-4" />
              {p.default_branch}
            </span>
            <span>
              {p.total_sessions} {p.total_sessions === 1 ? "session" : "sessions"}
              {p.active_sessions > 0 && <span className="text-accent"> · {p.active_sessions} active</span>}
            </span>
          </div>
        </header>
      ) : (
        <div>
          <Skeleton className="h-10 w-80" />
          <Skeleton className="mt-4 h-4 w-48" />
        </div>
      )}
      {/* Scrolls sideways where the tabs don't fit (phones). */}
      <nav className="-mx-6 mt-8 overflow-x-auto px-6 [scrollbar-width:none]">
        <div className="flex w-max min-w-full gap-1 border-b border-line">
          {(
            [
              "sessions",
              "tickets",
              "schedules",
              "reviews",
              "memory",
              "retrospectives",
              "repositories",
              "settings",
              "members",
              "secrets",
            ] as Tab[]
          )
            .filter((t) => OPEN_TABS.includes(t) || manage)
            .map((t) => (
              <Link
                key={t}
                to="/p/$slug"
                params={{ slug }}
                search={{ tab: t === "sessions" ? undefined : t }}
                className={cn(
                  "relative shrink-0 px-3 py-2.5 text-[13.5px] capitalize transition-colors",
                  tab === t ? "text-ink" : "text-ink-3 hover:text-ink",
                )}
              >
                {t}
                {tab === t && (
                  <span className="absolute inset-x-2 -bottom-px h-[2px] rounded-full bg-accent" />
                )}
              </Link>
            ))}
        </div>
      </nav>
      <div className="mt-8">
        {p && tab === "sessions" && <Sessions project={p} />}
        {p && tab === "tickets" && <TicketsTab key={p.id} project={p} returned={linear} />}
        {p && tab === "schedules" && <SchedulesTab project={p} open={schedule} />}
        {p && tab === "reviews" && <ReviewsTab project={p} />}
        {p && tab === "memory" && <MemoryTab project={p} />}
        {p && tab === "retrospectives" && <RetrospectivesTab project={p} />}
        {p && tab === "repositories" && <Repositories project={p} />}
        {p && tab === "settings" && manage && <Settings project={p} />}
        {p && tab === "settings" && p.role === "owner" && <ProjectDangerZone project={p} />}
        {p && tab === "members" && <Members project={p} />}
        {p && tab === "secrets" && manage && <Secrets project={p} />}
      </div>
    </div>
  );
}

function Sessions({ project }: { project: G.Project }) {
  const [filter, setFilter] = useState<"active" | "all" | "mine">("all");
  const mine = filter === "mine" || undefined;
  // Every session at work, however long ago it started. By the project's id, as the sidebar asks for
  // them under it: without `mine`, one request serves both.
  const active = useSessions({ project: project.id, status: "active", limit: 50, mine });
  const history = filter !== "active";
  const ended = useSessions({ project: project.id, status: "ended", limit: 100, mine }, { enabled: history });
  const { working, recent, older } = sections(
    history ? [active.data?.sessions, ended.data?.sessions] : [active.data?.sessions],
    Date.now(),
  );
  const canStart = project.role !== "viewer";
  const { data: me } = useMe();
  return (
    <div className="flex flex-col gap-10">
      {/* Another project's page opens its own draft. */}
      {canStart && me && <StartForm key={project.id} project={project} user={me.user.id} />}
      <div>
        <div className="mb-8 flex items-center gap-1">
          {(["all", "active", "mine"] as const).map((f) => (
            <button
              key={f}
              type="button"
              onClick={() => setFilter(f)}
              className={cn(
                "rounded-full px-3 py-1 text-[12.5px] capitalize transition-colors",
                filter === f ? "bg-raised text-ink" : "text-ink-3 hover:text-ink",
              )}
            >
              {f}
            </button>
          ))}
        </div>
        {active.isPending || (history && ended.isPending) ? (
          <div className="space-y-3 py-3">
            <Skeleton className="h-11" />
            <Skeleton className="h-11" />
          </div>
        ) : history && !working.length && !recent.length && !older.length ? (
          <Empty title="No sessions yet">
            Describe a task above and an agent starts on it in a fresh sandbox.
          </Empty>
        ) : (
          // In sections, as on the Agent Floor: the sessions at work, then the ended ones that moved in
          // the last day, whose times read in minutes and hours, then the rest.
          <div className="flex flex-col gap-12">
            <SessionSection title="Working now" note={`${working.length} active`}>
              {working.length ? (
                <SessionList sessions={working} />
              ) : (
                <p className="border-y border-line py-6 text-[13.5px] text-ink-4">
                  {mine ? "No agents are working for you." : "No agents are working."}
                  {canStart && " Give one a task above."}
                </p>
              )}
            </SessionSection>
            {history && recent.length > 0 && (
              <SessionSection title="Recently">
                <SessionList sessions={recent} />
              </SessionSection>
            )}
            {history && older.length > 0 && (
              <SessionSection title="Older">
                <SessionList sessions={older} />
              </SessionSection>
            )}
          </div>
        )}
      </div>
    </div>
  );
}

/** A section of a project's sessions, as the Agent Floor has them. */
function SessionSection({ title, note, children }: { title: string; note?: string; children: ReactNode }) {
  return (
    <section>
      <div className="mb-2 flex items-baseline justify-between">
        <h2 className="text-[13px] font-semibold text-ink">{title}</h2>
        {note && <span className="text-[12px] text-ink-4">{note}</span>}
      </div>
      {children}
    </section>
  );
}

/**
 * Sessions in their project's list: each one's number, branch and pull requests, who started it and
 * when it last moved.
 */
function SessionList({ sessions }: { sessions: G.Session[] }) {
  return (
    <div className="border-t border-line">
      {sessions.map((s) => (
        <div
          key={s.id}
          className="group relative isolate flex items-center gap-3.5 border-b border-line p-3 transition-colors last:border-b-0 hover:bg-hover/30"
        >
          <span className="w-9 shrink-0 text-right font-mono text-[12px] text-ink-4">#{s.number}</span>
          <StatusDot status={s.status} />
          <div className="min-w-0 flex-1">
            <SessionLink s={s} />
            <div className="mt-0.5 flex gap-2 text-[12px] text-ink-4">
              <ScheduledMark s={s} />
              <TicketMark s={s} />
              {s.branch && <span className="truncate font-mono">{s.branch}</span>}
              <PullRequestLinks s={s} />
            </div>
          </div>
          <Avatar user={s.created_by} size={20} />
          <time
            dateTime={new Date(lastMoved(s)).toISOString()}
            title={stamp(lastMoved(s))}
            className="w-10 shrink-0 text-right text-[12px] whitespace-nowrap text-ink-4 tabular-nums"
          >
            {agoShort(lastMoved(s))}
          </time>
          <ArrowRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-all group-hover:opacity-100 max-sm:hidden" />
        </div>
      ))}
    </div>
  );
}

/**
 * The Sessions tab's task box. What is written in it stays in this browser until a session starts
 * with it, apart from the home page's draft and other projects'.
 */
function StartForm({ project, user }: { project: G.Project; user: string }) {
  const keep = draftId(user, project.id);
  const [kept] = useState(() => loadText(keep));
  const [prompt, setPrompt] = useState(kept?.prompt ?? "");
  const { choice, picked, pick } = useStartChoice(project, kept);
  const ref = useRef<HTMLTextAreaElement>(null);
  const files = useAttachments({ project: project.slug }, { keep });
  const draft = useKeepDraft(keep, {
    kept,
    textarea: ref,
    text: { prompt, ...picked },
    files: files.drafts.length,
  });
  const { start, starting } = useStartSession(draft.forget);
  const go = () => start(project.slug, prompt, files, asked(project, choice));
  const attach = (f: File[], pasted?: boolean) => {
    files.add(f, { pasted });
    ref.current?.focus();
  };
  const dragging = useFileDrop(attach, !starting);
  const workspace = useWorkspaceIndex().get(project.workspace?.id ?? "");
  return (
    <div>
      <form
        onSubmit={(e) => {
          e.preventDefault();
          go();
        }}
        className={cn(
          "relative rounded-2xl border border-line-strong bg-surface/80 transition-[border-color,box-shadow] focus-within:border-accent/45 focus-within:shadow-[0_0_0_4px_var(--c-accent-soft)]",
          dragging && "border-accent/50",
        )}
      >
        <DraftTray
          drafts={files.drafts}
          onRemove={files.remove}
          onRetry={files.retry}
          readOnly={starting}
          className="px-4 pt-4"
        />
        <Textarea
          ref={ref}
          bare
          minRows={2}
          maxRows={12}
          value={prompt}
          readOnly={starting}
          onChange={(e) => setPrompt(e.target.value)}
          onBlur={draft.save}
          onKeyDown={(e) => {
            if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
              e.preventDefault();
              go();
            }
          }}
          onPaste={(e) => {
            const pasted = filesFromPaste(e.clipboardData);
            if (!pasted.length || starting) return;
            e.preventDefault();
            attach(pasted, true);
          }}
          placeholder={`What should an agent do in ${project.name}?`}
          className="block px-5 pt-4 font-display text-[17px] leading-[1.5] placeholder:text-ink-4"
        />
        {/* More room above the pickers where they take a line of their own. On a phone everything
            keeps one line, down to 360 px with a model named by its ID and the GitHub picker: the
            gaps are narrower there, and the pickers go to the end without the spacer and its
            gaps. */}
        <TextareaControls
          textarea={ref}
          className="flex-wrap gap-3 pt-2 pr-3 pb-3 pl-2.5 max-sm:gap-2 sm:max-lg:pt-3"
        >
          <AttachButton onFiles={(f) => attach(f)} disabled={starting} />
          <span className="flex-1 max-sm:hidden" />
          <StartChoices
            project={project}
            choice={choice}
            onPick={pick}
            issues
            disabled={starting}
            className="max-sm:ml-auto"
          />
          {/* Set apart from the pickers, as on the home page. On a phone its room goes to the
              button. */}
          <span className="ml-4 text-[11.5px] text-ink-4 max-sm:hidden">⌘+Enter</span>
          <ScheduleButton
            project={project}
            prompt={prompt}
            choice={choice}
            files={files.drafts.length}
            disabled={starting}
            // The draft stays here until a schedule is set up with it.
            onSaved={() => void forgetDraft(keep)}
          />
          <StartButton starting={starting} disabled={!prompt.trim() && !files.drafts.length}>
            Start a session
          </StartButton>
        </TextareaControls>
        <DropHint show={dragging} />
      </form>
      <NoRunnerNote workspace={workspace} className="mt-3" />
    </div>
  );
}

const ACCESS: SelectOption<G.RepoAccess>[] = [
  { value: "read", label: "Read only", description: "Clone, fetch, and read issues and pull requests" },
  { value: "write", label: "Read and write", description: "Also push branches and open pull requests" },
];
const accessLabel = (a: G.RepoAccess) => (a === "write" ? "Read and write" : "Read only");
/** GitHub names ignore case. */
const same = (a: string, b?: string) => a.toLowerCase() === b?.toLowerCase();

function Repositories({ project }: { project: G.Project }) {
  const manage = project.role === "owner" || project.role === "maintainer";
  const { data: instance } = useInstance();
  const repos = useProjectRepos(project.id);
  const workspace = project.workspace?.id ?? "";
  // The repositories of the project's workspace.
  const github = useQuery({
    queryKey: keys.repos(workspace),
    queryFn: () => api.githubRepos(workspace),
    enabled: manage && !!workspace && !!instance?.features.github,
  });
  const [repo, setRepo] = useState("");
  const [access, setAccess] = useState<G.RepoAccess>("read");
  const suggestionsId = useId();
  const list = repos.data?.repos ?? [];
  const owner = list[0]?.owner;
  const wide = repo.trim().endsWith("/*");
  const save = (r: G.ProjectRepoList) => {
    queryClient.setQueryData(keys.projectRepos(project.id), r);
    // The project's header and the lists of projects count them.
    for (const k of [keys.project(project.slug), keys.project(project.id), keys.projects])
      void queryClient.invalidateQueries({ queryKey: k });
  };
  const add = useMutation({
    mutationFn: () => api.addProjectRepo(project.id, { repo: repo.trim(), access: wide ? "read" : access }),
    onSuccess: (r) => {
      save(r);
      setRepo("");
      setAccess("read");
    },
  });
  const change = useMutation({
    mutationFn: (b: G.AddProjectRepoRequest) => api.addProjectRepo(project.id, b),
    onSuccess: save,
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: (r: G.ProjectRepo) => api.removeProjectRepo(project.id, r.owner, r.name),
    onSuccess: save,
    onError: (e) => toast(e.message, "error"),
  });
  // The GitHub App's repositories on the project's account that are not listed yet.
  const suggestions = (github.data?.repos ?? [])
    .filter((r) => !owner || same(r.owner, owner))
    .filter((r) => !list.some((l) => same(l.owner, r.owner) && same(l.name ?? "*", r.name)))
    .map((r) => r.full_name);
  if (owner && !list.some((l) => !l.name)) suggestions.unshift(`${owner}/*`);
  return (
    <div>
      <p className="mb-6 max-w-2xl text-[13.5px] leading-relaxed text-ink-3">
        Agents start in the primary repository and clone the others when they need them, into{" "}
        <code className="font-mono text-[12.5px] text-ink-2">~/repos/&lt;owner&gt;/&lt;name&gt;</code>. They
        push and open pull requests only where they can write. All of a project's repositories belong to one
        GitHub account. Changes apply to sessions that start afterwards.
      </p>
      {manage && (
        <form
          className="mb-6"
          onSubmit={(e) => {
            e.preventDefault();
            if (repo.trim()) add.mutate();
          }}
        >
          <Field label="Add a repository" error={add.error?.message}>
            {(id) => (
              <div className="flex flex-wrap gap-2">
                <Input
                  id={id}
                  list={suggestionsId}
                  value={repo}
                  onChange={(e) => setRepo(e.target.value)}
                  placeholder={`${owner ?? "owner"}/name, or ${owner ?? "owner"}/* for all of them`}
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  className="min-w-[240px] flex-1 font-mono text-[13px]"
                />
                <datalist id={suggestionsId}>
                  {suggestions.map((n) => (
                    <option key={n} value={n} />
                  ))}
                </datalist>
                <Select
                  aria-label="Access"
                  value={wide ? "read" : access}
                  disabled={wide}
                  onChange={setAccess}
                  options={ACCESS}
                />
                <Button
                  type="submit"
                  variant="primary"
                  size="field"
                  icon={<Plus className="size-3.5" />}
                  loading={add.isPending}
                  disabled={!repo.trim()}
                >
                  Add
                </Button>
              </div>
            )}
          </Field>
        </form>
      )}
      {repos.isPending ? (
        <div className="space-y-3 py-3">
          <Skeleton className="h-11" />
          <Skeleton className="h-11" />
        </div>
      ) : list.length === 0 ? (
        <p className="border-t border-line py-6 text-[13px] text-ink-4">
          No repositories. Agents work in an empty directory.
        </p>
      ) : (
        <ul className="border-t border-line">
          {list.map((r) => {
            const label = `${r.owner}/${r.name ?? "*"}`;
            return (
              <li key={label} className="flex items-center gap-3 border-b border-line py-3">
                {r.name ? (
                  <GithubIcon className="size-4 shrink-0 text-ink-4" />
                ) : (
                  <Building2 className="size-4 shrink-0 text-ink-4" />
                )}
                <div className="min-w-0 flex-1">
                  <div className="truncate font-mono text-[13.5px] text-ink">
                    {r.name ? (
                      <a
                        href={`https://github.com/${r.owner}/${r.name}`}
                        target="_blank"
                        rel="noreferrer"
                        className="transition-colors hover:text-accent"
                      >
                        {label}
                      </a>
                    ) : (
                      label
                    )}
                  </div>
                  <div className="truncate text-[12px] text-ink-4">
                    {r.primary
                      ? "Primary: agents start in it"
                      : !r.name
                        ? `Every repository of ${r.owner} the GitHub App can reach`
                        : r.added_by && r.added_at
                          ? `Added by ${r.added_by.name} ${ago(r.added_at)}`
                          : null}
                  </div>
                </div>
                {manage && !r.primary && r.name ? (
                  <Select
                    size="sm"
                    aria-label={`Access to ${label}`}
                    value={r.access}
                    onChange={(access) => change.mutate({ repo: label, access })}
                    options={ACCESS}
                  />
                ) : (
                  <span className="text-[12.5px] text-ink-3">{accessLabel(r.access)}</span>
                )}
                {manage &&
                  (r.primary ? (
                    <span className="w-[26px] shrink-0" />
                  ) : (
                    <button
                      type="button"
                      aria-label={`Remove ${label}`}
                      className="rounded p-1.5 text-ink-4 hover:text-danger"
                      onClick={() => remove.mutate(r)}
                    >
                      <Trash2 className="size-3.5" />
                    </button>
                  ))}
              </li>
            );
          })}
        </ul>
      )}
    </div>
  );
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
    <p className="-mt-2 text-[12.5px] text-ink-3">
      Empty CPUs and Memory take the runner's default. Runners give a sandbox at most {cpus(most.cpus)} and{" "}
      {memory(most.memory_mb)}; a session that asks for more stays queued.
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

function Settings({ project }: { project: G.Project }) {
  const { data: instance } = useInstance();
  const { data: me } = useMe();
  const [s, setS] = useState<G.ProjectSettings>(project.settings);
  const [name, setName] = useState(project.name);
  const [description, setDescription] = useState(project.description ?? "");
  const [branch, setBranch] = useState(project.default_branch);
  const [envText, setEnvText] = useState(
    Object.entries(project.settings.env)
      .map(([k, v]) => `${k}=${v}`)
      .join("\n"),
  );
  const [source, setSource] = useState<ImageSource>(imageSource(project.settings));
  const typed = useRef<Typed>({});
  const [model, setModel] = useState<ModelChoice>(modelChoice(project.settings.model));
  /** The model ID typed last, so that choosing Model ID again brings it back. */
  const typedModel = useRef<string>(undefined);
  useEffect(() => {
    setS(project.settings);
    setSource(imageSource(project.settings));
    setModel(modelChoice(project.settings.model));
  }, [project.settings]);
  const save = useMutation({
    mutationFn: () => {
      if (model === "id" && !s.model?.trim())
        throw new ApiFailure(400, {
          code: "validation",
          field: "model",
          message: "Type a model ID, or choose another model",
        });
      const env: Record<string, string> = {};
      for (const line of envText.split("\n")) {
        const t = line.trim();
        if (!t || t.startsWith("#")) continue;
        const i = t.indexOf("=");
        if (i > 0) env[t.slice(0, i).trim()] = t.slice(i + 1);
      }
      return api.updateProject(project.id, {
        name,
        description,
        default_branch: branch,
        settings: { ...s, env },
      });
    },
    onSuccess: (p) => {
      queryClient.setQueryData(keys.project(project.slug), p);
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      toast("Settings saved");
    },
  });
  const set = <K extends keyof G.ProjectSettings>(k: K, v: G.ProjectSettings[K]) =>
    setS((x) => ({ ...x, [k]: v }));
  const followUps = s.follow_ups ?? NO_FOLLOW_UPS;
  const err = (f: string) =>
    (save.error as { field?: string } | null)?.field === f ? save.error?.message : undefined;
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        save.mutate();
      }}
    >
      <Section title="Project" description="How people find it.">
        <div className="flex flex-col gap-4">
          <Field label="Name" error={err("name")}>
            {(id) => <Input id={id} value={name} onChange={(e) => setName(e.target.value)} />}
          </Field>
          <Field label="Description">
            {(id) => <Input id={id} value={description} onChange={(e) => setDescription(e.target.value)} />}
          </Field>
          <Field
            label="Default branch"
            hint="Sessions start from it unless told otherwise."
            error={err("default_branch")}
          >
            {(id) => (
              <Input
                id={id}
                value={branch}
                onChange={(e) => setBranch(e.target.value)}
                className="font-mono"
              />
            )}
          </Field>
        </div>
      </Section>
      <Section title="Agents" description="What Claude is told and how it works in this project.">
        <div className="flex flex-col gap-4">
          <Field
            label="Instructions"
            hint="Appended to Claude's system prompt in every session. CLAUDE.md in the repository is read too."
            error={err("settings.instructions")}
          >
            {(id) => (
              <Textarea
                id={id}
                value={s.instructions ?? ""}
                onChange={(e) => set("instructions", e.target.value)}
                minRows={4}
                placeholder="Run `npm test` before you commit. Keep components small."
              />
            )}
          </Field>
          <div className="grid grid-cols-2 gap-4">
            <Field label="Model">
              {(id) => (
                <Select
                  id={id}
                  value={model}
                  onChange={(v) => {
                    if (model === "id") typedModel.current = s.model;
                    setModel(v);
                    set("model", chosenModel(v, typedModel.current));
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
                  onChange={(v) => set("effort", v || undefined)}
                  options={EFFORT_LEVELS}
                  className="w-full"
                />
              )}
            </Field>
          </div>
          <Field
            label="Agents on GitHub"
            hint="The most its sessions' agents may do there, and what they do unless a session asks for less. Running sessions get no more from their next GitHub token on, within the hour."
          >
            {(id) => (
              <Select
                id={id}
                value={s.github_access ?? "push"}
                onChange={(v) => set("github_access", v)}
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
          {model === "id" && (
            <Field label="Model ID" hint="Passed to Claude Code as --model." error={err("model")}>
              {(id) => (
                <Input
                  id={id}
                  value={s.model ?? ""}
                  onChange={(e) => set("model", e.target.value || undefined)}
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
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Open a pull request when done
              <span className="block text-[12.5px] text-ink-4">
                Agents are asked to open one with gh when the task is complete.
              </span>
            </span>
            <Switch
              checked={s.auto_pull_request}
              onChange={(v) => set("auto_pull_request", v)}
              label="Open pull requests"
            />
          </div>
          <Field
            label="Pull request descriptions"
            hint="How much agents write when they open a pull request. They keep the description up to date as they commit more."
          >
            {(id) => (
              <Select
                id={id}
                value={s.pull_request_descriptions ?? "balanced"}
                onChange={(v) => set("pull_request_descriptions", v)}
                options={DESCRIPTIONS}
                className="w-full"
              />
            )}
          </Field>
          <Field
            label="Pull requests and issues"
            hint="What agents that open pull requests are told about handling them and the issues they work on: fctl's rules until you change them, or none when you empty them."
            error={err("settings.pull_request_instructions")}
            action={
              !areDefaults(s.pull_request_instructions) && (
                <button
                  type="button"
                  className="text-[12px] text-ink-3 hover:text-ink"
                  onClick={() => set("pull_request_instructions", null)}
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
                onChange={(e) => set("pull_request_instructions", e.target.value)}
                minRows={4}
                maxRows={20}
                placeholder="Nothing: agents follow the repository's own instructions."
              />
            )}
          </Field>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Remote Control
              <span className="block text-[12.5px] text-ink-4">
                Follow sessions in claude.ai/code and the Claude apps (needs a full Claude login).
              </span>
            </span>
            <Switch
              checked={s.remote_control}
              onChange={(v) => set("remote_control", v)}
              label="Remote Control"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              claude.ai connectors
              <span className="block text-[12.5px] text-ink-4">
                Agents can use the connectors of the person who starts a session (mail, files, docs) without
                asking. Only with a full Claude login.
              </span>
            </span>
            <Switch
              checked={s.claude_ai_connectors}
              onChange={(v) => set("claude_ai_connectors", v)}
              label="claude.ai connectors"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Credit people in commits
              <span className="block text-[12.5px] text-ink-4">
                Commits name the person who started the session as a co-author, with the commit email in their
                profile.
              </span>
            </span>
            <Switch
              checked={s.credit_people}
              onChange={(v) => set("credit_people", v)}
              label="Credit people in commits"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Credit Claude in commits
              <span className="block text-[12.5px] text-ink-4">
                Commits name Claude as a co-author, Claude Code's default. Off: they leave out its trailer and
                the claude.ai session link.
              </span>
            </span>
            <Switch
              checked={s.credit_claude}
              onChange={(v) => set("credit_claude", v)}
              label="Credit Claude in commits"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Publish screenshots
              <span className="block text-[12.5px] text-ink-4">
                Agents can publish screenshots at links anyone who has one can open, so that pull requests
                show them: GitHub takes no uploads from a session. Off takes them offline.
              </span>
              {instance && !instance.features.publish_screenshots && (
                <span className="mt-1 block text-[12.5px] text-amber">
                  This instance's admins haven't allowed it yet
                  {me && me.user.role !== "member" ? (
                    <>
                      {" "}
                      (
                      <Link to="/admin" search={{ tab: "settings" }} className="underline underline-offset-2">
                        Admin → Settings
                      </Link>
                      )
                    </>
                  ) : null}
                  .
                </span>
              )}
            </span>
            <Switch
              checked={s.publish_screenshots}
              onChange={(v) => set("publish_screenshots", v)}
              label="Publish screenshots"
            />
          </div>
          {s.publish_screenshots && (
            <Field
              label="Screenshot links work for"
              hint="Then they stop working, and the screenshots are deleted."
              error={err("settings.screenshot_days")}
            >
              {(id) => (
                <Select
                  id={id}
                  value={String(s.screenshot_days ?? DEFAULT_DAYS)}
                  onChange={(v) => set("screenshot_days", Number(v))}
                  options={dayOptions(s.screenshot_days)}
                  className="w-full"
                />
              )}
            </Field>
          )}
        </div>
      </Section>
      <Section
        title="Pull requests"
        description="What brings an agent back to a pull request it opened. It carries on in its session, on the Claude subscription the session ran on."
      >
        <div className="flex flex-col gap-4">
          {instance && !instance.features.github_webhook && (
            <p className="text-[12.5px] leading-relaxed text-amber">
              {instance.features.github
                ? "This instance takes no GitHub webhooks yet, so nothing brings agents back. Its admins set GITHUB_WEBHOOK_SECRET."
                : "This instance has no GitHub App, so its agents open no pull requests to come back to."}
            </p>
          )}
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Comments that mention the agent
              <span className="block text-[12.5px] text-ink-4">
                People who can write to the repository write <Address>@fctl</Address> or{" "}
                <Address>@agent</Address> in a comment, a review or a comment on a line, or start a comment
                with <Address>fctl:</Address> or <Address>agent:</Address>. Apps and everyone else are
                ignored.
              </span>
            </span>
            <Switch
              checked={followUps.comments}
              onChange={(v) => set("follow_ups", { ...followUps, comments: v })}
              label="Comments that mention the agent"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Failed checks
              <span className="block text-[12.5px] text-ink-4">
                When a check of the pull request's newest commit fails, the agent fixes what its changes broke
                and pushes: up to three times in a row while nobody messages it.
              </span>
            </span>
            <Switch
              checked={followUps.checks}
              onChange={(v) => set("follow_ups", { ...followUps, checks: v })}
              label="Failed checks"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Merge conflicts
              <span className="block text-[12.5px] text-ink-4">
                When the pull request conflicts with its base branch, the agent merges the base branch in,
                resolves the conflicts and pushes: up to three times in a row while nobody messages it.
              </span>
            </span>
            <Switch
              checked={followUps.conflicts}
              onChange={(v) => set("follow_ups", { ...followUps, conflicts: v })}
              label="Merge conflicts"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Merged or closed
              <span className="block text-[12.5px] text-ink-4">
                The agent does what's left, such as noting the merge in the issue it fixed.
              </span>
            </span>
            <Switch
              checked={followUps.closed}
              onChange={(v) => set("follow_ups", { ...followUps, closed: v })}
              label="Merged or closed"
            />
          </div>
        </div>
      </Section>
      <Section
        title="Memory and retrospectives"
        description="What agents leave for the agents and people after them."
      >
        <div className="flex flex-col gap-4">
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Project memory
              <span className="block text-[12.5px] text-ink-4">
                Each session's agent starts with the notes earlier agents and people left, adds what a future
                agent should know, and marks notes that turned out wrong stale. People read and change them
                under Memory. Off, agents neither read nor add notes, and the ones there stay.
              </span>
            </span>
            <Switch
              checked={s.project_memory}
              onChange={(v) => set("project_memory", v)}
              label="Project memory"
            />
          </div>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Retrospectives
              <span className="block text-[12.5px] text-ink-4">
                As they finish a piece of work, agents record what they worked on, the approach they took,
                what got in the way and what a future agent should know. People read them on the session's
                page and under Retrospectives.
              </span>
            </span>
            <Switch
              checked={s.retrospectives}
              onChange={(v) => set("retrospectives", v)}
              label="Retrospectives"
            />
          </div>
        </div>
      </Section>
      <Section title="Sandbox" description="Where agents run.">
        <div className="flex flex-col gap-4">
          <Field
            label="Setup script"
            hint="Runs in the repository after cloning."
            error={err("settings.setup_script")}
          >
            {(id) => (
              <Textarea
                id={id}
                value={s.setup_script ?? ""}
                onChange={(e) => set("setup_script", e.target.value)}
                minRows={3}
                className="font-mono text-[12.5px]"
                placeholder={"npm ci\nnpx playwright install chromium"}
              />
            )}
          </Field>
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Package caches
              <span className="block text-[12.5px] text-ink-4">
                Sessions on the same runner share what npm, pnpm, Yarn, Bun, Cargo, uv, pip, Go and Playwright
                download, so setup scripts fetch only what's new. Each runner keeps up to 10 GiB for the
                project, or what its owner sets, and empties them past that. Reviews and Lima runners don't
                use them.
              </span>
            </span>
            <Switch
              checked={s.package_caches}
              onChange={(v) => set("package_caches", v)}
              label="Package caches"
            />
          </div>
          <Field
            label="Image"
            hint="Apple container and Docker runners make sandboxes from it. Lima runners clone the VM their host prepared, and the process driver uses none."
          >
            {(id) => (
              <Select
                id={id}
                value={source}
                onChange={(v) => {
                  typed.current = remember(s, typed.current);
                  setSource(v);
                  setS((x) => withSource(x, v, typed.current));
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
                  onChange={(e) => set("image", e.target.value || undefined)}
                  className="font-mono"
                  placeholder="ghcr.io/acme/agent:1"
                />
              )}
            </Field>
          )}
          {source === "build" && s.image_build && (
            <BuildFields
              project={project}
              build={s.image_build}
              onChange={(b) => set("image_build", b)}
              error={err("settings.image_build")}
            />
          )}
          <div className="grid grid-cols-3 gap-4">
            <Field label="CPUs" error={err("settings.cpus")}>
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  min={1}
                  max={64}
                  value={s.cpus ?? ""}
                  onChange={(e) => set("cpus", e.target.value ? Number(e.target.value) : undefined)}
                  placeholder="host"
                />
              )}
            </Field>
            <Field label="Memory (MiB)" error={err("settings.memory_mb")}>
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  min={512}
                  step={512}
                  value={s.memory_mb ?? ""}
                  onChange={(e) => set("memory_mb", e.target.value ? Number(e.target.value) : undefined)}
                  placeholder="host"
                />
              )}
            </Field>
            <Field label="Idle timeout (min)" error={err("settings.idle_timeout_minutes")}>
              {(id) => (
                <Input
                  id={id}
                  type="number"
                  min={1}
                  value={s.idle_timeout_minutes ?? ""}
                  onChange={(e) =>
                    set("idle_timeout_minutes", e.target.value ? Number(e.target.value) : undefined)
                  }
                  placeholder="10"
                />
              )}
            </Field>
          </div>
          <LargestSandbox />
          <p className="-mt-2 text-[12.5px] text-ink-3">
            A session that waits for a message longer than the idle timeout stops, and its runner takes other
            work. A message resumes it. Work its agent left running in the background keeps it up for up to an
            hour after the turn, or for the idle timeout when that is longer.
          </p>
          <Field
            label="Environment"
            hint="KEY=value per line. Not secret; use Secrets for credentials."
            error={err("settings.env")}
          >
            {(id) => (
              <Textarea
                id={id}
                value={envText}
                onChange={(e) => setEnvText(e.target.value)}
                minRows={2}
                className="font-mono text-[12.5px]"
                placeholder="NODE_ENV=development"
              />
            )}
          </Field>
        </div>
      </Section>
      <div className="sticky bottom-[max(1rem,env(safe-area-inset-bottom))] mt-4 flex justify-end">
        <Button type="submit" variant="primary" loading={save.isPending}>
          Save settings
        </Button>
      </div>
      {save.error && !(save.error as { field?: string }).field && (
        <Banner tone="error" className="mt-4">
          {save.error.message}
        </Banner>
      )}
    </form>
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

/** Project roles; only owners make owners. */
const projectRoles = (actor?: G.ProjectRole): SelectOption<G.ProjectRole>[] => [
  { value: "viewer", label: "Viewer", description: "Follows sessions" },
  { value: "editor", label: "Editor", description: "Also starts sessions and messages agents" },
  { value: "maintainer", label: "Maintainer", description: "Also manages settings, secrets and members" },
  {
    value: "owner",
    label: "Owner",
    description: "Also deletes the project and changes owners",
    disabled: actor !== "owner",
  },
];

function Members({ project }: { project: G.Project }) {
  const { data: me } = useMe();
  const members = useQuery({ queryKey: keys.members(project.id), queryFn: () => api.members(project.id) });
  const [email, setEmail] = useState("");
  const [role, setRole] = useState<G.ProjectRole>("editor");
  const manage = project.role === "owner" || project.role === "maintainer";
  const add = useMutation({
    mutationFn: () => api.addMember(project.id, { email, role }),
    onSuccess: (r) => {
      queryClient.setQueryData(keys.members(project.id), r);
      setEmail("");
    },
  });
  const change = useMutation({
    mutationFn: (v: { user: string; role: G.ProjectRole }) =>
      api.setMemberRole(project.id, v.user, { role: v.role }),
    onSuccess: (r) => queryClient.setQueryData(keys.members(project.id), r),
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: (user: string) => api.removeMember(project.id, user),
    onSuccess: () => members.refetch(),
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <div>
      {manage && (
        <form
          className="mb-6"
          onSubmit={(e) => {
            e.preventDefault();
            add.mutate();
          }}
        >
          <Field label="Add someone by email" error={add.error?.message}>
            {(id) => (
              <div className="flex gap-2">
                <Input
                  id={id}
                  type="email"
                  value={email}
                  onChange={(e) => setEmail(e.target.value)}
                  placeholder="teammate@example.com"
                  className="min-w-0 flex-1"
                />
                <Select
                  aria-label="Role"
                  value={role}
                  onChange={setRole}
                  options={projectRoles(project.role)}
                />
                <Button
                  type="submit"
                  variant="primary"
                  size="field"
                  icon={<Plus className="size-3.5" />}
                  loading={add.isPending}
                  disabled={!email}
                >
                  Add
                </Button>
              </div>
            )}
          </Field>
        </form>
      )}
      <ul className="border-t border-line">
        {members.data?.members.map((m) => (
          <li key={m.user.id} className="flex items-center gap-3 border-b border-line py-3">
            <Avatar user={m.user} size={28} />
            <div className="min-w-0 flex-1">
              <div className="text-[14px] text-ink">
                {m.user.name}
                {m.user.id === me?.user.id && <span className="ml-1.5 text-[12px] text-ink-4">you</span>}
              </div>
              <div className="text-[12px] text-ink-4">{m.email}</div>
            </div>
            {manage ? (
              <Select
                size="sm"
                aria-label={`Role of ${m.user.name}`}
                value={m.role}
                onChange={(role) => change.mutate({ user: m.user.id, role })}
                options={projectRoles(project.role)}
              />
            ) : (
              <span className="text-[12.5px] text-ink-3 capitalize">{m.role}</span>
            )}
            {(manage || m.user.id === me?.user.id) && (
              <button
                type="button"
                aria-label="Remove"
                className="rounded p-1.5 text-ink-4 hover:text-danger"
                onClick={() => remove.mutate(m.user.id)}
              >
                <Trash2 className="size-3.5" />
              </button>
            )}
          </li>
        ))}
      </ul>
      <p className="mt-4 text-[12.5px] leading-relaxed text-ink-4">
        Viewers follow sessions. Editors also start sessions and message agents. Maintainers manage settings,
        secrets and members.
      </p>
    </div>
  );
}

function Secrets({ project }: { project: G.Project }) {
  const secrets = useQuery({ queryKey: keys.secrets(project.id), queryFn: () => api.secrets(project.id) });
  const [name, setName] = useState("");
  const [value, setValue] = useState("");
  const set = useMutation({
    mutationFn: () => api.setSecret(project.id, name.trim().toUpperCase(), value),
    onSuccess: (r) => {
      queryClient.setQueryData(keys.secrets(project.id), r);
      setName("");
      setValue("");
      toast("Secret saved");
    },
  });
  const del = useMutation({
    mutationFn: (n: string) => api.deleteSecret(project.id, n),
    onSuccess: (r) => queryClient.setQueryData(keys.secrets(project.id), r),
  });
  return (
    <div>
      <Banner tone="info" className="mb-6">
        Secrets are encrypted at rest and handed to sandboxes as environment variables. They are never shown
        again — not even to you. Names that would change how Claude bills (ANTHROPIC_*) are refused.
      </Banner>
      {/* The fields line up by their labels, so an error under the name moves nothing.
          On a phone, the value goes under the name. */}
      <form
        className="mb-6 grid gap-3 sm:grid-cols-[1fr_1.4fr] sm:items-start sm:gap-2"
        onSubmit={(e) => {
          e.preventDefault();
          set.mutate();
        }}
      >
        <Field
          label="Name"
          error={(set.error as { field?: string } | null)?.field === "name" ? set.error?.message : undefined}
        >
          {(id) => (
            <Input
              id={id}
              value={name}
              onChange={(e) => setName(e.target.value)}
              className="font-mono uppercase"
              placeholder="STRIPE_KEY"
              autoComplete="off"
              autoCapitalize="off"
              spellCheck={false}
            />
          )}
        </Field>
        <Field label="Value">
          {(id) => (
            <div className="flex gap-2">
              <SecretInput
                id={id}
                value={value}
                onChange={(e) => setValue(e.target.value)}
                className="min-w-0 flex-1"
              />
              <Button
                type="submit"
                variant="primary"
                size="field"
                icon={<KeyRound className="size-3.5" />}
                loading={set.isPending}
                disabled={!name || !value}
              >
                Save
              </Button>
            </div>
          )}
        </Field>
      </form>
      <ul className="border-t border-line">
        {secrets.data?.secrets.map((x) => (
          <li key={x.name} className="flex items-center gap-3 border-b border-line py-3">
            <code className="min-w-0 flex-1 font-mono text-[13px] text-ink [overflow-wrap:anywhere]">
              {x.name}
            </code>
            <span className="text-[12px] text-ink-4">
              updated {ago(x.updated_at)}
              {x.updated_by ? ` by ${x.updated_by.name}` : ""}
            </span>
            <button
              type="button"
              aria-label={`Delete ${x.name}`}
              className="rounded p-1.5 text-ink-4 hover:text-danger"
              onClick={() => del.mutate(x.name)}
            >
              <Trash2 className="size-3.5" />
            </button>
          </li>
        ))}
        {secrets.data?.secrets.length === 0 && (
          <li className="py-6 text-[13px] text-ink-4">No secrets yet.</li>
        )}
      </ul>
    </div>
  );
}

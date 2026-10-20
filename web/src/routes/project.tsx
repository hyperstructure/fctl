// A project: its sessions, repositories, settings, people and secrets.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useParams, useSearch } from "@tanstack/react-router";
import { ArrowRight, Building2, GitBranch, KeyRound, Plus, Trash2 } from "lucide-react";
import { useEffect, useId, useRef, useState } from "react";
import {
  AttachButton,
  DraftTray,
  DropHint,
  useAttachments,
  useFileDrop,
} from "@/components/session/attachments";
import { StartButton, useStartSession } from "@/components/session/start";
import {
  Avatar,
  Banner,
  Button,
  DangerZone,
  Dialog,
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
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { NoRunnerNote } from "@/components/workspace/no-runners";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { filesFromPaste } from "@/lib/attachments";
import { cn } from "@/lib/cn";
import { ago, agoShort, cpus, memory, stamp } from "@/lib/format";
import {
  type ImageSource,
  imageSource,
  isDevcontainer,
  remember,
  type Typed,
  withSource,
} from "@/lib/images";
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
import { freshest, useWorkspaceIndex } from "@/lib/workspaces";

type Tab = "sessions" | "repositories" | "settings" | "members" | "secrets";

export function ProjectPage() {
  const { slug } = useParams({ from: "/app/p/$slug" });
  const { tab = "sessions" } = useSearch({ from: "/app/p/$slug" });
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
              className="mb-3 inline-flex items-center gap-2 text-[13px] text-ink-3 transition-colors hover:text-ink"
            >
              <WorkspaceAvatar workspace={workspace} size={18} />
              {workspace.name}
            </Link>
          )}
          <h1 className="font-display text-[38px] leading-[1.1] font-medium tracking-[-0.022em]">{p.name}</h1>
          {p.description && (
            <p className="mt-2 max-w-2xl text-[15px] leading-relaxed text-ink-3">{p.description}</p>
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
          {(["sessions", "repositories", "settings", "members", "secrets"] as Tab[])
            .filter((t) => t === "sessions" || t === "repositories" || t === "members" || manage)
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
  const list = useSessions({
    project: project.slug,
    status: filter === "active" ? "active" : undefined,
    mine: filter === "mine",
    limit: 100,
  });
  const [prompt, setPrompt] = useState("");
  const canStart = project.role !== "viewer";
  const { start, starting } = useStartSession();
  const ref = useRef<HTMLTextAreaElement>(null);
  const files = useAttachments(canStart ? { project: project.slug } : undefined);
  const attach = (f: File[], pasted?: boolean) => {
    files.add(f, { pasted });
    ref.current?.focus();
  };
  const dragging = useFileDrop(attach, canStart && !starting);
  const workspace = useWorkspaceIndex().get(project.workspace?.id ?? "");
  return (
    <div className="flex flex-col gap-10">
      {canStart && (
        <div>
          <form
            onSubmit={(e) => {
              e.preventDefault();
              start(project.slug, prompt, files);
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
              className="px-4 pt-4"
            />
            <Textarea
              ref={ref}
              bare
              minRows={2}
              value={prompt}
              readOnly={starting}
              onChange={(e) => setPrompt(e.target.value)}
              onKeyDown={(e) => {
                if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
                  e.preventDefault();
                  start(project.slug, prompt, files);
                }
              }}
              onPaste={(e) => {
                const pasted = filesFromPaste(e.clipboardData);
                if (!pasted.length || starting) return;
                e.preventDefault();
                attach(pasted, true);
              }}
              placeholder={`What should an agent do in ${project.name}?`}
              className="block px-5 pt-4 pb-14 font-display text-[17px] leading-[1.5] placeholder:text-ink-4"
            />
            <div className="absolute bottom-3 left-2.5">
              <AttachButton onFiles={(f) => attach(f)} />
            </div>
            <div className="absolute right-3 bottom-3 flex items-center gap-3">
              <span className="text-[11.5px] text-ink-4">⌘↵</span>
              <StartButton starting={starting} disabled={!prompt.trim() && !files.drafts.length}>
                Start a session
              </StartButton>
            </div>
            <DropHint show={dragging} />
          </form>
          <NoRunnerNote workspace={workspace} className="mt-3" />
        </div>
      )}
      <section>
        <div className="mb-2 flex items-center gap-1">
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
        {list.isPending ? (
          <div className="space-y-3 py-3">
            <Skeleton className="h-11" />
            <Skeleton className="h-11" />
          </div>
        ) : list.data?.sessions.length === 0 ? (
          <Empty title="No sessions yet">
            Describe a task above and an agent starts on it in a fresh sandbox.
          </Empty>
        ) : (
          <div className="border-t border-line">
            {list.data?.sessions.map((s) => (
              <Link
                key={s.id}
                to="/s/$id"
                params={{ id: s.id }}
                className="group flex items-center gap-3.5 border-b border-line p-3 transition-colors hover:bg-hover/30"
              >
                <span className="w-9 shrink-0 text-right font-mono text-[12px] text-ink-4">#{s.number}</span>
                <StatusDot status={s.status} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[14.5px] text-ink">{s.title}</div>
                  <div className="mt-0.5 flex gap-2 text-[12px] text-ink-4">
                    {s.branch && <span className="truncate font-mono">{s.branch}</span>}
                    {s.pull_request && <span>· PR #{s.pull_request.number}</span>}
                  </div>
                </div>
                <Avatar user={s.created_by} size={20} />
                <time
                  dateTime={new Date(s.last_activity_at ?? s.created_at).toISOString()}
                  title={stamp(s.last_activity_at ?? s.created_at)}
                  className="w-10 shrink-0 text-right text-[12px] whitespace-nowrap text-ink-4 tabular-nums"
                >
                  {agoShort(s.last_activity_at ?? s.created_at)}
                </time>
                <ArrowRight className="size-3.5 shrink-0 text-ink-4 opacity-0 transition-all group-hover:opacity-100 max-sm:hidden" />
              </Link>
            ))}
          </div>
        )}
      </section>
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
          className="mb-6 flex flex-wrap items-end gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            if (repo.trim()) add.mutate();
          }}
        >
          <Field label="Add a repository" className="min-w-[240px] flex-1" error={add.error?.message}>
            {(id) => (
              <>
                <Input
                  id={id}
                  list={suggestionsId}
                  value={repo}
                  onChange={(e) => setRepo(e.target.value)}
                  placeholder={`${owner ?? "owner"}/name, or ${owner ?? "owner"}/* for all of them`}
                  autoComplete="off"
                  autoCapitalize="off"
                  spellCheck={false}
                  className="font-mono text-[13px]"
                />
                <datalist id={suggestionsId}>
                  {suggestions.map((n) => (
                    <option key={n} value={n} />
                  ))}
                </datalist>
              </>
            )}
          </Field>
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
            icon={<Plus className="size-3.5" />}
            loading={add.isPending}
            disabled={!repo.trim()}
          >
            Add
          </Button>
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

/** Claude's effort levels; the empty value leaves it to Claude. */
const EFFORTS: SelectOption[] = [
  { value: "", label: "Claude's default" },
  ...["low", "medium", "high", "xhigh", "max"].map((x) => ({ value: x, label: x })),
];

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
  useEffect(() => {
    setS(project.settings);
    setSource(imageSource(project.settings));
  }, [project.settings]);
  const save = useMutation({
    mutationFn: () => {
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
            <Field label="Model" hint="opus, sonnet, fable, or a model id." error={err("model")}>
              {(id) => (
                <Input
                  id={id}
                  value={s.model ?? ""}
                  onChange={(e) => set("model", e.target.value || undefined)}
                  placeholder="Claude's default"
                />
              )}
            </Field>
            <Field label="Effort" error={err("effort")}>
              {(id) => (
                <Select
                  id={id}
                  value={s.effort ?? ""}
                  onChange={(v) => set("effort", v || undefined)}
                  options={EFFORTS}
                  className="w-full"
                />
              )}
            </Field>
          </div>
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
          className="mb-6 flex items-end gap-2"
          onSubmit={(e) => {
            e.preventDefault();
            add.mutate();
          }}
        >
          <Field label="Add someone by email" className="flex-1" error={add.error?.message}>
            {(id) => (
              <Input
                id={id}
                type="email"
                value={email}
                onChange={(e) => setEmail(e.target.value)}
                placeholder="teammate@example.com"
              />
            )}
          </Field>
          <Select aria-label="Role" value={role} onChange={setRole} options={projectRoles(project.role)} />
          <Button
            type="submit"
            variant="primary"
            icon={<Plus className="size-3.5" />}
            loading={add.isPending}
            disabled={!email}
          >
            Add
          </Button>
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
      <form
        className="mb-6 grid grid-cols-[1fr_1.4fr_auto] items-end gap-2"
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
          {(id) => <SecretInput id={id} value={value} onChange={(e) => setValue(e.target.value)} />}
        </Field>
        <Button
          type="submit"
          variant="primary"
          icon={<KeyRound className="size-3.5" />}
          loading={set.isPending}
          disabled={!name || !value}
        >
          Save
        </Button>
      </form>
      <ul className="border-t border-line">
        {secrets.data?.secrets.map((x) => (
          <li key={x.name} className="flex items-center gap-3 border-b border-line py-3">
            <code className="min-w-0 flex-1 font-mono text-[13px] text-ink">{x.name}</code>
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

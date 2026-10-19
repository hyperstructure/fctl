// Creating a project in a workspace: link one of its repositories, start a
// new one in one of its organizations, or none at all.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useNavigate, useSearch } from "@tanstack/react-router";
import { ArrowRight, FolderGit2, Lock, Plus, Sparkles } from "lucide-react";
import { useEffect, useMemo, useState } from "react";
import { Banner, Button, Empty, Field, GithubIcon, Input, Select, Switch, Textarea } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago } from "@/lib/format";
import { keys, queryClient, useInstance, useWorkspaces } from "@/lib/queries";

type Mode = "link" | "create" | "none";

export function NewProject() {
  const navigate = useNavigate();
  const search = useSearch({ from: "/app/new" });
  const { data: instance } = useInstance();
  const list = useWorkspaces();
  const open = (list.data?.workspaces ?? []).filter((w) => !w.blocked);
  const [chosen, setChosen] = useState<string | undefined>(search.workspace);
  // The workspace asked for, else the first one.
  const ws = open.find((w) => w.slug === chosen || w.id === chosen) ?? open[0];
  useEffect(() => {
    if (search.workspace) setChosen(search.workspace);
  }, [search.workspace]);
  const github = instance?.features.github;
  const works = (ws?.github.length ?? 0) > 0;
  const [mode, setMode] = useState<Mode>(github ? "link" : "none");
  const [name, setName] = useState("");
  const [description, setDescription] = useState("");
  const [repo, setRepo] = useState("");
  const [filter, setFilter] = useState("");
  const [newRepo, setNewRepo] = useState("");
  const [owner, setOwner] = useState<string>();
  const [priv, setPriv] = useState(true);
  const [setup, setSetup] = useState("");
  const [connectors, setConnectors] = useState(false);
  const repos = useQuery({
    queryKey: keys.repos(ws?.id ?? ""),
    queryFn: () => api.githubRepos(ws?.id ?? ""),
    enabled: !!github && !!ws && works && mode === "link",
  });
  const shown = useMemo(
    () =>
      (repos.data?.repos ?? [])
        .filter((r) => r.full_name.toLowerCase().includes(filter.toLowerCase()))
        .slice(0, 60),
    [repos.data, filter],
  );
  const create = useMutation({
    mutationFn: () =>
      api.createProject({
        workspace: ws?.id,
        name: name.trim() || (mode === "create" ? newRepo : (repo.split("/")[1] ?? "")) || "Untitled",
        description: description || undefined,
        repo: mode === "link" ? repo : undefined,
        create_repo:
          mode === "create"
            ? {
                name: newRepo || name,
                private: priv,
                // The organization shown; with one, the control plane picks it.
                owner: ws && ws.github.length > 1 ? (owner ?? ws.github[0]) : undefined,
              }
            : undefined,
        settings:
          setup.trim() || !connectors
            ? {
                setup_script: setup,
                auto_pull_request: true,
                remote_control: true,
                claude_ai_connectors: connectors,
                credit_people: true,
                credit_claude: true,
                publish_screenshots: false,
                env: {},
              }
            : undefined,
      }),
    onSuccess: (p) => {
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      void navigate({ to: "/p/$slug", params: { slug: p.slug }, search: { tab: undefined } });
    },
  });
  const ready = !!ws && (mode === "link" ? !!repo : mode === "create" ? !!(newRepo || name) : !!name.trim());
  if (list.data && open.length === 0)
    return (
      <div className="mx-auto max-w-[680px] px-6 py-14">
        <Empty
          icon={<FolderGit2 className="size-6" />}
          title="Projects belong to workspaces"
          action={
            <Link to="/w/new">
              <Button variant="primary" icon={<Plus className="size-3.5" />}>
                Create a workspace
              </Button>
            </Link>
          }
        >
          Create a workspace for the people and GitHub accounts you work with, or join one you are invited to
          on the floor.
        </Empty>
      </div>
    );
  return (
    <div className="mx-auto max-w-[680px] px-6 py-14">
      <h1 className="font-display text-[34px] leading-tight font-medium tracking-[-0.02em]">A new project</h1>
      <p className="mt-2 text-[15px] text-ink-3">
        A project is a repository plus what agents need to know to work in it.
      </p>
      {open.length > 1 && ws && (
        <div className="mt-8">
          <Field label="Workspace" hint="Its people work in the project, on its runners.">
            {(id) => (
              <Select
                id={id}
                value={ws.id}
                onChange={(v) => {
                  setChosen(v);
                  setRepo("");
                  setOwner(undefined);
                }}
                options={open.map((w) => ({
                  value: w.id,
                  label: w.name,
                  icon: <WorkspaceAvatar workspace={w} size={16} title={false} />,
                  description: w.github.length ? `Works in ${w.github.join(", ")}` : "No GitHub account yet",
                }))}
              />
            )}
          </Field>
        </div>
      )}
      {open.length === 1 && ws && (
        <div className="mt-6 flex items-center gap-2 text-[13px] text-ink-3">
          <WorkspaceAvatar workspace={ws} size={18} /> In {ws.name}
        </div>
      )}
      <div className="mt-8 grid grid-cols-3 gap-2">
        {(
          [
            ["link", <GithubIcon key="g" className="size-4" />, "Existing repository", !github],
            ["create", <Sparkles key="s" className="size-4" />, "New repository", !github],
            ["none", <FolderGit2 key="f" className="size-4" />, "No repository", false],
          ] as const
        ).map(([m, icon, label, disabled]) => (
          <button
            key={m}
            type="button"
            disabled={disabled}
            onClick={() => setMode(m)}
            className={cn(
              "flex flex-col items-start gap-2 rounded-lg border px-3.5 py-3 text-left text-[13px] transition-all duration-150 disabled:opacity-40",
              mode === m
                ? "border-accent/50 bg-accent-soft text-ink shadow-[0_0_0_3px_var(--c-accent-soft)]"
                : "border-line-strong text-ink-3 hover:border-ink-4 hover:text-ink",
            )}
          >
            <span className={cn(mode === m ? "text-accent" : "text-ink-4")}>{icon}</span>
            {label}
          </button>
        ))}
      </div>
      {!github && (
        <Banner tone="info" className="mt-4">
          Connect a GitHub App to this instance to link repositories (see the self-hosting guide).
        </Banner>
      )}
      {github && ws && !works && mode !== "none" && (
        <Banner tone="info" className="mt-4">
          {ws.name} works in no GitHub account yet.{" "}
          <Link
            to="/w/$slug"
            params={{ slug: ws.slug }}
            search={{ tab: "github" }}
            className="underline underline-offset-2"
          >
            Connect one
          </Link>{" "}
          to link its repositories.
        </Banner>
      )}
      <form
        className="mt-8 flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          create.mutate();
        }}
      >
        {mode === "link" && (
          <Field label="Repository">
            {(id) => (
              <div className="overflow-hidden rounded-md border border-line-strong">
                <input
                  id={id}
                  value={filter}
                  onChange={(e) => setFilter(e.target.value)}
                  placeholder="Search repositories"
                  className="h-9.5 w-full border-b border-line bg-sunken/60 px-3 text-[14px] outline-none placeholder:text-ink-4"
                />
                <div className="max-h-64 overflow-y-auto">
                  {repos.isPending && (
                    <div className="px-3 py-3 text-[13px] text-ink-4">Loading repositories…</div>
                  )}
                  {repos.isError && (
                    <div className="px-3 py-3 text-[13px] text-danger">{repos.error.message}</div>
                  )}
                  {shown.map((r) => (
                    <button
                      key={r.full_name}
                      type="button"
                      disabled={!!r.project_id}
                      onClick={() => {
                        setRepo(r.full_name);
                        if (!name) setName(r.name);
                        if (!description && r.description) setDescription(r.description);
                      }}
                      className={cn(
                        "flex w-full items-center gap-3 px-3 py-2 text-left text-[13px] transition-colors disabled:opacity-40",
                        repo === r.full_name ? "bg-accent-soft text-ink" : "text-ink-2 hover:bg-hover",
                      )}
                    >
                      {r.private ? (
                        <Lock className="size-3.5 text-ink-4" />
                      ) : (
                        <GithubIcon className="size-3.5 text-ink-4" />
                      )}
                      <span className="min-w-0 flex-1 truncate font-mono text-[12.5px]">{r.full_name}</span>
                      <span className="shrink-0 text-[11.5px] text-ink-4">
                        {r.project_id ? "already a project" : r.pushed_at ? ago(Date.parse(r.pushed_at)) : ""}
                      </span>
                    </button>
                  ))}
                </div>
              </div>
            )}
          </Field>
        )}
        {mode === "create" && (
          <div className="flex items-end gap-4">
            {ws && ws.github.length > 1 && (
              <Field label="Organization">
                {(id) => (
                  <Select
                    id={id}
                    value={owner ?? ws.github[0]}
                    onChange={setOwner}
                    options={ws.github.map((a) => ({ value: a, label: a }))}
                  />
                )}
              </Field>
            )}
            <Field
              label="Repository name"
              className="flex-1"
              hint={`Created in ${owner ?? (ws?.github.length === 1 ? ws.github[0] : "the workspace's organization")}, with a README.`}
            >
              {(id) => (
                <Input
                  id={id}
                  value={newRepo}
                  onChange={(e) => setNewRepo(e.target.value)}
                  placeholder="my-new-app"
                />
              )}
            </Field>
            <div className="mb-7 flex items-center gap-2 text-[13px] text-ink-2">
              <Switch checked={priv} onChange={setPriv} label="Private" /> Private
            </div>
          </div>
        )}
        <Field
          label="Name"
          error={
            create.error && (create.error as { field?: string }).field === "name"
              ? create.error.message
              : undefined
          }
        >
          {(id) => (
            <Input
              id={id}
              value={name}
              onChange={(e) => setName(e.target.value)}
              placeholder={mode === "link" ? repo.split("/")[1] || "My app" : "My app"}
            />
          )}
        </Field>
        <Field label="Description" hint="Optional. Shown to people, not to agents.">
          {(id) => <Input id={id} value={description} onChange={(e) => setDescription(e.target.value)} />}
        </Field>
        <Field
          label="Setup script"
          hint="Runs in the repository before the agent starts, e.g. npm ci. You can change it later."
        >
          {(id) => (
            <Textarea
              id={id}
              value={setup}
              onChange={(e) => setSetup(e.target.value)}
              minRows={2}
              className="font-mono text-[12.5px]"
              placeholder="npm ci"
            />
          )}
        </Field>
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            claude.ai connectors
            <span className="block text-[12.5px] text-ink-4">
              Agents can use the connectors of the person who starts a session (mail, files, docs) without
              asking. Only with a full Claude login.
            </span>
          </span>
          <Switch checked={connectors} onChange={setConnectors} label="claude.ai connectors" />
        </div>
        {create.error && !(create.error as { field?: string }).field && (
          <Banner tone="error">{create.error.message}</Banner>
        )}
        <div className="flex justify-end">
          <Button
            type="submit"
            variant="primary"
            size="lg"
            loading={create.isPending}
            disabled={!ready}
            trailing={<ArrowRight className="size-4" />}
          >
            Create project
          </Button>
        </div>
      </form>
    </div>
  );
}

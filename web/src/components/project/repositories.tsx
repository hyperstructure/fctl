// A project's Repositories tab: where its agents work, the primary one first,
// and what they may do in each. Everyone in the project sees them; its maintainers add and remove
// them, and change what agents may do.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Building2, Plus, Trash2 } from "lucide-react";
import { useState } from "react";
import { RepoPicker } from "@/components/project/repo-picker";
import { Button, Field, GithubIcon, Select, type SelectOption, Skeleton, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { manages } from "@/lib/project-page";
import { keys, queryClient, useInstance, useProjectRepos } from "@/lib/queries";
import { parseRepo, repoAdditions, repoChoices, repoValue } from "@/lib/repos";
import { useWorkspaceIndex } from "@/lib/workspaces";

const ACCESS: SelectOption<G.RepoAccess>[] = [
  { value: "read", label: "Read only", description: "Clone, fetch, and read issues and pull requests" },
  { value: "write", label: "Read and write", description: "Also push branches and open pull requests" },
];
const accessLabel = (a: G.RepoAccess) => (a === "write" ? "Read and write" : "Read only");
/** GitHub names ignore case. */
const same = (a: string, b?: string) => a.toLowerCase() === b?.toLowerCase();

/** The Repositories tab: the ones its agents work in, and for its maintainers, adding and removing them. */
export function Repositories({ project }: { project: G.Project }) {
  const manage = manages(project.role);
  const { data: instance } = useInstance();
  const repos = useProjectRepos(project.id);
  const workspace = project.workspace?.id ?? "";
  const accounts = useWorkspaceIndex().get(workspace)?.github ?? [];
  // The repositories of the project's workspace.
  const github = useQuery({
    queryKey: keys.repos(workspace),
    queryFn: () => api.githubRepos(workspace),
    enabled: manage && !!workspace && !!instance?.features.github,
  });
  const [chosen, setChosen] = useState<string[]>([]);
  const [query, setQuery] = useState("");
  const [access, setAccess] = useState<G.RepoAccess>("read");
  const list = repos.data?.repos ?? [];
  const owner = list[0]?.owner;
  const choices = repoChoices({ listed: list, available: github.data?.repos ?? [], accounts, chosen, query });
  const adding = repoAdditions(chosen, query, list);
  const wide = adding.length > 0 && adding.every((v) => v.endsWith("/*"));
  const show = (r: G.ProjectRepoList) => queryClient.setQueryData(keys.projectRepos(project.id), r);
  // The project's header and the lists of projects count them.
  const recount = () => {
    for (const k of [keys.project(project.slug), keys.project(project.id), keys.projects])
      void queryClient.invalidateQueries({ queryKey: k });
  };
  const save = (r: G.ProjectRepoList) => {
    show(r);
    recount();
  };
  // One by one, in the order they were chosen: those added leave the field, and the first one the
  // control plane refuses stays in it with the ones after it, under its reason.
  const add = useMutation({
    mutationFn: async (values: string[]) => {
      for (const repo of values) {
        let r: G.ProjectRepoList;
        try {
          r = await api.addProjectRepo(project.id, { repo, access: repo.endsWith("/*") ? "read" : access });
        } catch (e) {
          const message = e instanceof Error ? e.message : String(e);
          throw new Error(
            message.toLowerCase().includes(repo.toLowerCase()) ? message : `${repo}: ${message}`,
          );
        }
        show(r);
        setChosen((c) => c.filter((v) => !same(v, repo)));
        setQuery((q) => {
          const t = parseRepo(q);
          return t && same(repoValue(t), repo) ? "" : q;
        });
      }
    },
    onSuccess: () => setAccess("read"),
    onSettled: recount,
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
  // A new choice clears what the control plane said of the last one.
  const edited = () => {
    if (add.isError) add.reset();
  };
  const status = github.isError ? (
    `fctl couldn't list the workspace's repositories: ${github.error.message}`
  ) : github.isFetching && !github.data ? (
    "Listing the workspace's repositories…"
  ) : choices.some((c) => c.elsewhere) ? (
    "All of a project's repositories belong to one GitHub account."
  ) : query.trim() && choices.length === 0 ? (
    <>
      No repository on the list matches. Type one as{" "}
      <code className="font-mono text-[11.5px] text-ink-2">owner/name</code> to add it by hand.
    </>
  ) : undefined;
  return (
    <div>
      <p className="mb-6 max-w-2xl text-[13.5px] leading-relaxed text-ink-3">
        Agents start in the primary repository and clone the others when they need them, into{" "}
        <code className="font-mono text-[12.5px] text-ink-2">~/repos/&lt;owner&gt;/&lt;name&gt;</code>. They
        push and open pull requests only where they can write. All of a project's repositories belong to one
        GitHub account, and changes apply to sessions that start afterwards.
      </p>
      {manage && (
        <form
          className="mb-6"
          onSubmit={(e) => {
            e.preventDefault();
            if (adding.length && !add.isPending) add.mutate(adding);
          }}
        >
          <Field label="Add repositories" error={add.error?.message}>
            {(id) => (
              // The access and the button stay by the field's first line as what's chosen fills more.
              <div className="flex flex-wrap items-start gap-2">
                <div className="min-w-[240px] flex-1">
                  <RepoPicker
                    id={id}
                    choices={choices}
                    chosen={chosen}
                    onChange={(c) => {
                      setChosen(c);
                      edited();
                    }}
                    query={query}
                    onQuery={(q) => {
                      setQuery(q);
                      edited();
                    }}
                    loaded={!!github.data}
                    status={status}
                    placeholder={`Choose from the list, or type ${owner ?? "owner"}/name`}
                    invalid={add.isError}
                  />
                </div>
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
                  disabled={adding.length === 0}
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

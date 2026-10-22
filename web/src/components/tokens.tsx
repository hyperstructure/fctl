// Settings' access tokens: for scripts, CI and bots, each limited to what it is for, with a
// sentence that says what it does. A new one is made in a dialog that says what it will do
// before it does, and shows the secret once.

import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowUpRight, KeyRound, Plus } from "lucide-react";
import { useId, useMemo, useState } from "react";
import { GithubAccessIcon } from "@/components/github-access";
import { Banner, Button, CodeBox, Dialog, Field, Input, Section, Select } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago } from "@/lib/format";
import { GITHUB_ACCESS } from "@/lib/github-access";
import { siteUrl } from "@/lib/install";
import { keys, queryClient, useProjects, useWorkspaces } from "@/lib/queries";
import {
  describeScope,
  EXPIRIES,
  example,
  formScope,
  NEW_TOKEN,
  reach,
  ready,
  TOKEN_ACCESS,
  type TokenForm,
  tokenRequest,
} from "@/lib/tokens";

/** The guide to agents in CI, review agents and bots. */
const GUIDE = () => `${siteUrl()}/docs/automation/`;

export function AccessTokens() {
  const list = useQuery({ queryKey: keys.tokens, queryFn: api.tokens });
  const [open, setOpen] = useState(false);
  const revoke = useMutation({
    mutationFn: (id: string) => api.deleteToken(id),
    onSuccess: () => list.refetch(),
  });
  const tokens = list.data?.tokens.filter((t) => t.kind === "pat") ?? [];
  return (
    <Section
      title="Access tokens"
      description="For scripts, CI and bots: each works as you, limited to what it is for, until it expires or you revoke it."
      actions={
        <Button size="sm" icon={<Plus className="size-3.5" />} onClick={() => setOpen(true)}>
          New access token
        </Button>
      }
    >
      {tokens.length ? (
        <ul className="divide-y divide-line border-y border-line">
          {tokens.map((t) => (
            <li key={t.id} className="flex items-start gap-3 py-3">
              <KeyRound className="mt-0.5 size-4 shrink-0 text-ink-4" />
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13px] text-ink">
                  {t.name}
                  {t.current && <span className="ml-2 text-[11.5px] text-accent">this token</span>}
                </div>
                <p className="mt-0.5 text-[12.5px] leading-relaxed text-ink-3 [overflow-wrap:anywhere]">
                  {describeScope(t.scope)}
                </p>
                <div className="mt-0.5 text-[11.5px] text-ink-4">
                  {t.last_used_at ? `Used ${ago(t.last_used_at)}` : "Not used yet"}
                  {t.expires_at ? ` · expires ${ago(t.expires_at)}` : " · never expires"}
                </div>
              </div>
              <Button
                size="sm"
                variant="ghost"
                loading={revoke.isPending && revoke.variables === t.id}
                onClick={() => revoke.mutate(t.id)}
              >
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="max-w-xl text-[13px] leading-relaxed text-ink-3">
          None yet. A token for CI can start review agents on your pull requests that comment but can't push;
          one for a dashboard can read sessions and change nothing.{" "}
          <a href={GUIDE()} target="_blank" rel="noreferrer" className="text-accent hover:underline">
            How agents run from CI
          </a>
        </p>
      )}
      <NewTokenDialog open={open} onClose={() => setOpen(false)} />
    </Section>
  );
}

/** Makes an access token: what it does, where, what its agents may do on GitHub, how long it works. */
export function NewTokenDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [form, setForm] = useState<TokenForm>(NEW_TOKEN);
  const [created, setCreated] = useState<G.CreatedToken | null>(null);
  const set = (f: Partial<TokenForm>) => setForm((was) => ({ ...was, ...f }));
  const create = useMutation({
    mutationFn: () => api.createToken(tokenRequest(form)),
    onSuccess: (t) => {
      setCreated(t);
      void queryClient.invalidateQueries({ queryKey: keys.tokens });
    },
  });
  const close = () => {
    onClose();
    setForm(NEW_TOKEN);
    setCreated(null);
    create.reset();
  };
  return (
    <Dialog
      open={open}
      onClose={close}
      wide
      title={created ? `${created.token.name ?? "Your token"} is ready` : "New access token"}
      description={
        created ? describeScope(created.token.scope) : "It works as you, within the limits you set here."
      }
    >
      {created ? (
        <Created token={created} onDone={close} />
      ) : (
        <TokenFormView form={form} set={set} create={create} />
      )}
    </Dialog>
  );
}

function TokenFormView({
  form,
  set,
  create,
}: {
  form: TokenForm;
  set: (f: Partial<TokenForm>) => void;
  create: { mutate: () => void; isPending: boolean; error: Error | null };
}) {
  const workspaces = useWorkspaces().data?.workspaces ?? [];
  const projects = useProjects().data?.projects ?? [];
  const names = useMemo(
    () => ({
      workspaces: new Map(workspaces.map((w) => [w.id, w.name])),
      projects: new Map(projects.map((p) => [p.id, p.name])),
    }),
    [workspaces, projects],
  );
  const scope = formScope(form, names);
  const expiry = EXPIRIES.find((e) => e.value === form.expires)?.days;
  const field = (create.error as { field?: string } | null)?.field;
  const err = (f: string) => (field === f ? create.error?.message : undefined);
  const id = useId();
  return (
    <form
      className="flex flex-col gap-6"
      onSubmit={(e) => {
        e.preventDefault();
        if (ready(form)) create.mutate();
      }}
    >
      <Field label="Name" hint="What it's for, so that you know it in your list." error={err("name")}>
        {(id) => (
          <Input
            id={id}
            value={form.name}
            onChange={(e) => set({ name: e.target.value })}
            placeholder="PR reviews"
            autoFocus
          />
        )}
      </Field>

      <fieldset className="flex flex-col gap-2.5">
        <legend className="mb-2 text-[12.5px] font-medium text-ink-2">What it can do</legend>
        {TOKEN_ACCESS.map((a) => (
          <Choice
            key={a.value}
            name={`${id}-access`}
            checked={form.access === a.value}
            onChange={() => set({ access: a.value })}
            label={a.label}
            description={a.description}
          />
        ))}
      </fieldset>

      <fieldset className="flex flex-col gap-2.5">
        <legend className="mb-2 text-[12.5px] font-medium text-ink-2">Where</legend>
        <Choice
          name={`${id}-where`}
          checked={!form.places}
          onChange={() => set({ places: undefined })}
          label="All your projects"
          description="Those of workspaces you join later too."
        />
        <Choice
          name={`${id}-where`}
          checked={!!form.places}
          onChange={() => set({ places: form.places ?? { workspaces: [], projects: [] } })}
          label="Only some"
          description="The projects you choose, or every project of a workspace."
        />
        {form.places && (
          <Places
            workspaces={workspaces}
            projects={projects}
            places={form.places}
            onChange={(places) => set({ places })}
            error={err("projects") ?? err("workspaces")}
          />
        )}
      </fieldset>

      <div className="grid items-start gap-4 sm:grid-cols-2">
        {form.access !== "read" && (
          <Field
            label="Its sessions' agents on GitHub"
            hint="At most this, within each project's own limit. It can't message or resume a session whose agent may do more."
          >
            {(fid) => (
              <Select
                id={fid}
                value={form.github}
                onChange={(github) => set({ github })}
                options={GITHUB_ACCESS.map((l) => ({
                  value: l.value,
                  label: l.label,
                  icon: <GithubAccessIcon access={l.value} />,
                  description: l.description,
                }))}
              />
            )}
          </Field>
        )}
        <Field label="Expires" error={err("expires_in_days")}>
          {(fid) => (
            <Select
              id={fid}
              value={form.expires}
              onChange={(expires) => set({ expires })}
              options={EXPIRIES.map((e) => ({ value: e.value, label: e.label }))}
            />
          )}
        </Field>
      </div>

      <div
        aria-live="polite"
        className="rounded-lg border border-line bg-sunken/50 px-4 py-3 text-[13px] leading-relaxed text-ink-2"
      >
        {describeScope(scope)} {reach(scope)}{" "}
        {expiry
          ? `It expires on ${new Date(Date.now() + expiry * 86_400_000).toLocaleDateString(undefined, { dateStyle: "long" })}.`
          : "It never expires: revoke it when you no longer need it."}
      </div>

      {create.error && !field && <Banner tone="error">{create.error.message}</Banner>}

      <div className="flex justify-end gap-2">
        <Button type="submit" variant="primary" loading={create.isPending} disabled={!ready(form)}>
          Create token
        </Button>
      </div>
    </form>
  );
}

/** One of a group of radio buttons, with what it means under its label. */
function Choice({
  name,
  checked,
  onChange,
  label,
  description,
}: {
  name: string;
  checked: boolean;
  onChange: () => void;
  label: string;
  description: string;
}) {
  return (
    <label className="flex cursor-pointer items-start gap-2.5 text-[13.5px] text-ink select-none">
      <input
        type="radio"
        name={name}
        checked={checked}
        onChange={onChange}
        className="mt-[3px] size-4 shrink-0 accent-[var(--c-accent)]"
      />
      <span>
        {label}
        <span className="block text-[12.5px] leading-relaxed text-ink-3">{description}</span>
      </span>
    </label>
  );
}

/** The workspaces and projects a token reaches: a workspace stands for all its projects. */
function Places({
  workspaces,
  projects,
  places,
  onChange,
  error,
}: {
  workspaces: G.Workspace[];
  projects: G.Project[];
  places: { workspaces: string[]; projects: string[] };
  onChange: (p: { workspaces: string[]; projects: string[] }) => void;
  error?: string;
}) {
  const toggle = (list: string[], id: string, on: boolean) =>
    on ? [...list, id] : list.filter((x) => x !== id);
  const groups = workspaces
    .map((w) => ({ w, projects: projects.filter((p) => p.workspace?.id === w.id && !p.archived_at) }))
    .filter((g) => g.projects.length > 0);
  return (
    <div className="flex flex-col gap-2 sm:pl-6.5">
      <ul className="max-h-72 overflow-y-auto rounded-lg border border-line">
        {groups.map(({ w, projects }) => {
          const whole = places.workspaces.includes(w.id);
          return (
            <li key={w.id} className="border-b border-line last:border-b-0">
              <label className="flex cursor-pointer items-center gap-3 px-3 py-2 hover:bg-hover">
                <input
                  type="checkbox"
                  checked={whole}
                  onChange={(e) =>
                    onChange({
                      workspaces: toggle(places.workspaces, w.id, e.target.checked),
                      // Its projects come with it.
                      projects: places.projects.filter((id) => !projects.some((p) => p.id === id)),
                    })
                  }
                  className="size-4 shrink-0 accent-[var(--c-accent)]"
                />
                <WorkspaceAvatar workspace={w} size={16} title={false} />
                <span className="min-w-0 flex-1 text-[13px] text-ink [overflow-wrap:anywhere]">
                  {w.name}
                  <span className="block text-[12px] text-ink-4">Every project, those made later too</span>
                </span>
              </label>
              <ul>
                {projects.map((p) => (
                  <li key={p.id}>
                    <label
                      className={cn(
                        "flex items-center gap-3 py-1.5 pr-3 pl-9 hover:bg-hover",
                        whole ? "cursor-default opacity-60" : "cursor-pointer",
                      )}
                    >
                      <input
                        type="checkbox"
                        checked={whole || places.projects.includes(p.id)}
                        disabled={whole}
                        onChange={(e) =>
                          onChange({ ...places, projects: toggle(places.projects, p.id, e.target.checked) })
                        }
                        className="size-4 shrink-0 accent-[var(--c-accent)]"
                      />
                      <span className="min-w-0 text-[13px] text-ink-2 [overflow-wrap:anywhere]">
                        {p.name}
                      </span>
                    </label>
                  </li>
                ))}
              </ul>
            </li>
          );
        })}
        {groups.length === 0 && <li className="px-3 py-2 text-[13px] text-ink-4">You see no project yet.</li>}
      </ul>
      {error ? (
        <p role="alert" className="text-[12.5px] text-danger">
          {error}
        </p>
      ) : (
        places.workspaces.length + places.projects.length === 0 && (
          <p className="text-[12.5px] text-ink-3">Choose at least one.</p>
        )
      )}
    </div>
  );
}

/** The new token, once: the secret, how to use it, and where to read more. */
function Created({ token, onDone }: { token: G.CreatedToken; onDone: () => void }) {
  return (
    <div className="flex flex-col gap-4">
      <div>
        <Banner tone="warn">Copy it now: it is shown once.</Banner>
        <CodeBox value={token.secret} className="mt-2" />
      </div>
      <div>
        <p className="text-[13px] leading-relaxed text-ink-3">
          Use it as <code className="rounded bg-raised px-1 font-mono text-[11.5px]">FCTL_TOKEN</code> with
          the CLI, or in an{" "}
          <code className="rounded bg-raised px-1 font-mono text-[11.5px]">Authorization: Bearer</code>{" "}
          header:
        </p>
        <CodeBox value={example(token.token.scope, token.secret)} className="mt-2" />
      </div>
      <div className="flex flex-wrap items-center justify-between gap-3">
        <a
          href={GUIDE()}
          target="_blank"
          rel="noreferrer"
          className="inline-flex items-center gap-1 text-[13px] text-accent hover:underline"
        >
          Review pull requests from GitHub Actions <ArrowUpRight className="size-3.5" />
        </a>
        <Button variant="primary" onClick={onDone}>
          Done
        </Button>
      </div>
    </div>
  );
}

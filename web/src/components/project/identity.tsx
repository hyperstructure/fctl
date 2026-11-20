// Settings → Identity OIDC: whether the project's agents mint identity tokens, which prove to your own
// services and clouds that a request comes from one of its sessions, its workspace's issuer and the
// subject of its tokens, and the clouds its agents reach with them (AWS, Google Cloud, Azure), each
// with what to set up there so that it trusts them. The project's owners and maintainers see it.

import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowUpRight, ChevronRight } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { SwitchRow, SwitchRows, sameDraft, UnsavedGuard, useDraft } from "@/components/settings/form";
import {
  Banner,
  CodeBox,
  CopyButton,
  Field,
  Input,
  SaveBar,
  Section,
  SectionLayout,
  Skeleton,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { cloudAccess, IDENTITY_FIELDS, type IdentityDraft, identityDraft } from "@/lib/identity";
import { siteUrl } from "@/lib/install";
import { placeOf } from "@/lib/project-page";
import { keys, queryClient } from "@/lib/queries";

/** The field a failed save names, if any. */
const failedField = (e: Error | null) => (e as { field?: string } | null)?.field;

/** Inputs that hold ids and addresses, which no browser should correct. */
const literal = { autoCapitalize: "off", autoCorrect: "off", spellCheck: false } as const;

export function IdentitySettings({ project }: { project: G.Project }) {
  const saved = identityDraft(project.settings);
  const { draft: d, set, setDraft, dirty, reset } = useDraft(saved);
  const identity = useQuery({
    queryKey: keys.projectIdentity(project.id),
    queryFn: () => api.projectIdentity(project.id),
  });
  const save = useMutation({
    mutationFn: () =>
      api.updateProject(project.id, {
        settings: { ...project.settings, id_tokens: d.id_tokens, cloud_access: cloudAccess(d) },
      }),
    onSuccess: (p) => {
      queryClient.setQueryData(keys.project(project.slug), p);
      // What was stored, as the control plane tidied it: nothing is left to save.
      setDraft(identityDraft(p.settings));
      void queryClient.invalidateQueries({ queryKey: keys.projects });
      void queryClient.invalidateQueries({ queryKey: keys.projectIdentity(project.id) });
      toast("Saved: sessions take it as they next start or resume");
    },
  });
  const field = failedField(save.error);
  const err = (f: keyof typeof IDENTITY_FIELDS) =>
    field === IDENTITY_FIELDS[f] ? save.error?.message : undefined;
  const id = identity.data;
  /** What a cloud needs, once what the page shows of it is what's saved. */
  const setup = (fields: (keyof IdentityDraft)[]) => fields.every((f) => sameDraft(d[f], saved[f]));
  return (
    <Section
      title="Identity OIDC"
      description="How the project's agents prove who they are to your own services and clouds: with identity tokens your workspace signs, which last 10 minutes, so that they need no keys of yours."
    >
      <form
        onSubmit={(e) => {
          e.preventDefault();
          if (dirty) save.mutate();
        }}
      >
        <SwitchRows>
          <SwitchRow
            label="Identity tokens"
            hint={
              <>
                Agents mint them with <code>fctl id-token</code>: OpenID Connect ID tokens that say whom a
                session runs for, its project and runner, and what it may do on GitHub. A service takes them
                as bearer tokens and checks them with your workspace's keys. Review agents never mint them.
              </>
            }
            checked={d.id_tokens}
            onChange={(v) => set({ id_tokens: v })}
          />
        </SwitchRows>
        <Issuer identity={id} loading={identity.isPending} />
        <SectionLayout value="part">
          <div className="mt-14 flex flex-col">
            {!d.id_tokens && (
              <Banner tone="info" className="mb-8">
                While identity tokens are off, the project's agents reach none of these clouds.
              </Banner>
            )}
            <Section
              title="AWS"
              description={`The role the project's agents assume: the AWS CLI and SDKs sign in as it on their own, and CloudTrail names each session (fctl-${project.slug}-12).`}
            >
              <div className="flex flex-col gap-4">
                <div className="grid items-start gap-4 sm:grid-cols-[minmax(0,1fr)_11rem]">
                  <Field label="Role ARN" error={err("aws_role")}>
                    {(i) => (
                      <Input
                        id={i}
                        value={d.aws_role}
                        onChange={(e) => set({ aws_role: e.target.value })}
                        className="font-mono"
                        placeholder="arn:aws:iam::123456789012:role/fctl-agents"
                        {...literal}
                      />
                    )}
                  </Field>
                  <Field label="Region" error={err("aws_region")}>
                    {(i) => (
                      <Input
                        id={i}
                        value={d.aws_region}
                        onChange={(e) => set({ aws_region: e.target.value })}
                        className="font-mono"
                        placeholder="The SDKs' own"
                        disabled={!d.aws_role.trim()}
                        {...literal}
                      />
                    )}
                  </Field>
                </div>
                <SwitchRows>
                  <SwitchRow
                    label="Session tags"
                    hint={
                      <>
                        Tokens carry the project, the person, the session and more as session tags (
                        <code>fctl:project</code>, <code>fctl:user</code>…), for conditions in the role's
                        trust policy and permissions on them. The trust policy must then allow{" "}
                        <code>sts:TagSession</code>, or AWS refuses the role.
                      </>
                    }
                    checked={d.aws_session_tags}
                    onChange={(v) => set({ aws_session_tags: v })}
                    disabled={!d.aws_role.trim()}
                  />
                </SwitchRows>
                {id?.aws && setup(["aws_role", "aws_session_tags"]) && (
                  <Setup cloud="AWS">
                    <Commands text={id.aws.commands} />
                    <Commands label="trust-policy.json" text={id.aws.trust_policy} />
                  </Setup>
                )}
              </div>
            </Section>
            <Section
              title="Google Cloud"
              description="The workload identity provider the project's agents' tokens go to: the client libraries and gcloud sign in through it on their own."
            >
              <div className="flex flex-col gap-4">
                <Field
                  label="Workload identity provider"
                  hint="Its resource name, as its page in the console shows it."
                  error={err("gcp_provider")}
                >
                  {(i) => (
                    <Input
                      id={i}
                      value={d.gcp_provider}
                      onChange={(e) => set({ gcp_provider: e.target.value })}
                      className="font-mono"
                      placeholder="projects/123456789/locations/global/workloadIdentityPools/fctl/providers/acme"
                      {...literal}
                    />
                  )}
                </Field>
                <Field
                  label="Service account"
                  hint="The agents act as it. Empty: give the project's agents roles themselves."
                  error={err("gcp_service_account")}
                >
                  {(i) => (
                    <Input
                      id={i}
                      value={d.gcp_service_account}
                      onChange={(e) => set({ gcp_service_account: e.target.value })}
                      className="font-mono"
                      placeholder="agents@my-project.iam.gserviceaccount.com"
                      disabled={!d.gcp_provider.trim()}
                      {...literal}
                    />
                  )}
                </Field>
                {id?.gcp && setup(["gcp_provider", "gcp_service_account"]) && (
                  <Setup cloud="Google Cloud">
                    <Commands text={id.gcp.commands} />
                  </Setup>
                )}
              </div>
            </Section>
            <Section
              title="Azure"
              description="The Microsoft Entra application, or managed identity, the project's agents act as: the Azure SDKs sign in as it on their own."
            >
              <div className="flex flex-col gap-4">
                <div className="grid items-start gap-4 sm:grid-cols-2">
                  <Field label="Tenant ID" error={err("azure_tenant")}>
                    {(i) => (
                      <Input
                        id={i}
                        value={d.azure_tenant}
                        onChange={(e) => set({ azure_tenant: e.target.value })}
                        className="font-mono"
                        placeholder="00000000-0000-0000-0000-000000000000"
                        {...literal}
                      />
                    )}
                  </Field>
                  <Field label="Client ID" error={err("azure_client")}>
                    {(i) => (
                      <Input
                        id={i}
                        value={d.azure_client}
                        onChange={(e) => set({ azure_client: e.target.value })}
                        className="font-mono"
                        placeholder="00000000-0000-0000-0000-000000000000"
                        {...literal}
                      />
                    )}
                  </Field>
                </div>
                {id?.azure && setup(["azure_tenant", "azure_client"]) && (
                  <Setup cloud="Azure">
                    <p className="text-[12.5px] leading-relaxed text-ink-3">
                      A federated credential on the application, with the subject{" "}
                      <code className="font-mono text-ink-2">{id.azure.subject}</code>: Azure matches it
                      exactly, so its tokens name the project alone.
                    </p>
                    <Commands text={id.azure.commands} />
                  </Setup>
                )}
              </div>
            </Section>
          </div>
        </SectionLayout>
        <UnsavedGuard dirty={dirty && !save.isPending} page="Identity OIDC" at={(l) => placeOf(project, l)} />
        {(dirty || save.isPending) && (
          <SaveBar
            pending={save.isPending}
            hint="Sessions take it as they next start or resume."
            onReset={() => {
              reset();
              save.reset();
            }}
          />
        )}
        {save.error && !(field && Object.values<string>(IDENTITY_FIELDS).includes(field)) && (
          <Banner tone="error" className="mt-4">
            {save.error.message}
          </Banner>
        )}
      </form>
    </Section>
  );
}

/** The workspace's issuer, the subject of the project's tokens, and where services read more. */
function Issuer({ identity, loading }: { identity?: G.ProjectIdentity; loading: boolean }) {
  return (
    <div className="mt-6 flex flex-col gap-4">
      <Labeled
        label="Issuer"
        hint="Your workspace's: services trust it alone, and check tokens with its keys."
      >
        {loading ? <Skeleton className="h-10.5 w-full" /> : identity && <CodeBox value={identity.issuer} />}
      </Labeled>
      <Labeled label="Subject" hint="The project and the person a session runs for: what a cloud matches.">
        {loading ? <Skeleton className="h-10.5 w-full" /> : identity && <CodeBox value={identity.subject} />}
      </Labeled>
      <a
        href={`${siteUrl()}/docs/identity-tokens`}
        target="_blank"
        rel="noreferrer"
        className="inline-flex w-fit items-center gap-0.5 text-[12.5px] text-ink-3 underline-offset-2 hover:text-ink hover:underline"
      >
        What tokens say, and how services check them <ArrowUpRight className="size-3" />
      </a>
    </div>
  );
}

/** A value with its label and a hint under it, which no form field holds. */
function Labeled({ label, hint, children }: { label: string; hint: string; children: ReactNode }) {
  return (
    <div className="flex flex-col gap-1.5">
      <span className="text-[13px] font-medium text-ink-2">{label}</span>
      {children}
      <span className="text-[12px] leading-relaxed text-ink-4">{hint}</span>
    </div>
  );
}

/** What a cloud needs to trust the project's agents, folded until someone opens it. */
function Setup({ cloud, children }: { cloud: string; children: ReactNode }) {
  const [open, setOpen] = useState(false);
  const id = useId();
  return (
    <div>
      <button
        type="button"
        aria-expanded={open}
        aria-controls={id}
        onClick={() => setOpen((o) => !o)}
        className="-ml-1 inline-flex items-center gap-1 rounded-md py-1 pr-2 pl-1 text-[13px] text-ink-2 transition-colors hover:bg-hover hover:text-ink"
      >
        <ChevronRight className={cn("size-4 transition-transform duration-150", open && "rotate-90")} />
        What {cloud} needs to trust them
      </button>
      {open && (
        <div id={id} className="mt-2 flex animate-rise flex-col gap-3">
          {children}
        </div>
      )}
    </div>
  );
}

/** Commands or a document to copy, as they are. */
function Commands({ text, label }: { text: string; label?: string }) {
  return (
    <div className="flex flex-col gap-1">
      {label && <span className="font-mono text-[11.5px] text-ink-4">{label}</span>}
      <div className="relative rounded-lg border border-line bg-sunken/70">
        <pre className="overflow-x-auto py-2.5 pr-24 pl-3.5 font-mono text-[11.5px] leading-[1.7] text-ink-2">
          {text.trimEnd()}
        </pre>
        <CopyButton text={text} className="absolute top-1.5 right-1.5 bg-sunken" />
      </div>
    </div>
  );
}

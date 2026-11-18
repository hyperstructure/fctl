// Admin → Settings' admin tokens: what an instance's own programs provision people with, where its
// deployment turns them on (FCTL_ADMIN_TOKENS). Each acts as the admin who made it, on the admin
// API's people endpoints alone. A new one is made in a dialog that says what it can do, and shows
// the secret once. Where the instance doesn't take them, the section shows only the tokens left from
// when it did, to revoke.

import { useMutation, useQuery } from "@tanstack/react-query";
import { ArrowUpRight, KeyRound, Plus } from "lucide-react";
import { useState } from "react";
import { Banner, Button, CodeBox, Dialog, Field, Input, Section, Select, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { ago } from "@/lib/format";
import { siteUrl } from "@/lib/install";
import { keys, queryClient } from "@/lib/queries";
import { EXPIRIES } from "@/lib/tokens";

/** The guide to provisioning people with admin tokens. */
const GUIDE = () => `${siteUrl()}/docs/self-hosting/#provision-people-with-admin-tokens`;

/** What every admin token can do, and can't. */
export const ADMIN_TOKEN_DOES =
  "It acts as you, on people alone: it creates accounts, changes their names, addresses, pictures and Claude tokens, adds them to any workspace and project, and disables and deletes them. It can't change roles, the allowlist or the instance's settings.";

/** A request that creates an account with the token `secret`, on this instance. */
export function provisionExample(secret: string, origin: string = location.origin): string {
  return [
    `curl -X POST ${origin}/api/v1/admin/users \\`,
    `  -H "Authorization: Bearer ${secret}" -H "Content-Type: application/json" \\`,
    `  -d '{"email": "ada@example.com", "given_name": "Ada", "family_name": "Lovelace"}'`,
  ].join("\n");
}

export function AdminTokens() {
  const list = useQuery({ queryKey: keys.adminTokens, queryFn: api.adminTokens });
  const [open, setOpen] = useState(false);
  const revoke = useMutation({
    mutationFn: (t: G.AdminToken) => api.revokeAdminToken(t.id),
    onSuccess: (_, t) => {
      queryClient.setQueryData<G.AdminTokenList>(
        keys.adminTokens,
        (l) => l && { ...l, tokens: l.tokens.filter((x) => x.id !== t.id) },
      );
      toast(`Revoked ${t.name}: it stopped working`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const enabled = list.data?.enabled ?? false;
  const tokens = list.data?.tokens ?? [];
  // An instance that doesn't take them shows only those left from when it did.
  if (!enabled && tokens.length === 0) return null;
  return (
    <Section
      title="Admin tokens"
      description="For your own programs that provision people, such as your identity provider's or HR system's: each works as the admin who made it until it expires or someone revokes it."
      actions={
        enabled && (
          <Button size="sm" icon={<Plus className="size-3.5" />} onClick={() => setOpen(true)}>
            New admin token
          </Button>
        )
      }
    >
      {!enabled && (
        <Banner tone="warn" className="mb-4">
          This instance doesn't take admin tokens any more, so these don't work. Its deployment turns them on
          with FCTL_ADMIN_TOKENS=1.
        </Banner>
      )}
      {tokens.length ? (
        <ul className="divide-y divide-line border-y border-line">
          {tokens.map((t) => (
            <li key={t.id} className="flex items-start gap-3 py-3">
              <KeyRound className="mt-0.5 size-4 shrink-0 text-ink-4" />
              <div className="min-w-0 flex-1">
                <div className="text-[13px] text-ink [overflow-wrap:anywhere]">{t.name}</div>
                <div className="mt-0.5 text-[11.5px] text-ink-4">
                  {`Made by ${t.created_by.name} ${ago(t.created_at)} · `}
                  {t.last_used_at ? `used ${ago(t.last_used_at)}` : "not used yet"}
                  {t.expires_at ? ` · expires ${ago(t.expires_at)}` : " · never expires"}
                </div>
              </div>
              <Button
                size="sm"
                variant="ghost"
                aria-label={`Revoke ${t.name}`}
                loading={revoke.isPending && revoke.variables?.id === t.id}
                onClick={() => revoke.mutate(t)}
              >
                Revoke
              </Button>
            </li>
          ))}
        </ul>
      ) : (
        <p className="max-w-xl text-[13px] leading-relaxed text-ink-3">
          None yet. A token for your identity provider creates accounts for the people who join, adds them to
          their team's workspaces and projects, and deletes them when they leave.{" "}
          <a href={GUIDE()} target="_blank" rel="noreferrer" className="text-accent hover:underline">
            How admin tokens provision people
          </a>
        </p>
      )}
      <NewAdminTokenDialog open={open} onClose={() => setOpen(false)} />
    </Section>
  );
}

/** Makes an admin token: its name and how long it works, then its secret, once. */
function NewAdminTokenDialog({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [name, setName] = useState("");
  const [expires, setExpires] = useState("365");
  const [created, setCreated] = useState<G.CreatedAdminToken | null>(null);
  const days = EXPIRIES.find((e) => e.value === expires)?.days;
  const create = useMutation({
    mutationFn: () => api.createAdminToken({ name: name.trim(), expires_in_days: days }),
    onSuccess: (t) => {
      setCreated(t);
      void queryClient.invalidateQueries({ queryKey: keys.adminTokens });
    },
  });
  const close = () => {
    onClose();
    setName("");
    setExpires("365");
    setCreated(null);
    create.reset();
  };
  const field = (create.error as { field?: string } | null)?.field;
  const err = (f: string) => (field === f ? create.error?.message : undefined);
  return (
    <Dialog
      open={open}
      onClose={close}
      wide
      title={created ? `${created.token.name} is ready` : "New admin token"}
      description={created ? undefined : "For a program that provisions people on this instance."}
    >
      {created ? (
        <div className="flex flex-col gap-4">
          <div>
            <Banner tone="warn">Copy it now: it is shown once.</Banner>
            <CodeBox value={created.secret} className="mt-2" />
          </div>
          <div>
            <p className="text-[13px] leading-relaxed text-ink-3">
              Send it in an{" "}
              <code className="rounded bg-raised px-1 font-mono text-[11.5px]">Authorization: Bearer</code>{" "}
              header, or use it as{" "}
              <code className="rounded bg-raised px-1 font-mono text-[11.5px]">FCTL_TOKEN</code> with{" "}
              <code className="rounded bg-raised px-1 font-mono text-[11.5px]">fctl admin users</code>. This
              creates an account:
            </p>
            <CodeBox value={provisionExample(created.secret)} className="mt-2" />
          </div>
          <div className="flex flex-wrap items-center justify-between gap-3">
            <a
              href={GUIDE()}
              target="_blank"
              rel="noreferrer"
              className="inline-flex items-center gap-1 text-[13px] text-accent hover:underline"
            >
              How admin tokens provision people <ArrowUpRight className="size-3.5" />
            </a>
            <Button variant="primary" onClick={close}>
              Done
            </Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-6"
          onSubmit={(e) => {
            e.preventDefault();
            if (name.trim()) create.mutate();
          }}
        >
          <div className="grid items-start gap-4 sm:grid-cols-[1fr_12rem]">
            <Field label="Name" hint="What it's for, so that you know it in the list." error={err("name")}>
              {(id) => (
                <Input
                  id={id}
                  value={name}
                  onChange={(e) => setName(e.target.value)}
                  placeholder="Okta provisioning"
                  autoFocus
                />
              )}
            </Field>
            <Field label="Expires" error={err("expires_in_days")}>
              {(id) => (
                <Select
                  id={id}
                  value={expires}
                  onChange={setExpires}
                  options={EXPIRIES.map((e) => ({ value: e.value, label: e.label }))}
                />
              )}
            </Field>
          </div>
          <div
            aria-live="polite"
            className="rounded-lg border border-line bg-sunken/50 px-4 py-3 text-[13px] leading-relaxed text-ink-2"
          >
            {ADMIN_TOKEN_DOES}{" "}
            {days
              ? `It expires on ${new Date(Date.now() + days * 86_400_000).toLocaleDateString(undefined, { dateStyle: "long" })}, or once you stop being an admin.`
              : "It never expires: it works until someone revokes it, or you stop being an admin."}
          </div>
          {create.error && !field && <Banner tone="error">{create.error.message}</Banner>}
          <div className="flex justify-end gap-2">
            <Button type="submit" variant="primary" loading={create.isPending} disabled={!name.trim()}>
              Create admin token
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

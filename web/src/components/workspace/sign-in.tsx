// A workspace's own sign-in requirement (G.Workspace.sign_in, ADR 0103): its members who are in it as an address at
// some domains open it only once they've signed in with Google as such an address. Its admins set it in the Access
// section, which says whom it covers before it saves; the workspace page says why it's closed, and how to open it.

import { useMutation } from "@tanstack/react-query";
import { Building2 } from "lucide-react";
import { useState } from "react";
import { ProviderMark } from "@/components/sign-in/provider";
import { Banner, Button, Field, Input, Switch, toast } from "@/components/ui";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { useInstance } from "@/lib/queries";
import { domainOf, domainsWords, GOOGLE, parseDomains, people, providers, startHref } from "@/lib/sign-in";

/** Which members a requirement at `domains` covers: those in the workspace as an address at one of them. */
export function covered(
  members: readonly G.WorkspaceMember[],
  domains: readonly string[],
): G.WorkspaceMember[] {
  return members.filter((m) => {
    const d = domainOf(m.email);
    return !!d && domains.includes(d);
  });
}

/** What requiring Google at `domains` does to the workspace's members, in sentences. */
export function coverage(
  name: string,
  members: readonly G.WorkspaceMember[],
  domains: readonly string[],
): string[] {
  const them = covered(members, domains);
  const others = members.length - them.length;
  const lines = [
    them.length === 0
      ? `No member is in ${name} as an address at ${domainsWords(domains, "or")} yet: those who join as one open it once they've signed in with Google as that address.`
      : `${people(them.length)} ${them.length === 1 ? "is" : "are"} in ${name} as an address at ${domainsWords(domains, "or")}: ${them.length === 1 ? "it opens" : "it opens to them"} once they've signed in with Google as that address, their other workspaces as before.`,
  ];
  if (others > 0)
    lines.push(`${people(others)} in it as other addresses ${others === 1 ? "signs" : "sign"} in as before.`);
  return lines;
}

/** The setting, in a workspace's Access section (admins). */
export function WorkspaceSignIn({
  w,
  members,
  onSaved,
}: {
  w: G.Workspace;
  members?: readonly G.WorkspaceMember[];
  onSaved: () => void;
}) {
  const { data: instance } = useInstance();
  const google = providers(instance).find((p) => p.id === GOOGLE);
  const rule = w.sign_in;
  const [editing, setEditing] = useState(false);
  const [text, setText] = useState(rule?.domains.join(", ") ?? domainOf(w.email) ?? "");
  const [passkeys, setPasskeys] = useState(rule?.passkeys ?? false);
  const parsed = parseDomains(text);
  const save = useMutation({
    mutationFn: (sign_in: G.SignInRule) => workspaces.update(w.id, { sign_in }),
    onSuccess: (_, r) => {
      onSaved();
      setEditing(false);
      toast(
        r.domains.length
          ? `${w.name} requires Google at ${domainsWords(r.domains)}`
          : `${w.name}'s members sign in as the instance lets them`,
      );
    },
  });
  const ready = parsed.domains.length > 0 && parsed.invalid.length === 0;
  return (
    <div className="flex flex-col gap-3 text-[13.5px] text-ink-2">
      <div className="flex items-start justify-between gap-4">
        <span>
          Members at your domains sign in with Google
          <span className="block text-[12.5px] leading-relaxed text-ink-4">
            {rule
              ? `Members in ${w.name} as an address at ${domainsWords(rule.domains)} open it only once they've signed in with Google as that address${rule.passkeys ? ", or with a passkey" : ""}: under your Google Workspace's two-step verification and device checks, and not once it closes their account. Members in it as other addresses sign in as before.`
              : `Members in ${w.name} as an address at your domains open it only once they've signed in with Google as that address. Members in it as other addresses, contractors say, sign in as before.`}
          </span>
        </span>
        {rule && !editing && (
          <span className="inline-flex shrink-0 items-center gap-1 rounded-full bg-accent-soft px-1.5 py-px text-[11px] text-ink-2">
            <ProviderMark id={GOOGLE} className="size-3" />
            {domainsWords(rule.domains)}
          </span>
        )}
      </div>
      {!google ? (
        <p className="text-[12.5px] text-ink-4">
          This instance doesn't sign people in with Google: its admins set that up first.
        </p>
      ) : editing ? (
        <form
          className="flex flex-col gap-4 rounded-lg border border-line px-4 py-4"
          onSubmit={(e) => {
            e.preventDefault();
            if (ready) save.mutate({ domains: parsed.domains, provider: GOOGLE, passkeys });
          }}
        >
          <Field
            label="Domains"
            hint="Separate them with commas. A member at a subdomain needs its own: eu.acme.com isn't acme.com."
            error={
              parsed.invalid.length > 0
                ? `${parsed.invalid.join(", ")} isn't a domain: give one like acme.com`
                : undefined
            }
          >
            {(id) => (
              <Input
                id={id}
                autoFocus
                value={text}
                placeholder="acme.com, acme.dev"
                onChange={(e) => setText(e.target.value)}
                spellCheck={false}
                autoCapitalize="off"
              />
            )}
          </Field>
          <div className="flex items-center justify-between gap-4">
            <span>
              Passkeys open it too
              <span className="block text-[12.5px] leading-relaxed text-ink-4">
                {passkeys
                  ? "Quick, but past Google's checks: a passkey still opens it to someone whose Google account you closed."
                  : "Off: a sign-in with a passkey alone doesn't open it."}
              </span>
            </span>
            <Switch checked={passkeys} onChange={setPasskeys} label="Passkeys open it too" />
          </div>
          {ready && members && (
            <ul
              className="flex flex-col gap-1.5 rounded-lg border border-line bg-sunken/40 px-4 py-3 text-[13px] leading-relaxed text-ink-2"
              aria-live="polite"
            >
              {coverage(w.name, members, parsed.domains).map((l) => (
                <li key={l}>{l}</li>
              ))}
            </ul>
          )}
          {save.error && <Banner tone="error">{save.error.message}</Banner>}
          <div className="flex justify-end gap-2">
            <Button
              type="button"
              variant="ghost"
              onClick={() => {
                setEditing(false);
                save.reset();
              }}
            >
              Cancel
            </Button>
            <Button type="submit" variant="primary" loading={save.isPending} disabled={!ready}>
              {rule ? "Save" : "Require Google"}
            </Button>
          </div>
        </form>
      ) : (
        <div className="flex flex-wrap gap-2">
          <Button size="sm" variant={rule ? "ghost" : "secondary"} onClick={() => setEditing(true)}>
            <Building2 className="size-3.5" />
            {rule ? "Change domains" : "Require Google"}
          </Button>
          {rule && (
            <Button
              size="sm"
              variant="ghost"
              loading={save.isPending}
              onClick={() => save.mutate({ domains: [], provider: GOOGLE, passkeys: false })}
            >
              Stop requiring it
            </Button>
          )}
          {save.error && <Banner tone="error">{save.error.message}</Banner>}
        </div>
      )}
    </div>
  );
}

/** Why the workspace is closed to its member, the caller, by its own requirement, and the way in. */
export function SignInBlocked({ w }: { w: G.Workspace }) {
  const { data: instance } = useInstance();
  const google = providers(instance).find((p) => p.id === GOOGLE);
  const domains = w.sign_in?.domains ?? [];
  return (
    <Banner tone="warn" className="self-stretch">
      {w.name} requires its members at {domainsWords(domains, "and")} to sign in with Google as that address,
      and you signed in another way. Until you do, its projects and sessions are closed to you; your other
      workspaces aren't.{" "}
      {google && (
        <a
          href={startHref(google, { next: `/w/${w.slug}`, hint: w.email })}
          className="font-medium underline underline-offset-2"
        >
          Sign in with Google as {w.email}
        </a>
      )}
    </Banner>
  );
}

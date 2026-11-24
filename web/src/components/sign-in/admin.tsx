// Admin → Sign-in: how people sign in to the instance, its providers, the domains whose people sign in with one only
// (InstanceSettings.sign_in), and whether signing in makes accounts. Requiring a provider shows whom it concerns
// before it saves (POST /api/v1/admin/sign-in/preview), and never signs out the admin who saves it.

import { keepPreviousData, useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Building2, Crown, Fingerprint, KeyRound, RectangleEllipsis, UserRoundPlus } from "lucide-react";
import { type ReactNode, useMemo, useState } from "react";
import {
  AvatarStack,
  Banner,
  Button,
  CodeBox,
  Dialog,
  Field,
  Input,
  Section,
  Select,
  Skeleton,
  Switch,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys, queryClient, useInstance, useMe } from "@/lib/queries";
import { useSettled } from "@/lib/search";
import {
  providers as buttons,
  consequences,
  domainOf,
  domainsWords,
  GOOGLE,
  parseDomains,
  people,
  says,
  startHref,
  today,
  without,
  withRequired,
} from "@/lib/sign-in";
import { ProviderMark } from "./provider";

/** Where an admin comes back after signing in with a provider to check that it works. */
const BACK = "/admin?tab=sign-in";

/** fctl.sh's guide to setting up Google sign-in. */
const GOOGLE_GUIDE = "https://fctl.sh/docs/self-hosting#google-sign-in-optional";

/** Where a provider's sign-in starts, as the sign-in page has it. */
function useStart(id: string): G.SignInProvider | undefined {
  const { data: instance } = useInstance();
  return buttons(instance).find((p) => p.id === id);
}

/** As people see a person: the preview's people are user refs to the avatars. */
const refOf = (p: G.SignInPerson): G.UserRef => ({ id: p.id, name: p.name, avatar_url: p.avatar_url });

/** A row's icon, in a tile, as Settings draws them. */
function Tile({ children, on }: { children: ReactNode; on?: boolean }) {
  return (
    <span
      className={cn(
        "flex size-8 shrink-0 items-center justify-center rounded-md border [&_svg]:size-4",
        on ? "border-accent/30 bg-accent-soft text-accent" : "border-line bg-raised text-ink-3",
      )}
    >
      {children}
    </span>
  );
}

/** Saves the instance's sign-in rules, then shows them as they are. */
function useSaveRules(onSaved: (rules: G.SignInRule[]) => void) {
  return useMutation({
    mutationFn: (rules: G.SignInRule[]) => api.updateInstanceSettings({ sign_in: rules }),
    onSuccess: (s) => {
      queryClient.setQueryData(keys.instanceSettings, s);
      void queryClient.invalidateQueries({ queryKey: keys.signIn });
      void queryClient.invalidateQueries({ queryKey: keys.instance });
      void queryClient.invalidateQueries({ queryKey: keys.me });
      onSaved(s.sign_in);
    },
  });
}

/** The tab: how people sign in today, the providers, the domains that require one, and accounts. */
export function SignInAdmin({
  save,
  saving,
}: {
  save: (b: G.UpdateInstanceSettingsRequest) => void;
  saving: boolean;
}) {
  const overview = useQuery({ queryKey: keys.signIn, queryFn: api.signIn });
  const o = overview.data;
  const [editing, setEditing] = useState<{ rule?: G.SignInRule } | null>(null);
  const [stopping, setStopping] = useState<G.SignInRule | null>(null);
  const configured = o?.providers.filter((p) => p.configured) ?? [];
  const name = (id: string) => o?.providers.find((p) => p.id === id)?.name ?? id;
  return (
    <>
      <Section
        title="How people sign in"
        description={
          o ? (
            <>
              {today(o)}{" "}
              {o.rules.length === 0
                ? "No domain requires a provider: each person chooses how they sign in."
                : `${domainsWords(o.rules.map((r) => `${says(r.domains, name(r.provider))} only`))}.`}
            </>
          ) : (
            "Who signs in here, and how."
          )
        }
      >
        {overview.isError ? (
          <Banner tone="error">{overview.error.message}</Banner>
        ) : !o ? (
          <Skeleton className="h-16 w-full" />
        ) : (
          <Ways o={o} />
        )}
      </Section>
      <Section
        title="Providers"
        description="The accounts people sign in with besides a password and passkeys. The instance's deployment sets them up."
      >
        {!o ? <Skeleton className="h-24 w-full" /> : <Providers o={o} />}
      </Section>
      <Section
        title="Domains that sign in with a provider"
        description="Your organization's people sign in with its provider alone, as their address there: under its two-step verification and device checks, and unable to sign in again once it closes their account. People at other domains sign in as before."
      >
        {!o ? (
          <Skeleton className="h-20 w-full" />
        ) : (
          <Domains
            o={o}
            canRequire={configured.length > 0}
            onRequire={() => setEditing({})}
            onEdit={(rule) => setEditing({ rule })}
            onStop={setStopping}
          />
        )}
      </Section>
      <Section title="Accounts" description="Who makes the accounts people sign in to.">
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Signing in makes accounts
            <span className="block text-[12.5px] leading-relaxed text-ink-4">
              {(o?.self_sign_up ?? true)
                ? "Signing in with Google makes an account for an address the allowlist admits (Access), its *@domain entries included."
                : "Only invitations, addresses allowed one by one and admin tokens make accounts: a *@domain entry admits the accounts at its domain, and makes none. The sign-in page tells people without one to ask an admin."}
            </span>
          </span>
          <Switch
            checked={o?.self_sign_up ?? true}
            disabled={!o || saving}
            onChange={(v) => save({ self_sign_up: v })}
            label="Signing in makes accounts"
          />
        </div>
      </Section>
      {o && editing && (
        <RuleDialog o={o} rule={editing.rule} providers={configured} onClose={() => setEditing(null)} />
      )}
      {o && stopping && <StopDialog o={o} rule={stopping} onClose={() => setStopping(null)} />}
    </>
  );
}

/** How many people sign in each way, as small counts under the lead. */
function Ways({ o }: { o: G.SignInOverview }) {
  const ways: { icon: ReactNode; n: number; label: string }[] = [
    { icon: <RectangleEllipsis />, n: o.passwords, label: "with a password" },
    ...o.providers
      .filter((p) => p.configured || p.people > 0)
      .map((p) => ({ icon: <ProviderMark id={p.id} />, n: p.people, label: `with ${p.name}` })),
    { icon: <Fingerprint />, n: o.passkeys, label: "with a passkey" },
    { icon: <UserRoundPlus />, n: o.no_way_in, label: "not signed in yet" },
  ];
  return (
    <ul className="grid grid-cols-2 gap-2.5 sm:grid-cols-4">
      {ways.map((w) => (
        <li
          key={w.label}
          className="flex items-center gap-2.5 rounded-lg border border-line bg-sunken/40 px-3 py-2.5 [&_svg]:size-4 [&_svg]:shrink-0 [&_svg]:text-ink-3"
        >
          {w.icon}
          <span className="min-w-0 text-[12.5px] leading-tight text-ink-3">
            <span className="block font-mono text-[15px] text-ink tabular-nums">{w.n}</span>
            {w.label}
          </span>
        </li>
      ))}
    </ul>
  );
}

/** What the admin's own sign-in says of a provider: whether it showed that it works for them. */
function YourSignIn({ o, provider }: { o: G.SignInOverview; provider: G.SignInProviderStatus }) {
  const { data: me } = useMe();
  const start = useStart(provider.id);
  if (o.method === provider.id)
    return (
      <>
        You signed in with {provider.name}
        {o.method_domain ? ` as an address at ${o.method_domain}` : ""}: it works for you.
      </>
    );
  const linked = me?.identities.some((i) => i.provider === provider.id);
  const how = o.method === "passkey" ? "a passkey" : "a password";
  if (!linked)
    return (
      <>
        You haven't linked {provider.name}:{" "}
        <Link to="/settings" search={{ tab: "security" }} className="text-ink-2 underline underline-offset-2">
          link it in Settings
        </Link>
        , then sign in with it to check that it works before you require it.
      </>
    );
  if (!start) return <>You signed in with {how}.</>;
  return (
    <>
      You signed in with {how}.{" "}
      <a href={startHref(start, { next: BACK })} className="text-ink-2 underline underline-offset-2">
        Sign in with {provider.name}
      </a>{" "}
      to check that it works for you before you require it.
    </>
  );
}

function Providers({ o }: { o: G.SignInOverview }) {
  return (
    <ul className="flex flex-col gap-3">
      {o.providers.map((p) => (
        <li key={p.id} className="flex flex-col gap-3 rounded-xl border border-line px-4 py-4">
          <div className="flex items-center gap-3">
            <Tile on={p.configured}>
              <ProviderMark id={p.id} />
            </Tile>
            <div className="min-w-0 flex-1">
              <div className="flex items-center gap-2 text-[14px] text-ink">
                {p.name}
                <span
                  className={cn(
                    "rounded-full px-1.5 py-px text-[11px]",
                    p.configured ? "bg-accent-soft text-ink-2" : "bg-raised text-ink-4",
                  )}
                >
                  {p.configured ? "Set up" : "Not set up"}
                </span>
              </div>
              <div className="text-[12px] text-ink-4">
                {p.configured
                  ? p.people > 0
                    ? `${people(p.people)} ${p.people === 1 ? "signs" : "sign"} in with it`
                    : "Nobody has signed in with it yet"
                  : `The deployment sets it up with ${p.setup}.`}
              </div>
            </div>
          </div>
          {p.configured ? (
            <div className="flex flex-col gap-2 pl-11">
              <div className="flex flex-col gap-1.5">
                <span className="text-[12px] text-ink-3">Redirect URI</span>
                <CodeBox value={p.redirect_url} />
                <span className="text-[12px] leading-relaxed text-ink-4">
                  Its OAuth client lists this as an authorized redirect URI.
                </span>
              </div>
              <p className="text-[12.5px] leading-relaxed text-ink-3">
                <YourSignIn o={o} provider={p} />
              </p>
            </div>
          ) : (
            <p className="pl-11 text-[12.5px] leading-relaxed text-ink-3">
              Make an OAuth client in Google Cloud, then give the deployment its client ID and secret:{" "}
              <a
                href={GOOGLE_GUIDE}
                target="_blank"
                rel="noreferrer"
                className="text-ink-2 underline underline-offset-2"
              >
                Google sign-in ↗
              </a>
              . Domains can require it once it's set up.
            </p>
          )}
        </li>
      ))}
    </ul>
  );
}

function Domains({
  o,
  canRequire,
  onRequire,
  onEdit,
  onStop,
}: {
  o: G.SignInOverview;
  canRequire: boolean;
  onRequire: () => void;
  onEdit: (rule: G.SignInRule) => void;
  onStop: (rule: G.SignInRule) => void;
}) {
  const provider = (id: string) => o.providers.find((p) => p.id === id);
  return (
    <div className="flex flex-col gap-4">
      {o.rules.length > 0 && (
        <ul className="flex flex-col divide-y divide-line rounded-xl border border-line">
          {o.rules.map((r, i) => {
            const p = provider(r.provider);
            const c = o.coverage[i];
            return (
              <li
                key={r.domains.join(",")}
                className="flex flex-col gap-2 px-4 py-4 sm:flex-row sm:items-center"
              >
                <div className="flex min-w-0 flex-1 items-start gap-3">
                  <Tile on>
                    <Building2 />
                  </Tile>
                  <div className="min-w-0 flex-1">
                    <div className="flex flex-wrap items-center gap-x-2 gap-y-1 text-[14px] text-ink">
                      <span className="[overflow-wrap:anywhere]">{domainsWords(r.domains)}</span>
                      <span className="inline-flex items-center gap-1 rounded-full bg-accent-soft px-1.5 py-px text-[11px] text-ink-2">
                        <ProviderMark id={r.provider} className="size-3" />
                        {p?.name ?? r.provider} only
                      </span>
                    </div>
                    <div className="mt-0.5 text-[12px] leading-relaxed text-ink-4">
                      {c &&
                        [
                          people(c.people),
                          `${c.linked} with ${p?.name ?? r.provider}`,
                          c.passwords > 0 &&
                            `${c.passwords} still ${c.passwords === 1 ? "has a password" : "have a password"}, which goes at their next sign-in`,
                          r.passkeys ? "passkeys sign them in too" : "passkeys don't sign them in alone",
                        ]
                          .filter(Boolean)
                          .join(" · ")}
                    </div>
                    {p && !p.configured && (
                      <p className="mt-1.5 text-[12.5px] text-amber">
                        {p.name} sign-in isn't set up any more: nobody at {domainsWords(r.domains)} can sign
                        in, the primary instance owner aside. The deployment sets {p.setup}.
                      </p>
                    )}
                  </div>
                </div>
                <div className="flex shrink-0 gap-1 self-end sm:self-auto">
                  <Button size="sm" variant="ghost" onClick={() => onEdit(r)}>
                    Edit
                  </Button>
                  <Button size="sm" variant="ghost" onClick={() => onStop(r)}>
                    Stop requiring
                  </Button>
                </div>
              </li>
            );
          })}
        </ul>
      )}
      {o.rules.length === 0 && (
        <p className="text-[13px] leading-relaxed text-ink-3">
          No domain requires one yet. Requiring Google at acme.com, say, moves everyone with an acme.com
          address off passwords: their next sign-in goes through Google, which links it to their account.
        </p>
      )}
      {o.rules.length > 0 && o.owner && (
        <p className="flex items-start gap-2 text-[12.5px] leading-relaxed text-ink-4">
          <Crown className="mt-[3px] size-3.5 shrink-0" />
          <span>
            {o.owner.name}, the primary instance owner, keeps their password and passkeys whatever these say:
            the instance's way in while a provider is down.
          </span>
        </p>
      )}
      <div className="flex flex-col items-start gap-2">
        <Button variant={o.rules.length ? "secondary" : "primary"} disabled={!canRequire} onClick={onRequire}>
          <KeyRound className="size-3.5" />
          {o.rules.length ? "Require a provider at more domains" : "Require a provider"}
        </Button>
        {!canRequire && (
          <p className="text-[12.5px] text-ink-4">Set up a provider first: domains require one that is.</p>
        )}
      </div>
    </div>
  );
}

/** What a change to the rules does, as it would happen: sentences, the people it concerns, and the admin's own. */
function Consequences({
  preview,
  provider,
  loading,
}: {
  preview?: G.SignInPreview;
  provider: string;
  loading: boolean;
}) {
  const { data: me } = useMe();
  if (!preview) return loading ? <Skeleton className="h-20 w-full" /> : null;
  const lines = consequences(preview, provider, me?.user.id);
  const affected = [
    ...preview.passwords,
    ...preview.passkeys.filter((k) => !preview.passwords.some((p) => p.id === k.id)),
  ];
  return (
    <div
      className={cn(
        "flex flex-col gap-2.5 rounded-lg border border-line bg-sunken/40 px-4 py-3.5 transition-opacity",
        loading && "opacity-60",
      )}
      aria-live="polite"
    >
      {affected.length > 0 && <AvatarStack users={affected.map(refOf)} max={8} size={22} />}
      <ul className="flex flex-col gap-1.5 text-[13px] leading-relaxed text-ink-2">
        {lines.map((l) => (
          <li key={l}>{l}</li>
        ))}
      </ul>
      {preview.email && (preview.password_count > 0 || preview.passkey_count > 0) && (
        <p className="text-[12.5px] text-ink-4">fctl emails each of them what changes.</p>
      )}
    </div>
  );
}

/** The change would sign out the admin who saves it: they sign in with the provider first. */
function SignsOut({ provider, domains }: { provider: G.SignInProviderStatus; domains: string[] }) {
  const start = useStart(provider.id);
  return (
    <Banner tone="warn">
      This would sign you out: you signed in another way. Sign in with {provider.name} as your{" "}
      {domainsWords(domains, "or")} address first, which shows that it works, then require it.{" "}
      {start && (
        <a
          href={startHref(start, { next: BACK })}
          className="font-medium text-ink underline underline-offset-2"
        >
          Sign in with {provider.name}
        </a>
      )}
    </Banner>
  );
}

/** Requires a provider at some domains, or changes a rule: whom it concerns shows as the admin types. */
function RuleDialog({
  o,
  rule,
  providers,
  onClose,
}: {
  o: G.SignInOverview;
  rule?: G.SignInRule;
  providers: G.SignInProviderStatus[];
  onClose: () => void;
}) {
  const { data: me } = useMe();
  const [text, setText] = useState(rule ? rule.domains.join(", ") : "");
  const [providerId, setProviderId] = useState(rule?.provider ?? providers[0]?.id ?? GOOGLE);
  const [passkeys, setPasskeys] = useState(rule?.passkeys ?? false);
  // An address of the admin's own is the likeliest domain: suggested while nothing is typed.
  const suggestion = me && !rule ? domainOf(me.user.email) : undefined;
  const parsed = parseDomains(useSettled(text, 250));
  const provider = providers.find((p) => p.id === providerId) ?? providers[0];
  const next = useMemo(() => {
    const base = rule ? without(o.rules, rule.domains) : o.rules;
    return parsed.domains.length ? withRequired(base, parsed.domains, providerId, passkeys) : base;
  }, [o.rules, rule, parsed.domains, providerId, passkeys]);
  const ready = parsed.domains.length > 0 && parsed.invalid.length === 0;
  const preview = useQuery({
    queryKey: [...keys.signIn, "preview", next],
    queryFn: () => api.signInPreview(next),
    enabled: ready,
    placeholderData: keepPreviousData,
    retry: false,
  });
  const save = useSaveRules(() => {
    toast(
      rule
        ? "Saved: people sign in as the rule now says"
        : `${says(parsed.domains, provider?.name ?? providerId)} only now: their other ways in stop working`,
    );
    onClose();
  });
  const signsOut = preview.data?.you === "signs_out";
  return (
    <Dialog
      open
      onClose={onClose}
      wide
      title={rule ? `Change how ${domainsWords(rule.domains)} signs in` : "Require a provider"}
      description="Everyone with an address at these domains signs in with the provider alone, as that address. Nothing changes until you save."
    >
      <form
        className="flex flex-col gap-5"
        onSubmit={(e) => {
          e.preventDefault();
          if (ready && !signsOut) save.mutate(next);
        }}
      >
        <Field
          label="Domains"
          hint={
            parsed.invalid.length > 0
              ? undefined
              : "Separate them with commas. An address at a subdomain needs its own: eu.acme.com isn't acme.com."
          }
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
              placeholder={suggestion ?? "acme.com, acme.dev"}
              onChange={(e) => setText(e.target.value)}
              spellCheck={false}
              autoCapitalize="off"
            />
          )}
        </Field>
        {suggestion && !text && (
          <button
            type="button"
            onClick={() => setText(suggestion)}
            className="-mt-3 self-start text-[12.5px] text-ink-3 underline-offset-2 hover:text-ink hover:underline"
          >
            Use {suggestion}, your own domain
          </button>
        )}
        {providers.length > 1 && (
          <Field label="Provider">
            {(id) => (
              <Select
                id={id}
                value={providerId}
                options={providers.map((p) => ({ value: p.id, label: p.name }))}
                onChange={setProviderId}
              />
            )}
          </Field>
        )}
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Passkeys sign them in too
            <span className="block text-[12.5px] leading-relaxed text-ink-4">
              {passkeys
                ? `Quick, but past ${provider?.name ?? "the provider"}'s checks: a passkey still signs in someone whose account ${provider?.name ?? "it"} closed, until an admin disables theirs here.`
                : `Off: passkeys confirm it's them after ${provider?.name ?? "the provider"}'s sign-in, and before sensitive actions, but don't sign them in alone.`}
            </span>
          </span>
          <Switch checked={passkeys} onChange={setPasskeys} label="Passkeys sign them in too" />
        </div>
        {ready && (
          <Consequences
            preview={preview.data}
            provider={provider?.name ?? providerId}
            loading={preview.isFetching}
          />
        )}
        {preview.isError && <Banner tone="error">{preview.error.message}</Banner>}
        {signsOut && provider && <SignsOut provider={provider} domains={parsed.domains} />}
        {save.error && <Banner tone="error">{save.error.message}</Banner>}
        <div className="flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            loading={save.isPending}
            disabled={!ready || signsOut || !preview.data}
          >
            {rule ? "Save" : `Require ${provider?.name ?? "it"}`}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

/** Stops requiring a provider at a rule's domains, after saying what that does. */
function StopDialog({ o, rule, onClose }: { o: G.SignInOverview; rule: G.SignInRule; onClose: () => void }) {
  const next = useMemo(() => without(o.rules, rule.domains), [o.rules, rule.domains]);
  const preview = useQuery({
    queryKey: [...keys.signIn, "preview", next],
    queryFn: () => api.signInPreview(next),
    retry: false,
  });
  const name = o.providers.find((p) => p.id === rule.provider)?.name ?? rule.provider;
  const save = useSaveRules(() => {
    toast(
      `${domainsWords(rule.domains)} ${rule.domains.length === 1 ? "signs" : "sign"} in as they like again`,
    );
    onClose();
  });
  const released = preview.data?.released;
  return (
    <Dialog
      open
      onClose={onClose}
      title={`Stop requiring ${name} at ${domainsWords(rule.domains)}?`}
      description={`Their people sign in as they like again: those who still have a password, with it. Passwords that went at their first sign-in with ${name} don't come back: each sets one in Settings, or with Forgot? on the sign-in page.`}
    >
      <div className="flex flex-col gap-5">
        {released !== undefined && released > 0 && (
          <p className="text-[13.5px] leading-relaxed text-ink-2">
            {people(released)} {released === 1 ? "signs" : "sign"} in as they like again.
          </p>
        )}
        {save.error && <Banner tone="error">{save.error.message}</Banner>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" loading={save.isPending} onClick={() => save.mutate(next)}>
            Stop requiring {name}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

// How people sign in to the instance: its providers, the domains whose people sign in with one only
// (InstanceSettings.sign_in), and the words the sign-in page, Settings and Admin → Sign-in say about them.
// The rules mirror crates/fctl-domain/src/sign_in.rs, and the sentences the CLI's (crates/fctl/src/cmd/sign_in.rs).

import type * as G from "@/gen";

/** Google's id, as rules, tokens and identities name it. */
export const GOOGLE = "google";

/** Google's button, for control planes from before `InstanceInfo.sign_in`. */
const GOOGLE_PROVIDER: G.SignInProvider = { id: GOOGLE, name: "Google", start: "/api/v1/auth/google/start" };

/** The providers the sign-in page shows buttons for. */
export function providers(instance?: G.InstanceInfo): G.SignInProvider[] {
  if (instance?.sign_in) return instance.sign_in.providers;
  return instance?.features.google ? [GOOGLE_PROVIDER] : [];
}

/** The domain of an address, lowercase; undefined until it has one. */
export function domainOf(address: string): string | undefined {
  const at = address.trim().lastIndexOf("@");
  if (at < 1) return undefined;
  const domain = address
    .trim()
    .slice(at + 1)
    .toLowerCase();
  return domain || undefined;
}

/** The rule that covers a typed address, and its provider, if one does and the instance has it set up. */
export function ruleFor(
  instance: G.InstanceInfo | undefined,
  address: string,
): { rule: G.SignInRule; provider?: G.SignInProvider } | undefined {
  const domain = domainOf(address);
  const info = instance?.sign_in;
  if (!domain || !info) return undefined;
  const rule = info.rules.find((r) => r.domains.includes(domain));
  if (!rule) return undefined;
  return { rule, provider: info.providers.find((p) => p.id === rule.provider) };
}

/** Where a provider's sign-in starts: to sign in and go on to `next`, or to link it from Settings. */
export function startHref(
  provider: G.SignInProvider,
  { intent = "login", next, hint }: { intent?: "login" | "link"; next?: string; hint?: string } = {},
): string {
  const q = new URLSearchParams({ intent });
  if (next && next !== "/") q.set("return_to", next);
  if (hint) q.set("login_hint", hint.trim());
  return `${provider.start}?${q}`;
}

/** "acme.com", "acme.com and acme.dev", "acme.com, acme.dev and acme.io". */
export function domainsWords(domains: readonly string[], last = "and"): string {
  if (domains.length <= 1) return domains[0] ?? "";
  return `${domains.slice(0, -1).join(", ")} ${last} ${domains[domains.length - 1]}`;
}

/** What a rule requires, as the server's refusals begin: "acme.com signs in with Google". */
export function says(domains: readonly string[], provider: string): string {
  return `${domainsWords(domains)} ${domains.length === 1 ? "signs" : "sign"} in with ${provider}`;
}

/** A domain as rules keep it, from what an admin typed (`ACME.com`, `@acme.com`, `*@acme.com`), or undefined. */
export function cleanDomain(typed: string): string | undefined {
  const d = typed.trim().toLowerCase().replace(/^\*?@/, "").replace(/\.$/, "");
  return /^[a-z0-9-]+(\.[a-z0-9-]+)+$/.test(d) && !d.startsWith("-") ? d : undefined;
}

/** The domains in what an admin typed, separated by commas, spaces or lines: each once, and the ones that aren't. */
export function parseDomains(text: string): { domains: string[]; invalid: string[] } {
  const domains: string[] = [];
  const invalid: string[] = [];
  for (const part of text.split(/[\s,;]+/).filter(Boolean)) {
    const d = cleanDomain(part);
    if (!d) invalid.push(part);
    else if (!domains.includes(d)) domains.push(d);
  }
  return { domains, invalid };
}

/**
 * `rules` with `domains` requiring `provider` (and passkeys alone too, where `passkeys`): each moves into the rule that
 * says the same of its people, or a new one, and rules left without domains go (fctl_domain::sign_in::with_required).
 */
export function withRequired(
  rules: readonly G.SignInRule[],
  domains: readonly string[],
  provider: string,
  passkeys: boolean,
): G.SignInRule[] {
  const next = without(rules, domains);
  const moved = domains.map((d) => cleanDomain(d) ?? d);
  const same = next.find((r) => r.provider === provider && r.passkeys === passkeys);
  if (same) {
    same.domains = [...new Set([...same.domains, ...moved])].sort();
    return next;
  }
  return [...next, { domains: [...new Set(moved)].sort(), provider, passkeys }];
}

/** `rules` without `domains`: their people sign in as they like again. */
export function without(rules: readonly G.SignInRule[], domains: readonly string[]): G.SignInRule[] {
  const gone = domains.map((d) => cleanDomain(d) ?? d);
  return rules
    .map((r) => ({ ...r, domains: r.domains.filter((d) => !gone.includes(d)) }))
    .filter((r) => r.domains.length > 0);
}

/** "1 person", "38 people". */
export function people(n: number): string {
  return n === 1 ? "1 person" : `${n} people`;
}

/** Up to `most` names, and how many more. */
export function names(list: readonly G.SignInPerson[], all: number, most = 3): string {
  const shown = list.slice(0, most).map((p) => p.name);
  const more = all - shown.length;
  if (more <= 0) return domainsWords(shown);
  return `${shown.join(", ")} and ${more} more`;
}

/** What a change to the rules does, sentence by sentence, as the CLI says it before it asks. */
export function consequences(p: G.SignInPreview, provider: string, you?: string): string[] {
  const lines: string[] = [];
  if (p.password_count > 0)
    lines.push(
      `${people(p.password_count)} ${p.password_count === 1 ? "signs in" : "sign in"} with a password today: their next sign-in goes through ${provider}, and their ${p.password_count === 1 ? "password stops" : "passwords stop"} working.`,
    );
  if (p.passkey_count > 0)
    lines.push(
      `${people(p.passkey_count)} ${p.passkey_count === 1 ? "has" : "have"} passkeys that stop signing them in alone; they still confirm it's them after ${provider}.`,
    );
  if (p.linked > 0)
    lines.push(`${people(p.linked)} ${p.linked === 1 ? "signs in" : "sign in"} with ${provider} already.`);
  if (p.first > 0)
    lines.push(
      `${people(p.first)} ${p.first === 1 ? "has" : "have"} no way in yet: their first sign-in with ${provider} links it.`,
    );
  if (p.released > 0)
    lines.push(
      `${people(p.released)} ${p.released === 1 ? "signs in" : "sign in"} as they like again; those who still have a password, with it.`,
    );
  if (p.covered > 0 && p.others > 0)
    lines.push(`${people(p.others)} at other domains ${p.others === 1 ? "signs in" : "sign in"} as before.`);
  if (p.you === "owner")
    lines.push(
      `You keep your password and passkeys, as the primary instance owner: the way in if ${provider} is down.`,
    );
  else if (p.owner && p.owner.id !== you)
    lines.push(
      `${p.owner.name}, the primary instance owner, keeps their password and passkeys: the way in if ${provider} is down.`,
    );
  if (lines.length === 0) lines.push("Nobody's way in changes.");
  return lines;
}

/** How the instance's people sign in today, in a sentence: Admin → Sign-in's lead. */
export function today(o: G.SignInOverview): string {
  if (o.active === 0) return "Nobody has an account yet.";
  const ways: string[] = [];
  if (o.passwords > 0) ways.push(`${o.passwords} with a password`);
  for (const p of o.providers) if (p.people > 0) ways.push(`${p.people} with ${p.name}`);
  if (o.passkeys > 0) ways.push(`${o.passkeys} with a passkey`);
  const all = `${people(o.active)} ${o.active === 1 ? "has" : "have"} an account`;
  const sentence = ways.length > 0 ? `${all}: ${domainsWords(ways)}.` : `${all}.`;
  return o.no_way_in > 0
    ? `${sentence} ${people(o.no_way_in)} ${o.no_way_in === 1 ? "hasn't" : "haven't"} signed in yet.`
    : sentence;
}

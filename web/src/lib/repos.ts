// A project's GitHub repositories in a few words: the one its agents start
// in, and how many more they may use.

import type * as G from "@/gen";

type Repos = Pick<G.Project, "repo" | "other_repos" | "all_repos_of">;

export interface RepoSummary {
  /** The primary repository ("hyperstructure/fctl"), or the account of an owner-wide grant without one. */
  name?: string;
  /** The others: "(+3 more)", "(+entire org)"; without a primary repository, "3 repositories". */
  more?: string;
}

/** What lists show of a project's repositories; undefined when it has none. */
export function repoSummary(p: Repos): RepoSummary | undefined {
  const n = p.other_repos ?? 0;
  if (p.repo) {
    const more = p.all_repos_of ? "(+entire org)" : n > 0 ? `(+${n} more)` : undefined;
    return { name: `${p.repo.owner}/${p.repo.name}`, more };
  }
  if (p.all_repos_of) return { name: p.all_repos_of, more: "(entire org)" };
  if (n > 0) return { more: `${n} ${n === 1 ? "repository" : "repositories"}` };
  return undefined;
}

/** The same in one line: "hyperstructure/fctl (+3 more)". */
export function repoLabel(p: Repos): string | undefined {
  const s = repoSummary(p);
  return s && [s.name, s.more].filter(Boolean).join(" ");
}

/** A repository someone means: `name` absent for every repository of `owner`. */
export interface RepoName {
  owner: string;
  name?: string;
}

/** "owner/name", or "owner/*" for every repository of the account. */
export const repoValue = (r: RepoName): string => `${r.owner}/${r.name ?? "*"}`;

/** GitHub's names ignore case. */
const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();

/** A part of a name as GitHub takes it (fctl_proto's RepoRef::parse). */
const part = (p: string) => p.length > 0 && p.length <= 100 && /^[\w.-]+$/.test(p) && p !== "." && p !== "..";

/**
 * A repository typed by hand, as the control plane reads it (fctl_domain::repos::parse):
 * owner/name, owner/* for every repository of the account, or an address on github.com, also of
 * something in the repository, such as a pull request.
 */
export function parseRepo(input: string): RepoName | undefined {
  let s = input.trim();
  const address = /^(?:https?:\/\/)?(?:www\.)?github\.com\/([^?#]*)/i.exec(s);
  if (address) s = (address[1] ?? "").split("/").slice(0, 2).join("/");
  const [owner, name, ...rest] = s.replace(/\/+$/, "").split("/");
  if (owner === undefined || name === undefined || rest.length > 0 || !part(owner)) return undefined;
  if (name === "*") return { owner };
  const bare = name.replace(/(\.git)+$/, "");
  return part(name) && bare ? { owner, name: bare } : undefined;
}

/** A repository to add to a project, or every repository of an account. */
export interface RepoChoice extends RepoName {
  /** `repoValue`, as GitHub spells it where fctl listed it. */
  value: string;
  private?: boolean;
  /** On the project already: as its primary repository, or added. */
  on?: "primary" | "added";
  /** Typed, rather than among the repositories the workspace may use. */
  typed?: boolean;
  /**
   * The account the project's repositories belong to, or the chosen ones do, when this one is of
   * another: it can't be chosen with them.
   */
  elsewhere?: string;
}

/** How many choices the list shows; typing finds the others. */
export const CHOICES_SHOWN = 50;

/**
 * What a project's maintainers may add to it, for the field that adds them, account by account:
 * every repository of the account, then its repositories the workspace may use, newest push first,
 * then those on the project already. Repositories chosen by hand lead their account's. All of a
 * project's repositories belong to one account: the account of those it has, else of those
 * chosen. What's typed narrows them down, closest first, and a repository typed whole that isn't
 * among them comes last.
 */
export function repoChoices({
  listed,
  available,
  accounts,
  chosen,
  query,
}: {
  /** The project's repositories, the primary one first. */
  listed: readonly G.ProjectRepo[];
  /** The repositories the workspace may use (GET /github/repos). */
  available: readonly G.GithubRepo[];
  /** The GitHub accounts the workspace works in. */
  accounts: readonly string[];
  /** The repositories chosen so far, as `repoValue`s. */
  chosen: readonly string[];
  query: string;
}): RepoChoice[] {
  const project = listed[0]?.owner;
  const owners: string[] = [];
  for (const o of project ? [project] : [...accounts, ...available.map((r) => r.owner)])
    if (!owners.some((x) => same(x, o))) owners.push(o);
  const groups = owners.map((owner) => {
    const on = listed.filter((l) => same(l.owner, owner));
    const has = (name?: string) =>
      on.some((l) => (name === undefined ? !l.name : !!l.name && same(l.name, name)));
    const group: RepoChoice[] = has() ? [] : [{ value: repoValue({ owner }), owner }];
    for (const r of available)
      if (same(r.owner, owner) && !has(r.name))
        group.push({ value: r.full_name, owner: r.owner, name: r.name, private: r.private });
    for (const l of on)
      group.push({
        value: repoValue(l),
        owner: l.owner,
        name: l.name,
        private: available.find((r) => same(r.owner, l.owner) && !!l.name && same(r.name, l.name))?.private,
        on: l.primary ? "primary" : "added",
      });
    return group;
  });
  const listedValues = groups.flat().map((c) => c.value);
  const byHand = (rs: (RepoName | undefined)[]): RepoChoice[] =>
    rs
      .filter((r): r is RepoName => !!r)
      .filter((r) => !listedValues.some((v) => same(v, repoValue(r))))
      .map((r) => ({ ...r, value: repoValue(r), typed: true }));
  for (const c of byHand(chosen.map(parseRepo)).reverse()) {
    const i = owners.findIndex((o) => same(o, c.owner));
    if (i >= 0) groups[i]?.unshift(c);
    else groups.unshift([c]);
  }
  const pool = groups.flat();
  const account = accountOf(listed, chosen);
  const mark = (c: RepoChoice): RepoChoice =>
    account && !same(c.owner, account) ? { ...c, elsewhere: account } : c;
  const q = query.trim().toLowerCase();
  if (!q) return pool.map(mark);
  // An address narrows the list down to its repository.
  const typed = parseRepo(query);
  const needle = typed ? repoValue(typed).toLowerCase() : q;
  const rank = (c: RepoChoice) => {
    const v = c.value.toLowerCase();
    const n = (c.name ?? "*").toLowerCase();
    if (v === needle || n === needle) return 0;
    if (v.startsWith(needle) || n.startsWith(needle)) return 1;
    return v.includes(needle) ? 2 : -1;
  };
  const found = pool
    .map((c) => ({ c, r: rank(c) }))
    .filter((x) => x.r >= 0)
    .sort((a, b) => a.r - b.r)
    .map((x) => x.c);
  // One typed whole comes last, so that typing on to a longer name doesn't choose it.
  const extra = byHand([typed]).filter((t) => !pool.some((c) => same(c.value, t.value)));
  return [...found, ...extra].map(mark);
}

/** The account a project's repositories belong to: of those it has, else of those chosen. */
function accountOf(listed: readonly G.ProjectRepo[], chosen: readonly string[]): string | undefined {
  return listed[0]?.owner ?? chosen.map(parseRepo).find((r) => !!r)?.owner;
}

/**
 * What the field adds to a project: the repositories chosen, and one typed whole besides, as
 * `repoChoices` would let it be chosen: not one the project has, which adding again would only
 * give the access chosen, nor one of another account than its repositories'.
 */
export function repoAdditions(
  chosen: readonly string[],
  query: string,
  listed: readonly G.ProjectRepo[],
): string[] {
  const typed = parseRepo(query);
  if (!typed) return [...chosen];
  const value = repoValue(typed);
  const known = [...chosen, ...listed.map(repoValue)].some((v) => same(v, value));
  const account = accountOf(listed, chosen);
  return known || (account && !same(account, typed.owner)) ? [...chosen] : [...chosen, value];
}

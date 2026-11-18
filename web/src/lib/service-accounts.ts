// Service accounts (docs/architecture.md#service-accounts): a workspace's credentials for programs of
// its own, which start sessions for the people who allow them. What the app says of them: whom one
// asks, how each stands, how its people answered, and how a program uses one.

import type * as G from "@/gen";

/** The header a service account names the person it acts for in. */
export const ON_BEHALF_OF = "Fctl-On-Behalf-Of";

/** A service account of one of your workspaces that asks you, and hasn't heard your answer. */
export interface Ask {
  workspace: G.Workspace;
  account: G.ServiceAccount;
}

/**
 * What service accounts ask you, across your workspaces: those you haven't answered, where the
 * instance asks people first. Oldest first, as they were made, workspace by workspace. A workspace
 * closed to you (its two-factor requirement) asks nothing until it opens.
 */
export function asksOf(workspaces: readonly G.Workspace[] | undefined): Ask[] {
  const out: Ask[] = [];
  for (const w of workspaces ?? []) {
    if (w.blocked) continue;
    for (const a of w.service_accounts ?? [])
      if (a.consent && !a.answer) out.push({ workspace: w, account: a });
  }
  return out;
}

/** How a service account stands for you, in a few words. */
export function answerWords(a: G.ServiceAccount): string {
  if (!a.consent) return "Starts agents for anyone here";
  if (a.answer === "allowed") return "Starts agents for you";
  if (a.answer === "declined") return "You didn't allow it";
  return "Asks to start agents for you";
}

/** How a service account's people answered: "4 of 12 allowed it · 1 didn't · 7 haven't answered". */
export function tallyWords(t: G.ServiceAccountTally, consent: boolean): string {
  const all = t.allowed + t.declined + t.unanswered;
  if (!consent) return `It acts for all ${all === 1 ? "1 person" : `${all} people`} here`;
  const parts = [`${t.allowed} of ${all} allowed it`];
  if (t.declined) parts.push(`${t.declined} didn't`);
  if (t.unanswered) parts.push(`${t.unanswered} ${t.unanswered === 1 ? "hasn't" : "haven't"} answered`);
  return parts.join(" · ");
}

/** How a service account stands, for its admins: stopped with its keeper, working, or unused yet. */
export type AccountState = "stopped" | "used" | "new";

export function accountState(a: G.ServiceAccount): AccountState {
  if (a.stopped) return "stopped";
  return a.last_used_at ? "used" : "new";
}

/** What a program does with a service account, in each language the app shows. */
export type UsageLanguage = "cli" | "curl" | "node";

export const USAGE_LANGUAGES: { value: UsageLanguage; label: string; lang: string }[] = [
  { value: "cli", label: "fctl", lang: "bash" },
  { value: "curl", label: "curl", lang: "bash" },
  { value: "node", label: "Node.js", lang: "javascript" },
];

/**
 * How a program starts a session for someone with a service account, on this instance (`origin`):
 * in project `project` (its slug), for `person` (an address), with its secret where it's known (just
 * made), else read from FCTL_TOKEN.
 */
export function usageSnippet(
  language: UsageLanguage,
  { origin, project, person, secret }: { origin: string; project: string; person: string; secret?: string },
): string {
  const token = secret ?? "fctl_svc_…";
  if (language === "cli")
    return [
      `export FCTL_SERVER=${origin}`,
      `export FCTL_TOKEN=${token}`,
      "",
      `# Starts a session for ${person}: theirs, on their Claude subscription.`,
      `fctl run -p ${project} --on-behalf-of ${person} -d "Fix the login redirect"`,
      "",
      "# What the agent does, the sessions it started, and the schedules it runs.",
      `fctl logs ${project}#12`,
      "fctl sessions --mine",
      `fctl schedules run "Nightly triage" -p ${project}`,
    ].join("\n");
  if (language === "curl")
    return [
      `curl -X POST ${origin}/api/v1/projects/${project}/sessions \\`,
      `  -H "Authorization: Bearer ${secret ? token : "$FCTL_TOKEN"}" \\`,
      `  -H "${ON_BEHALF_OF}: ${person}" \\`,
      `  -H "Content-Type: application/json" \\`,
      `  -d '{"prompt": "Fix the login redirect"}'`,
    ].join("\n");
  return [
    `const res = await fetch("${origin}/api/v1/projects/${project}/sessions", {`,
    `  method: "POST",`,
    `  headers: {`,
    `    Authorization: \`Bearer \${process.env.FCTL_TOKEN}\`,`,
    `    // The person signed in to your portal, by their address or fctl id.`,
    `    "${ON_BEHALF_OF}": user.email,`,
    `    "Content-Type": "application/json",`,
    `  },`,
    `  body: JSON.stringify({ prompt: "Fix the login redirect" }),`,
    `});`,
    `const body = await res.json();`,
    `// They haven't allowed it yet, or can't start sessions there: body.message says why.`,
    `if (body.field === "on_behalf_of") throw new Error(body.message);`,
    `const url = \`${origin}/s/\${body.id}\`;`,
  ].join("\n");
}

/** Whether a session is one a service account started, and its name, where it's still there. */
export function startedThrough(s: Pick<G.Session, "service_account">): string | undefined {
  if (!s.service_account) return undefined;
  return s.service_account.name ?? "a service account deleted since";
}

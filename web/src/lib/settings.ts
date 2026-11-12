// Settings' pages (routes/settings.tsx): which there are and how they group, the words that find
// each, which one an address opens, and what the list of them on a phone says about each.

import type * as G from "@/gen";
import { FEW_RECOVERY_CODES } from "./two-factor";
import { claudeName } from "./workspaces";

export const SETTINGS_TABS = [
  "profile",
  "emails",
  "workspaces",
  "claude",
  "linear",
  "notifications",
  "appearance",
  "log",
  "security",
  "tokens",
  "data",
] as const;

export type SettingsTab = (typeof SETTINGS_TABS)[number];

export interface SettingsPage {
  tab: SettingsTab;
  label: string;
  /** More words it is found by: in the command palette, and in an address like /settings/passkeys. */
  keywords: string;
}

/** The pages in their groups, as the navigation lists them. */
export const SETTINGS_GROUPS: readonly (readonly SettingsPage[])[] = [
  [
    { tab: "profile", label: "Profile", keywords: "name picture photo avatar gravatar nickname" },
    {
      tab: "emails",
      label: "Email addresses",
      keywords: "emails commit email co-author noreply primary address",
    },
    { tab: "workspaces", label: "Workspaces", keywords: "leave membership" },
    {
      tab: "claude",
      label: "Claude",
      keywords: "subscription connection setup-token login usage limits plan remote control",
    },
    { tab: "linear", label: "Linear", keywords: "link delegate issues tickets" },
  ],
  [
    { tab: "notifications", label: "Notifications", keywords: "push alerts follow browsers" },
    {
      tab: "appearance",
      label: "Appearance",
      keywords: "theme dark light clock 24-hour 12-hour",
    },
    {
      tab: "log",
      label: "App log",
      keywords: "diagnostics troubleshooting problem report bug black blank screen crash",
    },
  ],
  [
    {
      tab: "security",
      label: "Security",
      keywords:
        "password google sign-in methods passkeys two-factor 2fa authenticator recovery codes devices sign out",
    },
    { tab: "tokens", label: "Access tokens", keywords: "api pat ci bots scripts" },
    { tab: "data", label: "Your data", keywords: "export download delete account" },
  ],
];

export const SETTINGS_PAGES: readonly SettingsPage[] = SETTINGS_GROUPS.flat();

export const isSettingsTab = (v: unknown): v is SettingsTab => SETTINGS_TABS.some((t) => t === v);

/** The page a word names: its own (`tokens`), its label, or a word it is found by (`passkeys`). */
export function settingsPageFor(word: string): SettingsTab | undefined {
  const w = word.trim().toLowerCase();
  if (!w) return undefined;
  if (isSettingsTab(w)) return w;
  const slug = (s: string) => s.toLowerCase().replace(/\s+/g, "-");
  return (
    SETTINGS_PAGES.find((p) => slug(p.label) === w) ??
    SETTINGS_PAGES.find((p) => p.keywords.split(" ").includes(w))
  )?.tab;
}

/** What Settings' address says, besides its tab: where Google and Linear send people back to. */
export interface SettingsSearch {
  tab?: SettingsTab;
  /** Back from linking Google, which worked. */
  linked?: string;
  /** Back from linking Google, which didn't. */
  error?: string;
  /** Back from linking Linear. */
  linear?: string;
}

/**
 * The page an address opens: its tab; else the page that has what an older link named after `#`
 * (`/settings#claude`, from before Settings had pages) or what Google and Linear send people back
 * to. Undefined for Settings itself, which is the list of pages on a phone and Profile beside it.
 */
export function askedTab(search: SettingsSearch, hash: string): SettingsTab | undefined {
  if (search.tab) return search.tab;
  const named = hash.replace(/^#/, "");
  if (named === "connect-claude") return "claude";
  if (isSettingsTab(named)) return named;
  if (search.linear) return "linear";
  if (search.linked || search.error) return "security";
  return undefined;
}

/** What the list of pages says about one, under its name. */
export interface Gist {
  text: string;
  /** It needs the person: the list says it in amber, and the navigation beside the page marks it. */
  attention?: boolean;
}

export function emailsGist(me: Pick<G.Me, "emails" | "user">): Gist {
  const primary = me.emails.find((e) => e.primary)?.email ?? me.user.email;
  const more = me.emails.length - 1;
  return { text: more > 0 ? `${primary} and ${more} more` : primary };
}

export function workspacesGist(list: readonly unknown[] | undefined): Gist | undefined {
  if (!list) return undefined;
  if (list.length === 0) return { text: "None yet" };
  return { text: list.length === 1 ? "1 workspace" : `${list.length} workspaces` };
}

export function claudeGist(claude: G.ClaudeCredentialInfo[]): Gist {
  if (claude.length === 0) return { text: "Not connected yet", attention: true };
  const ended = claude.filter((c) => c.status === "invalid");
  if (ended.length === claude.length)
    return { text: ended.length === 1 ? "Needs reconnecting" : "They need reconnecting", attention: true };
  if (ended.length)
    return {
      text:
        ended.length === 1
          ? `${claudeName(ended[0] as G.ClaudeCredentialInfo)} needs reconnecting`
          : `${ended.length} need reconnecting`,
      attention: true,
    };
  const used = claude.find((c) => c.is_default) ?? claude[0];
  return {
    text: claude.length === 1 && used ? claudeName(used) : `${claude.length} connections`,
  };
}

export function linearGist(accounts: G.LinearAccount[] | undefined): Gist | undefined {
  if (!accounts) return undefined;
  const [first] = accounts;
  if (!first) return { text: "Not linked" };
  return {
    text:
      accounts.length === 1
        ? `Linked in ${first.organization_name}`
        : `Linked in ${accounts.length} Linear workspaces`,
  };
}

/**
 * The Security page: what needs the person first (a workspace closed to them until they set up
 * two-factor sign-in, recovery codes running out), else how they sign in.
 */
export function securityGist(me: Pick<G.Me, "two_factor" | "passkeys">, blocked = 0): Gist {
  const t = me.two_factor;
  // A workspace's page and the Agent Floor say which.
  if (blocked > 0) return { text: "Two-factor sign-in required", attention: true };
  if (t.authenticator_since !== undefined && t.recovery_codes <= FEW_RECOVERY_CODES)
    return {
      text: t.recovery_codes === 0 ? "No recovery codes left" : "Few recovery codes left",
      attention: true,
    };
  const passkeys = me.passkeys.length;
  return {
    text: [
      t.on ? "Two-factor sign-in on" : "Two-factor sign-in off",
      passkeys > 0 && (passkeys === 1 ? "1 passkey" : `${passkeys} passkeys`),
    ]
      .filter(Boolean)
      .join(" · "),
  };
}

export function tokensGist(tokens: Pick<G.TokenInfo, "kind">[] | undefined): Gist | undefined {
  if (!tokens) return undefined;
  const n = tokens.filter((t) => t.kind === "pat").length;
  return { text: n === 0 ? "None yet" : n === 1 ? "1 token" : `${n} tokens` };
}

// Settings: one page at a time, with the list of pages beside it where both fit, else the list
// alone (a phone), which opens one. Profile, email addresses, workspaces, Claude, Linear,
// notifications, appearance, the app log, security (sign-in methods, passkeys, two-factor sign-in,
// devices), access tokens and your data (the export, deleting the account). Which pages there
// are, and what the list says about each, is lib/settings.ts.

import { palette, themes } from "@hyperstructure/design/brand";
import { useMutation, useQuery } from "@tanstack/react-query";
import { Link, useLocation, useSearch } from "@tanstack/react-router";
import {
  Bell,
  BellOff,
  Check,
  ChevronLeft,
  ChevronRight,
  Download,
  ImageUp,
  KeyRound,
  Laptop,
  ListOrdered,
  LogOut,
  Mail,
  Monitor,
  Moon,
  Pencil,
  Plus,
  Radio,
  RectangleEllipsis,
  Shield,
  ShieldCheck,
  Sun,
  Terminal,
  Trash2,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { CONNECT_ID, ConnectClaude, EndedLogin, type Way } from "@/components/claude/connect";
import { OnRunnersDetails } from "@/components/claude/on-runners";
import { KeepTokenOut } from "@/components/claude/stand-in";
import { PasskeySettings } from "@/components/passkeys/settings";
import { PlanUsageList } from "@/components/plan-usage";
import { AppLog } from "@/components/settings/app-log";
import { SettingsIcon } from "@/components/settings/icon";
import { type NavEntry, type NavLinkProps, SettingsList, SettingsNav } from "@/components/settings/nav";
import { AccessTokens } from "@/components/tokens";
import { ConfirmWithCode } from "@/components/two-factor/confirm-code";
import { RecoveryCodes } from "@/components/two-factor/recovery-codes";
import { AuthenticatorSetup } from "@/components/two-factor/setup";
import {
  Avatar,
  Banner,
  Button,
  buttonClass,
  DangerZone,
  Dialog,
  Field,
  Input,
  LinearIcon,
  NameFields,
  nameParts,
  Section,
  SectionLayout,
  Select,
  Skeleton,
  Spinner,
  Switch,
  toast,
} from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { api, clickConfirmed, confirmFirst, emails, linearLinkUrl, workspaces as wsApi } from "@/lib/api";
import { forgetLog } from "@/lib/app-log";
import { avatarImage } from "@/lib/avatar-image";
import { tokenOnRunnersIn } from "@/lib/claude";
import { cn } from "@/lib/cn";
import { withAlpha } from "@/lib/color";
import { exported, exportHint, nextExport } from "@/lib/data-export";
import { forgetDraft } from "@/lib/draft";
import { ago, browserName } from "@/lib/format";
import { gravatarNews, gravatarPicture } from "@/lib/gravatar";
import { goTo } from "@/lib/history";
import { sampleTime, useHourCycle } from "@/lib/hour-cycle";
import { disablePush, enablePush, forgetPush, permission, pushSupport } from "@/lib/push";
import { fetchMe, keys, queryClient, useInstance, useMe, useWorkspaces } from "@/lib/queries";
import { forgetLeft } from "@/lib/reopen";
import {
  askedTab,
  claudeGist,
  emailsGist,
  type Gist,
  linearGist,
  SETTINGS_GROUPS,
  type SettingsSearch,
  type SettingsTab,
  securityGist,
  tokensGist,
  workspacesGist,
} from "@/lib/settings";
import { forgetSidebar } from "@/lib/sidebar";
import { useTheme } from "@/lib/theme";
import { useTickAt } from "@/lib/tick";
import { type LinearOutcome, linearOutcome } from "@/lib/tickets";
import { FEW_RECOVERY_CODES } from "@/lib/two-factor";
import { planOf } from "@/lib/usage";
import { claudeOptions, roleLabel, roomForAnother, workspaceCount } from "@/lib/workspaces";

const LINK_ERRORS: Record<string, string> = {
  google_linked_elsewhere: "That Google account is already linked to another user.",
  google_cancelled: "Linking was cancelled.",
  google_state: "Linking could not be verified. Try again.",
  google_expired: "Linking took too long. Try again.",
  google_reauth:
    "Linking Google takes a sign-in from the last 15 minutes. Sign out and in again, then link it.",
};

export function SettingsPage() {
  const { data: me } = useMe();
  const search = useSearch({ from: "/app/settings" });
  const hash = useLocation({ select: (l) => l.hash });
  const { data: instance } = useInstance();
  if (!me) return null;
  // Linear's page, where the instance has a Linear app.
  const has = (t: SettingsTab) => t !== "linear" || !!instance?.features.linear;
  const asked = askedTab(search, hash);
  const tab = asked && has(asked) ? asked : undefined;
  return (
    // Its own width (a container query) decides how it shows: the list of pages beside the page
    // open, Profile until one is chosen, where both fit; else the list, or the page it opened
    // with the way back to it.
    <div className="@container mx-auto max-w-[1000px] px-6 pt-12 pb-24">
      <h1
        className={cn(
          "font-display text-[34px] leading-tight font-medium tracking-[-0.02em]",
          tab && "@max-2xl:sr-only",
        )}
      >
        Settings
      </h1>
      {tab && (
        <Link
          to="/settings"
          className="-ml-1.5 inline-flex h-8 items-center gap-0.5 rounded-md pr-2 text-[14px] text-ink-3 transition-colors hover:text-ink @2xl:hidden"
        >
          <ChevronLeft className="size-[18px]" />
          Settings
        </Link>
      )}
      <div
        className={cn(
          "@2xl:mt-9 @2xl:grid @2xl:grid-cols-[180px_minmax(0,1fr)] @2xl:items-start @2xl:gap-10 @4xl:grid-cols-[200px_minmax(0,1fr)] @4xl:gap-14",
          tab ? "mt-4" : "mt-7",
        )}
      >
        <Pages me={me} tab={tab ?? "profile"} has={has} className="@max-2xl:hidden" />
        {!tab && <PageList me={me} has={has} className="@2xl:hidden" />}
        <div className={cn("min-w-0", !tab && "@max-2xl:hidden")}>
          <SectionLayout value="page">
            <SettingsBody key={tab ?? "profile"} tab={tab ?? "profile"} me={me} search={search} />
          </SectionLayout>
        </div>
      </div>
    </div>
  );
}

function SettingsBody({
  tab,
  me,
  search,
}: {
  tab: SettingsTab;
  me: G.Me;
  search: SettingsSearch & { linear?: LinearOutcome };
}) {
  switch (tab) {
    case "profile":
      return <Profile me={me} />;
    case "emails":
      return <Addresses me={me} />;
    case "workspaces":
      return <Workspaces me={me} />;
    case "claude":
      return <Claude me={me} />;
    case "linear":
      return <LinearAccounts returned={search.linear} />;
    case "notifications":
      return <Notifications me={me} />;
    case "appearance":
      return <Appearance />;
    case "log":
      return <AppLog />;
    case "security":
      return <Security me={me} search={search} />;
    case "tokens":
      return <AccessTokens />;
    case "data":
      return <YourData me={me} />;
  }
}

/**
 * What needs the person on each page, which the navigation marks: a Claude connection missing or
 * ended, a workspace closed to them until they set up two-factor sign-in, recovery codes running
 * out.
 */
function useAttention(me: G.Me): Partial<Record<SettingsTab, Gist>> {
  const blocked = useWorkspaces().data?.workspaces.filter((w) => w.blocked).length ?? 0;
  const claude = claudeGist(me.claude);
  const security = securityGist(me, blocked);
  return {
    claude: claude.attention ? claude : undefined,
    security: security.attention ? security : undefined,
  };
}

/** Settings' pages in their groups, as the lists show them: those this instance has. */
function entries(has: (t: SettingsTab) => boolean, but?: SettingsTab): NavEntry<SettingsTab>[][] {
  return SETTINGS_GROUPS.map((group) =>
    group
      .filter((p) => p.tab !== but && has(p.tab))
      .map((p) => ({
        key: p.tab,
        label: p.label,
        icon: (className: string) => <SettingsIcon tab={p.tab} className={className} />,
      })),
  );
}

/** A link to one of Settings' pages, as the lists draw it. */
const pageLink = (tab: SettingsTab, props: NavLinkProps) => (
  <Link to="/settings" search={{ tab }} {...props} />
);

/** The pages beside the one open: each with its icon, and a mark on what needs the person. */
function Pages({
  me,
  tab,
  has,
  className,
}: {
  me: G.Me;
  tab: SettingsTab;
  has: (t: SettingsTab) => boolean;
  className?: string;
}) {
  return (
    <SettingsNav
      label="Settings"
      groups={entries(has)}
      current={tab}
      attention={useAttention(me)}
      link={pageLink}
      className={className}
    />
  );
}

/**
 * Settings on a phone: who you are, then the pages in their groups, each with what it says now,
 * in amber where it needs you. Each opens on its own.
 */
function PageList({
  me,
  has,
  className,
}: {
  me: G.Me;
  has: (t: SettingsTab) => boolean;
  className?: string;
}) {
  return (
    <SettingsList
      label="Settings"
      head={
        <Link
          to="/settings"
          search={{ tab: "profile" }}
          className="flex items-center gap-3.5 rounded-xl border border-line bg-surface px-4 py-3.5 transition-colors hover:bg-hover active:bg-hover"
        >
          <Avatar user={me.user} size={44} />
          <span className="min-w-0 flex-1">
            <span className="block truncate text-[15.5px] font-medium text-ink">{me.user.name}</span>
            <span className="block truncate text-[12.5px] text-ink-3">Profile · {me.user.email}</span>
          </span>
          <ChevronRight className="size-4 shrink-0 text-ink-4" />
        </Link>
      }
      groups={entries(has, "profile")}
      gists={useGists(me, has)}
      link={pageLink}
      className={className}
    />
  );
}

/** What the list of pages says about each, as the instance and this browser have it now. */
function useGists(me: G.Me, has: (t: SettingsTab) => boolean): Partial<Record<SettingsTab, Gist>> {
  const workspaces = useWorkspaces().data?.workspaces;
  const linear = useQuery({ queryKey: keys.myLinear, queryFn: api.myLinear, enabled: has("linear") });
  const tokens = useQuery({ queryKey: keys.tokens, queryFn: api.tokens });
  const notifications = useQuery({ queryKey: keys.notifications, queryFn: api.notifications });
  const [theme] = useTheme();
  const [hours] = useHourCycle();
  const here = permission() === "granted" ? notifications.data?.devices.find((d) => d.current) : undefined;
  return {
    emails: emailsGist(me),
    workspaces: workspacesGist(workspaces),
    claude: claudeGist(me.claude),
    linear: linearGist(linear.data?.accounts),
    notifications: notifications.data && {
      text: here ? "On in this browser" : "Off in this browser",
    },
    appearance: {
      text: `${theme === "dark" ? "Dark" : "Light"} · ${hours}-hour clock`,
    },
    log: { text: "Kept in this browser, to copy into a report" },
    security: securityGist(me, workspaces?.filter((w) => w.blocked).length),
    tokens: tokensGist(tokens.data?.tokens),
    data: { text: "Export it, or delete your account" },
  };
}

function Profile({ me }: { me: G.Me }) {
  const [given, setGiven] = useState(() => nameParts(me.user).given);
  const [family, setFamily] = useState(() => nameParts(me.user).family);
  const [preferred, setPreferred] = useState(me.user.preferred_name ?? "");
  const file = useRef<HTMLInputElement>(null);
  const saved = (m: G.Me) => queryClient.setQueryData(keys.me, m);
  const save = useMutation({
    mutationFn: () => api.updateMe({ given_name: given, family_name: family, preferred_name: preferred }),
    onSuccess: (m) => {
      saved(m);
      toast("Profile saved");
    },
  });
  const upload = useMutation({
    mutationFn: async (f: File) => api.setAvatar(await avatarImage(f)),
    onSuccess: (m) => {
      saved(m);
      toast("New picture saved");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const removePicture = useMutation({
    mutationFn: () => api.removeAvatar(),
    onSuccess: saved,
    onError: (e) => toast(e.message, "error"),
  });
  const gravatar = useMutation({
    mutationFn: (on: boolean) => api.updateMe({ gravatar: on }),
    onSuccess: (m) => {
      // This says it, not the app's own notice.
      gravatarNews(gravatarPicture(m));
      saved(m);
      toast(gravatarSaid(m));
    },
    onError: (e) => toast(e.message, "error"),
  });
  const fromGravatar = me.avatar_source === "gravatar";
  const err = (f: string) =>
    (save.error as { field?: string } | null)?.field === f ? save.error?.message : undefined;
  return (
    <Section title="Profile" description="How you appear in sessions, in commits and to your team.">
      <div className="flex flex-col gap-5">
        {/* Beside the picture where the column has room, under it where it hasn't (from 768 px, beside the sidebar). */}
        <div className="@container">
          <div className="flex flex-col items-start gap-4 @2xs:flex-row @2xs:items-center">
            <Avatar user={me.user} size={64} />
            <div className="flex w-full min-w-0 flex-col gap-2.5 @2xs:flex-1">
              <div className="text-[13px] text-ink-3">
                <div className="truncate">{me.user.email}</div>
                <div className="text-[12px] text-ink-4">
                  {me.user.role === "owner"
                    ? "Primary instance owner · "
                    : me.user.role === "admin"
                      ? "An instance admin · "
                      : ""}
                  joined {ago(me.user.created_at)}
                </div>
                {fromGravatar && (
                  <div className="text-[12px] text-ink-4">
                    Your Gravatar, from{" "}
                    <a
                      href="https://gravatar.com"
                      target="_blank"
                      rel="noreferrer"
                      className="underline underline-offset-2 hover:text-ink"
                    >
                      gravatar.com
                    </a>
                  </div>
                )}
              </div>
              <div className="flex flex-wrap gap-2">
                <Button
                  size="sm"
                  icon={<ImageUp className="size-3.5" />}
                  loading={upload.isPending}
                  onClick={() => file.current?.click()}
                >
                  Upload a picture
                </Button>
                {me.user.avatar_url && (
                  <Button
                    size="sm"
                    variant="ghost"
                    loading={removePicture.isPending}
                    onClick={() => removePicture.mutate()}
                  >
                    Use your initials
                  </Button>
                )}
              </div>
              <input
                ref={file}
                type="file"
                accept="image/png,image/jpeg,image/gif,image/webp"
                className="hidden"
                onChange={(e) => {
                  const f = e.target.files?.[0];
                  if (f) upload.mutate(f);
                  e.target.value = "";
                }}
              />
            </div>
          </div>
        </div>
        {me.gravatar !== undefined && (
          <ul className="divide-y divide-line border-y border-line">
            <SwitchRow
              label="Show my Gravatar while I have no picture of my own"
              hint={gravatarHint(me)}
              checked={me.gravatar}
              disabled={gravatar.isPending}
              onChange={(v) => gravatar.mutate(v)}
            />
          </ul>
        )}
        <form
          className="flex flex-col gap-4"
          onSubmit={(e) => {
            e.preventDefault();
            save.mutate();
          }}
        >
          <NameFields given={given} family={family} onGiven={setGiven} onFamily={setFamily} error={err} />
          <div className="grid gap-4 sm:grid-cols-2">
            <Field
              label="What agents call you"
              hint="Left empty, agents use your first names. The Agent Floor greets you with it too."
              error={err("preferred_name")}
            >
              {(id) => (
                <Input
                  id={id}
                  autoComplete="nickname"
                  value={preferred}
                  onChange={(e) => setPreferred(e.target.value)}
                  placeholder={given.trim() || "Your first names"}
                />
              )}
            </Field>
          </div>
          <div>
            <Button type="submit" variant="primary" loading={save.isPending}>
              Save
            </Button>
          </div>
        </form>
      </div>
    </Section>
  );
}

/** What the Gravatar switch does for `me` now. */
function gravatarHint(me: G.Me): string {
  if (!me.gravatar) return "Off: fctl doesn't look it up on gravatar.com.";
  if (me.avatar_source === "gravatar")
    return `fctl looks it up on gravatar.com by ${me.user.email} once a day, and shows a new one the next day.`;
  if (me.user.avatar_url)
    return "Your own picture shows instead. Use your initials, then turn this on, for your Gravatar.";
  return `${me.user.email} has no Gravatar yet, so your initials show. fctl looks again once a day.`;
}

/** What turning the Gravatar switch on or off did. */
function gravatarSaid(me: G.Me): string {
  if (!me.gravatar) return "fctl doesn't show your Gravatar";
  if (me.avatar_source === "gravatar") return "Your picture is your Gravatar";
  if (me.user.avatar_url) return "Your Gravatar shows while you have no picture of your own";
  return `${me.user.email} has no Gravatar yet: fctl looks again once a day`;
}

/** Your addresses: each signs in, has its own commit email, and is how workspaces know you. */
function Addresses({ me }: { me: G.Me }) {
  const [disconnecting, setDisconnecting] = useState<G.EmailAddress | null>(null);
  const set = (m: G.Me) => {
    queryClient.setQueryData(keys.me, m);
    void queryClient.invalidateQueries({ queryKey: keys.workspaces });
  };
  const primary = useMutation({
    mutationFn: (email: string) => emails.update(email, { primary: true }),
    onSuccess: (m) => {
      set(m);
      toast(`${m.user.email} is your primary address`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const disconnect = useMutation({
    mutationFn: (email: string) => emails.remove(email),
    onSuccess: (m, email) => {
      set(m);
      setDisconnecting(null);
      toast(`Disconnected ${email}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const list = me.emails.length
    ? me.emails
    : [{ email: me.user.email, primary: true, commit_email: me.commit_email, workspaces: [], added_at: 0 }];
  return (
    <Section
      title="Email addresses"
      description="Each signs in to this account. A workspace knows you by the address you joined it as, and your commits in its projects credit that address's commit email."
    >
      <ul className="flex flex-col divide-y divide-line rounded-xl border border-line">
        {list.map((e) => (
          <li key={e.email} className="flex flex-col gap-3 px-4 py-4">
            <div className="flex items-center gap-3">
              <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-raised text-ink-3 shadow-[inset_0_0_0_1px_var(--c-line)]">
                <Mail className="size-4" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="flex items-center gap-2 text-[14px] text-ink">
                  <span className="truncate">{e.email}</span>
                  {e.primary && (
                    <span className="rounded-full bg-accent-soft px-1.5 py-px text-[11px] text-ink-2">
                      Primary
                    </span>
                  )}
                </div>
                <div className="mt-0.5 flex flex-wrap items-center gap-x-2.5 gap-y-1 text-[12px] text-ink-4">
                  {e.workspaces.length === 0 ? (
                    "No workspace knows you by it"
                  ) : (
                    <>
                      In
                      {e.workspaces.map((w) => (
                        <span key={w.id} className="inline-flex min-w-0 items-center gap-1 text-ink-3">
                          <WorkspaceAvatar workspace={w} size={13} />
                          <span className="truncate">{w.name}</span>
                        </span>
                      ))}
                    </>
                  )}
                </div>
              </div>
              {!e.primary && (
                <div className="flex shrink-0 gap-1">
                  <Button
                    size="sm"
                    variant="ghost"
                    loading={primary.isPending && primary.variables === e.email}
                    onClick={() => primary.mutate(e.email)}
                  >
                    Make primary
                  </Button>
                  <button
                    type="button"
                    aria-label={`Disconnect ${e.email}`}
                    className="rounded p-1.5 text-ink-4 hover:text-danger"
                    onClick={() => setDisconnecting(e)}
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
              )}
            </div>
            <CommitEmail address={e} onSaved={set} />
          </li>
        ))}
      </ul>
      <div className="mt-4 flex max-w-[64ch] flex-col gap-2 text-[12.5px] leading-relaxed text-ink-4">
        {/* Once for every address, rather than under each. */}
        <p>
          Commits credit you as a co-author with it, which anyone who can read the repository sees. Use one
          your GitHub account has, like your{" "}
          <a
            href="https://github.com/settings/emails"
            target="_blank"
            rel="noreferrer"
            className="text-ink-2 underline decoration-accent underline-offset-2 hover:text-ink"
          >
            noreply address
          </a>
          , so GitHub links the commits to you.
        </p>
        <p>
          To add another address, ask an instance admin to invite it. Open the invitation while you are signed
          in here, and connect it to this account.
        </p>
      </div>
      <Dialog
        open={!!disconnecting}
        onClose={() => setDisconnecting(null)}
        title={`Disconnect ${disconnecting?.email ?? ""}?`}
        description={
          disconnecting?.workspaces.length
            ? `It no longer signs you in, and you leave ${disconnecting.workspaces.map((w) => w.name).join(", ")}, which you joined as it. Your sessions there stop.`
            : "It no longer signs you in."
        }
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDisconnecting(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={disconnect.isPending}
            onClick={() => disconnecting && disconnect.mutate(disconnecting.email)}
          >
            Disconnect
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

/** The commit email of one address, saved on its own. */
function CommitEmail({ address, onSaved }: { address: G.EmailAddress; onSaved: (m: G.Me) => void }) {
  const [value, setValue] = useState(address.commit_email ?? "");
  useEffect(() => setValue(address.commit_email ?? ""), [address.commit_email]);
  const save = useMutation({
    mutationFn: () => emails.update(address.email, { commit_email: value.trim() }),
    onSuccess: (m) => {
      onSaved(m);
      toast(
        value.trim() ? `Commits credit you as ${value.trim()}` : "Those commits don't credit you any more",
      );
    },
  });
  const changed = value.trim() !== (address.commit_email ?? "");
  return (
    <form
      onSubmit={(e) => {
        e.preventDefault();
        if (changed) save.mutate();
      }}
    >
      <Field label="Commit email" error={save.error?.message}>
        {(id) => (
          <div className="flex gap-2">
            <Input
              id={id}
              type="email"
              value={value}
              onChange={(e) => setValue(e.target.value)}
              placeholder="None: commits don't credit you"
              autoComplete="off"
              spellCheck={false}
              className="min-w-0 flex-1"
            />
            <Button type="submit" size="field" disabled={!changed} loading={save.isPending}>
              Save
            </Button>
          </div>
        )}
      </Field>
    </form>
  );
}

/** The workspaces you are in: how each knows you, and what your sessions there run on. */
function Workspaces({ me }: { me: G.Me }) {
  const list = useWorkspaces();
  const [leaving, setLeaving] = useState<G.Workspace | null>(null);
  const leave = useMutation({
    mutationFn: (w: G.Workspace) => wsApi.removeMember(w.id, me.user.id),
    onSuccess: (_, w) => {
      setLeaving(null);
      for (const k of [keys.workspaces, keys.projects, ["sessions"], keys.runners, keys.me])
        void queryClient.invalidateQueries({ queryKey: k });
      toast(`You left ${w.name}`);
    },
    onError: (e) => toast(e.message, "error"),
  });
  const claude = useMutation({
    mutationFn: (v: { id: string; claude: string }) => wsApi.updateMembership(v.id, { claude: v.claude }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      toast("Saved: sessions take it when they next start or resume");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const data = list.data;
  return (
    <Section
      title="Workspaces"
      description="The ones you are in, each with its own people, projects, runners and GitHub accounts."
      actions={
        data && roomForAnother(data) ? (
          <Link to="/w/new">
            <Button size="sm" icon={<Plus className="size-3.5" />}>
              New workspace
            </Button>
          </Link>
        ) : undefined
      }
    >
      {!data ? (
        <Skeleton className="h-16" />
      ) : data.workspaces.length === 0 ? (
        <p className="text-[13.5px] text-ink-3">You aren't in a workspace yet.</p>
      ) : (
        // Its rows put their Claude connection beside the workspace where they have the room.
        <ul className="@container flex flex-col divide-y divide-line rounded-xl border border-line">
          {data.workspaces.map((w) => (
            <li key={w.id} className="flex flex-col gap-3 px-4 py-3.5 @xl:flex-row @xl:items-center">
              <Link
                to="/w/$slug"
                params={{ slug: w.slug }}
                className="flex min-w-0 flex-1 items-center gap-3 rounded-md"
              >
                <WorkspaceAvatar workspace={w} size={32} />
                <div className="min-w-0">
                  <div className="truncate text-[14px] text-ink">{w.name}</div>
                  <div className="truncate text-[12px] text-ink-4">
                    {roleLabel(w.role)} · as {w.email}
                    {w.blocked && <span className="text-amber"> · closed until you set up two-factor</span>}
                  </div>
                  {tokenOnRunnersIn(me, w) ? (
                    <div className="truncate text-[12px] text-ink-4">
                      Only on your runners that keep your Claude token
                    </div>
                  ) : (
                    w.own_runners && (
                      <div className="truncate text-[12px] text-ink-4">Only on your runners</div>
                    )
                  )}
                </div>
              </Link>
              <div className="flex items-center gap-2">
                <Select
                  size="sm"
                  aria-label={`Claude connection in ${w.name}`}
                  value={w.claude ?? "default"}
                  // Closed to you until you set up two-factor: so is choosing for it.
                  disabled={w.blocked}
                  onChange={(v) => claude.mutate({ id: w.id, claude: v === "default" ? "" : v })}
                  options={claudeOptions(me, w.claude, "Claude: ")}
                />
                {w.role !== "owner" && (
                  <Button
                    size="sm"
                    variant="ghost"
                    icon={<LogOut className="size-3.5" />}
                    onClick={() => setLeaving(w)}
                  >
                    Leave
                  </Button>
                )}
              </div>
            </li>
          ))}
        </ul>
      )}
      {data && <p className="mt-3 text-[12.5px] text-ink-4">{workspaceCount(data)}</p>}
      <Dialog
        open={!!leaving}
        onClose={() => setLeaving(null)}
        title={`Leave ${leaving?.name ?? ""}?`}
        description="You lose access to its projects and sessions at once. Your sessions there stop, and so do its sessions on your runners, which leave it. Its admins can invite you again."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setLeaving(null)}>
            Cancel
          </Button>
          <Button variant="danger" loading={leave.isPending} onClick={() => leaving && leave.mutate(leaving)}>
            Leave
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

function Claude({ me }: { me: G.Me }) {
  // Which way to connect is open: the recommended one until you have a connection.
  const [way, setWay] = useState<Way | null>(() => (me.claude.length === 0 ? "token" : null));
  const remove = useMutation({
    mutationFn: (id: string) => api.claudeRemove(id),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
  });
  const makeDefault = useMutation({
    mutationFn: (id: string) => api.claudeDefault(id),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
  });
  /** Opens a way to connect again, for a connection Claude ended, and brings it into view. */
  const reconnect = (w: Way) => {
    setWay(w);
    const reduce = matchMedia("(prefers-reduced-motion: reduce)").matches;
    requestAnimationFrame(() =>
      document
        .getElementById(CONNECT_ID)
        ?.scrollIntoView({ block: "start", behavior: reduce ? "auto" : "smooth" }),
    );
  };

  return (
    <Section
      id="claude"
      title="Claude"
      description={
        <>
          Sessions you start run on your own Claude subscription. fctl refuses API keys and stops a session
          before it would draw on paid extra usage. Removing a connection stops the sessions that run on it.
        </>
      }
    >
      <div className="flex flex-col gap-6">
        {me.claude.length > 0 && (
          <ul className="divide-y divide-line border-y border-line">
            {me.claude.map((c) => (
              <li key={c.id} className="py-3">
                <div className="flex items-center gap-3">
                  <span
                    className={cn(
                      "flex size-8 items-center justify-center rounded-md border",
                      (c.remote_control || c.on_runners) && c.status !== "invalid"
                        ? "border-accent/30 bg-accent-soft text-accent"
                        : "border-line bg-raised text-ink-3",
                    )}
                  >
                    {c.on_runners ? (
                      <ShieldCheck className="size-4" />
                    ) : c.remote_control ? (
                      <Radio className="size-4" />
                    ) : (
                      <KeyRound className="size-4" />
                    )}
                  </span>
                  <div className="min-w-0 flex-1">
                    <div className="truncate text-[13.5px] text-ink">
                      {c.account_email ??
                        c.label ??
                        (c.on_runners
                          ? "Token on your runners"
                          : c.kind === "oauth"
                            ? "Claude login"
                            : "Long-lived token")}
                      {c.is_default && (
                        <span className="ml-2 rounded-full bg-raised px-1.5 py-0.5 text-[10.5px] text-ink-3">
                          used for new sessions
                        </span>
                      )}
                    </div>
                    <div className="text-[12px] text-ink-4">
                      {c.on_runners
                        ? "Kept on your runners · never given to fctl · no Remote Control"
                        : c.kind === "oauth"
                          ? "Full login · Remote Control"
                          : "Inference only · no Remote Control"}
                      {planOf(c) && ` · ${planOf(c)}`}
                      {c.status === "invalid" && <span className="text-danger"> · needs reconnecting</span>}
                      {c.last_used_at && ` · used ${ago(c.last_used_at)}`}
                    </div>
                  </div>
                  {!c.is_default && (
                    <Button size="sm" variant="ghost" onClick={() => makeDefault.mutate(c.id)}>
                      Use by default
                    </Button>
                  )}
                  <button
                    type="button"
                    aria-label="Remove"
                    onClick={() => remove.mutate(c.id)}
                    className="rounded p-1.5 text-ink-4 hover:text-danger"
                  >
                    <Trash2 className="size-3.5" />
                  </button>
                </div>
                <div className="mt-3 flex flex-col gap-4 pl-11">
                  {c.status === "invalid" && c.kind === "oauth" && !c.on_runners && (
                    <EndedLogin onReconnect={reconnect} />
                  )}
                  {c.on_runners && <OnRunnersDetails />}
                  {c.usage?.length ? (
                    <PlanUsageList usage={c.usage} />
                  ) : (
                    <p className="text-[12px] text-ink-4">
                      Plan usage shows here once a session runs on this connection.
                    </p>
                  )}
                </div>
              </li>
            ))}
          </ul>
        )}
        <KeepTokenOut me={me} />
        <ConnectClaude me={me} way={way} onWay={setWay} />
      </div>
    </Section>
  );
}

function Notifications({ me }: { me: G.Me }) {
  const list = useQuery({ queryKey: keys.notifications, queryFn: api.notifications });
  const [support] = useState(pushSupport);
  const [allowed, setAllowed] = useState(permission);
  const n = list.data;
  const set = (next: G.Notifications) => queryClient.setQueryData(keys.notifications, next);
  const update = useMutation({
    mutationFn: (b: G.UpdateNotificationsRequest) => api.updateNotifications(b),
    onSuccess: set,
    onError: (e) => {
      toast(e.message, "error");
      void list.refetch();
    },
  });
  const change = (b: G.UpdateNotificationsRequest) => {
    if (n) set({ ...n, settings: { ...n.settings, ...b } });
    update.mutate(b);
  };
  const on = useMutation({
    mutationFn: () => enablePush(n?.public_key ?? "", me.user.id),
    onSuccess: () => {
      toast("Notifications are on in this browser");
      void list.refetch();
    },
    onSettled: () => setAllowed(permission()),
  });
  const off = useMutation({
    mutationFn: (id?: string) => disablePush(id),
    onSuccess: () => {
      toast("Notifications are off in this browser");
      void list.refetch();
    },
    onError: (e) => toast(e.message, "error"),
  });
  const remove = useMutation({
    mutationFn: (id: string) => api.removePushDevice(id),
    onSuccess: set,
    onError: (e) => toast(e.message, "error"),
  });
  const test = useMutation({
    mutationFn: api.testNotification,
    onSuccess: (r) =>
      r.failed
        ? toast(`Sent to ${r.sent} of ${r.sent + r.failed} browsers`, r.sent ? "info" : "error")
        : toast(r.sent === 1 ? "Sent. It should show in a moment." : `Sent to ${r.sent} browsers`),
    onError: (e) => toast(e.message, "error"),
  });
  const here = allowed === "granted" ? n?.devices.find((d) => d.current) : undefined;
  return (
    <Section
      title="Notifications"
      description="Sessions you follow notify you when they wait for your reply, pause on a usage limit or fail. You follow the sessions you start; follow others from their page."
    >
      {!n ? (
        <Skeleton className="h-28 w-full" />
      ) : (
        <div className="flex flex-col gap-6">
          {!n.public_key ? (
            <Banner tone="warn">
              This instance can't send notifications: it has no master key (FCTL_MASTER_KEY).
            </Banner>
          ) : support === "install" ? (
            <Banner tone="info">
              On iPhone and iPad, notifications come to fctl on the Home Screen. Tap Share, then Add to Home
              Screen, open fctl from there and turn them on.
            </Banner>
          ) : support === "unsupported" ? (
            <Banner tone="info">This browser can't show notifications.</Banner>
          ) : (
            <div>
              <div className="flex items-center gap-3 border-y border-line py-3">
                <span
                  className={cn(
                    "flex size-8 items-center justify-center rounded-md border",
                    here ? "border-accent/30 bg-accent-soft text-accent" : "border-line bg-raised text-ink-3",
                  )}
                >
                  {here ? <Bell className="size-4" /> : <BellOff className="size-4" />}
                </span>
                <div className="min-w-0 flex-1">
                  <div className="text-[13.5px] text-ink">
                    {here ? "On in this browser" : "Off in this browser"}
                  </div>
                  <div className="text-[12px] text-ink-4">
                    {here
                      ? (here.name ?? "This browser")
                      : allowed === "denied"
                        ? "Notifications are blocked for this site. Allow them in your browser's site settings."
                        : browserName(navigator.userAgent)}
                  </div>
                </div>
                {here ? (
                  <>
                    <Button size="sm" variant="ghost" onClick={() => test.mutate()} loading={test.isPending}>
                      Send a test
                    </Button>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => off.mutate(here.id)}
                      loading={off.isPending}
                    >
                      Turn off
                    </Button>
                  </>
                ) : (
                  <Button
                    size="sm"
                    variant="primary"
                    onClick={() => on.mutate()}
                    loading={on.isPending}
                    disabled={allowed === "denied"}
                  >
                    Turn on
                  </Button>
                )}
              </div>
              {on.error && (
                <Banner tone="error" className="mt-3">
                  {on.error.message}
                </Banner>
              )}
            </div>
          )}

          <div>
            <h3 className="text-[13.5px] font-medium text-ink">Notify me when a session I follow</h3>
            <ul className="mt-2 divide-y divide-line border-y border-line">
              <SwitchRow
                label="Waits for my reply"
                checked={n.settings.waiting}
                onChange={(v) => change({ waiting: v })}
              />
              <SwitchRow
                label="Pauses on a usage limit"
                hint="It continues on its own when the limit resets."
                checked={n.settings.paused}
                onChange={(v) => change({ paused: v })}
              />
              <SwitchRow label="Fails" checked={n.settings.failed} onChange={(v) => change({ failed: v })} />
            </ul>
          </div>
          <ul className="divide-y divide-line border-y border-line">
            <SwitchRow
              label="Follow the sessions I start or resume"
              hint="Off: you follow only the sessions you choose."
              checked={n.settings.follow_started}
              onChange={(v) => change({ follow_started: v })}
            />
          </ul>

          {n.devices.length > 0 && (
            <div>
              <h3 className="text-[13.5px] font-medium text-ink">Browsers that get them</h3>
              <ul className="mt-2 divide-y divide-line border-y border-line">
                {n.devices.map((d) => (
                  <li key={d.id} className="flex items-center gap-3 py-2.5">
                    <Bell className="size-4 text-ink-4" />
                    <div className="min-w-0 flex-1">
                      <div className="truncate text-[13px] text-ink">
                        {d.name ?? "Browser"}
                        {d.current && <span className="ml-2 text-[11.5px] text-accent">this browser</span>}
                      </div>
                      <div className="text-[11.5px] text-ink-4">
                        added {ago(d.created_at)}
                        {d.last_sent_at ? ` · last notified ${ago(d.last_sent_at)}` : " · nothing sent yet"}
                      </div>
                    </div>
                    <Button
                      size="sm"
                      variant="ghost"
                      onClick={() => (d.current ? off.mutate(d.id) : remove.mutate(d.id))}
                    >
                      Remove
                    </Button>
                  </li>
                ))}
              </ul>
            </div>
          )}
          <p className="text-[12.5px] leading-relaxed text-ink-3">
            Several that arrive close together come as one, and at most one a minute. Sessions you're looking
            at don't notify you.
          </p>
        </div>
      )}
    </Section>
  );
}

function SwitchRow({
  label,
  hint,
  checked,
  onChange,
  disabled,
}: {
  label: string;
  hint?: string;
  checked: boolean;
  onChange: (v: boolean) => void;
  disabled?: boolean;
}) {
  return (
    <li className="flex items-center justify-between gap-4 py-2.5">
      <div className="min-w-0">
        <div className="text-[13.5px] text-ink">{label}</div>
        {hint && <div className="text-[12px] text-ink-4">{hint}</div>}
      </div>
      <Switch checked={checked} onChange={onChange} label={label} disabled={disabled} />
    </li>
  );
}

/**
 * Who fctl knows you as in Linear workspaces: what you delegate to fctl there runs as you, and your
 * replies there reach its agents. Linking takes a Linear sign-in, which fctl forgets at once.
 */
function LinearAccounts({ returned }: { returned?: LinearOutcome }) {
  const { data: instance } = useInstance();
  const mine = useQuery({
    queryKey: keys.myLinear,
    queryFn: api.myLinear,
    enabled: !!instance?.features.linear,
  });
  const [dropping, setDropping] = useState<G.LinearAccount | null>(null);
  const unlink = useMutation({
    mutationFn: (organization: string) => api.unlinkLinear(organization),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.myLinear, m);
      void queryClient.invalidateQueries({ queryKey: ["project-tickets"] });
      setDropping(null);
      toast("Unlinked");
    },
    onError: (e) => toast(e.message, "error"),
  });
  if (!instance?.features.linear) return null;
  const accounts = mine.data?.accounts ?? [];
  const outcome = returned && linearOutcome(returned);
  return (
    <Section
      id="linear"
      title="Linear"
      description="Who fctl knows you as in Linear: issues you delegate to fctl there start sessions as you, on your Claude subscription, and your replies there reach its agents."
    >
      <div className="flex flex-col gap-4">
        {outcome && <Banner tone={outcome.tone}>{outcome.text}</Banner>}
        {mine.isPending ? (
          <Skeleton className="h-14" />
        ) : (
          <ul className="border-y border-line">
            {accounts.map((a) => (
              <li
                key={a.organization}
                className="flex items-center gap-3 border-b border-line py-3 last:border-b-0"
              >
                <Avatar user={{ id: a.linear_user, name: a.name, avatar_url: a.avatar_url }} size={28} />
                <div className="min-w-0 flex-1">
                  <div className="truncate text-[13.5px] text-ink">
                    {a.name}
                    <span className="text-ink-4"> in {a.organization_name}</span>
                  </div>
                  <div className="truncate text-[12px] text-ink-4">
                    {[a.email, a.url_key && `linear.app/${a.url_key}`].filter(Boolean).join(" · ")} · linked{" "}
                    {ago(a.linked_at)}
                  </div>
                </div>
                <Button size="sm" variant="ghost" onClick={() => setDropping(a)}>
                  Unlink
                </Button>
              </li>
            ))}
            {accounts.length === 0 && (
              <li className="flex items-center gap-3 py-3">
                <LinearIcon className="size-5 text-ink-3" />
                <span className="text-[13.5px] text-ink-3">
                  fctl doesn't know your Linear account yet, so what you delegate to fctl there starts
                  nothing.
                </span>
              </li>
            )}
          </ul>
        )}
        <div>
          <a
            href={linearLinkUrl()}
            className="inline-block"
            onClick={(e) =>
              clickConfirmed(e, linearLinkUrl())?.catch((err: Error) => toast(err.message, "error"))
            }
          >
            <Button icon={<LinearIcon className="size-3.5" />}>
              {accounts.length > 0 ? "Link another Linear account" : "Link your Linear account"}
            </Button>
          </a>
        </div>
      </div>
      <Dialog
        open={!!dropping}
        onClose={() => setDropping(null)}
        title={`Unlink your account in ${dropping?.organization_name ?? ""}?`}
        description="What you delegate to fctl there starts nothing, and your replies there don't reach its agents, until you link it again. Sessions already running carry on."
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDropping(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={unlink.isPending}
            onClick={() => dropping && unlink.mutate(dropping.organization)}
          >
            Unlink
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

/** Everything about signing in, and where you are signed in: a page of several sections. */
function Security({ me, search }: { me: G.Me; search: SettingsSearch }) {
  return (
    <Section title="Security" description="How you sign in, and everywhere you are signed in.">
      {/* Back from linking Google (api::google). */}
      {search.linked === "google" && (
        <Banner tone="ok" className="mb-7">
          Google is linked. You can sign in with it from now on.
        </Banner>
      )}
      {search.error && (
        <Banner tone="error" className="mb-7">
          {LINK_ERRORS[search.error] ?? "That did not work. Try again."}
        </Banner>
      )}
      <div className="border-t border-line pt-8">
        <SectionLayout value="part">
          <SignIn me={me} />
          <PasskeySettings me={me} />
          <TwoFactorSettings me={me} />
          <Devices />
        </SectionLayout>
      </div>
    </Section>
  );
}

/** A row's icon, in a tile: lit (`on`) where what it stands for is set up. */
function Tile({ on, children }: { on?: boolean; children: ReactNode }) {
  return (
    <span
      className={cn(
        "flex size-8 shrink-0 items-center justify-center rounded-md border transition-colors [&_svg]:size-4",
        on ? "border-accent/30 bg-accent-soft text-accent" : "border-line bg-raised text-ink-3",
      )}
    >
      {children}
    </span>
  );
}

function SignIn({ me }: { me: G.Me }) {
  const { data: instance } = useInstance();
  const [changing, setChanging] = useState(false);
  const unlink = useMutation({
    mutationFn: (id: string) => api.unlinkIdentity(id),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
    onError: (e) => toast(e.message, "error"),
  });
  const google = me.identities.find((i) => i.provider === "google");
  const password = me.user.has_password;
  return (
    <Section title="Sign-in methods" description="Keep at least one way in.">
      <ul className="divide-y divide-line border-y border-line">
        <li className="flex items-center gap-3 py-3">
          <Tile>
            <RectangleEllipsis />
          </Tile>
          <div className="min-w-0 flex-1">
            <div className="text-[13.5px] text-ink">Password</div>
            <div className="text-[12px] text-ink-4">
              {password
                ? "Signs you in with any of your email addresses"
                : "None yet: set one to sign in with your email address"}
            </div>
          </div>
          <Button size="sm" variant={password ? "ghost" : "secondary"} onClick={() => setChanging(true)}>
            {password ? "Change password" : "Set a password"}
          </Button>
        </li>
        <li className="flex items-center gap-3 py-3">
          <Tile>
            <svg viewBox="0 0 24 24" aria-hidden="true">
              <path
                fill="#4285F4"
                d="M23.5 12.3c0-.8-.1-1.6-.2-2.3H12v4.4h6.5a5.6 5.6 0 0 1-2.4 3.6v3h3.9c2.3-2.1 3.5-5.2 3.5-8.7z"
              />
              <path
                fill="#34A853"
                d="M12 24c3.2 0 6-1.1 8-2.9l-3.9-3c-1.1.7-2.5 1.2-4.1 1.2-3.1 0-5.8-2.1-6.7-5H1.3v3.1A12 12 0 0 0 12 24z"
              />
              <path fill="#FBBC05" d="M5.3 14.3a7.2 7.2 0 0 1 0-4.6V6.6H1.3a12 12 0 0 0 0 10.8l4-3.1z" />
              <path
                fill="#EA4335"
                d="M12 4.8c1.8 0 3.3.6 4.6 1.8l3.4-3.4A12 12 0 0 0 1.3 6.6l4 3.1c.9-2.9 3.6-4.9 6.7-4.9z"
              />
            </svg>
          </Tile>
          <div className="min-w-0 flex-1">
            <div className="text-[13.5px] text-ink">Google</div>
            <div className="truncate text-[12px] text-ink-4">
              {google
                ? `Linked as ${google.email}`
                : instance?.features.google
                  ? "Not linked"
                  : "Not available on this instance"}
            </div>
          </div>
          {google ? (
            <Button
              size="sm"
              variant="ghost"
              onClick={() => unlink.mutate(google.id)}
              loading={unlink.isPending}
            >
              Unlink
            </Button>
          ) : (
            instance?.features.google && (
              <a
                // Back at /settings?linked=google, or ?error=…: Settings opens this page for either.
                href="/api/v1/auth/google/start?intent=link&return_to=/settings"
                className={buttonClass({ size: "sm" })}
              >
                Link Google
              </a>
            )
          )}
        </li>
      </ul>
      <PasswordDialog me={me} open={changing} onClose={() => setChanging(false)} />
    </Section>
  );
}

/** Changing your password, or setting one: it signs out your other devices. */
function PasswordDialog({ me, open, onClose }: { me: G.Me; open: boolean; onClose: () => void }) {
  const [current, setCurrent] = useState("");
  const [next, setNext] = useState("");
  const had = me.user.has_password;
  const change = useMutation({
    mutationFn: () =>
      api.changePassword({
        current_password: had ? current : undefined,
        new_password: next,
      }),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: keys.me });
      toast(had ? "Password changed. Other sessions were signed out." : "Password set");
      onClose();
    },
  });
  useEffect(() => {
    if (!open) {
      setCurrent("");
      setNext("");
      change.reset();
    }
  }, [open, change.reset]);
  const err = (f: string) =>
    (change.error as { field?: string } | null)?.field === f ? change.error?.message : undefined;
  const other = change.error && !err("current_password") && !err("password") && !err("new_password");
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={had ? "Change your password" : "Set a password"}
      description={
        had
          ? "Your other devices are signed out, and sign in with the new one."
          : "Then you can also sign in with your email address and this password."
      }
    >
      <form
        className="flex flex-col gap-4"
        onSubmit={(e) => {
          e.preventDefault();
          change.mutate();
        }}
      >
        {/* For password managers: the account the password is for. */}
        <input type="email" autoComplete="username" value={me.user.email} readOnly hidden />
        {had && (
          <Field label="Current password" error={err("current_password")}>
            {(id) => (
              <Input
                id={id}
                type="password"
                autoComplete="current-password"
                autoFocus
                value={current}
                onChange={(e) => setCurrent(e.target.value)}
              />
            )}
          </Field>
        )}
        <Field
          label={had ? "New password" : "Password"}
          hint="At least 10 characters."
          error={err("password") ?? err("new_password")}
        >
          {(id) => (
            <Input
              id={id}
              type="password"
              autoComplete="new-password"
              autoFocus={!had}
              value={next}
              onChange={(e) => setNext(e.target.value)}
            />
          )}
        </Field>
        {other && <Banner tone="error">{change.error?.message}</Banner>}
        <div className="mt-1 flex justify-end gap-2">
          <Button type="button" variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button
            type="submit"
            variant="primary"
            loading={change.isPending}
            disabled={!next || (had && !current)}
          >
            {had ? "Change password" : "Set password"}
          </Button>
        </div>
      </form>
    </Dialog>
  );
}

type TwoFactorDialog = "setup" | "replace" | "off" | "codes";

function TwoFactorSettings({ me }: { me: G.Me }) {
  const { data: instance } = useInstance();
  const t = me.two_factor;
  const on = t.authenticator_since !== undefined;
  const few = on && t.recovery_codes <= FEW_RECOVERY_CODES;
  const [dialog, setDialog] = useState<TwoFactorDialog | null>(null);
  const [codes, setCodes] = useState<string[] | null>(null);
  // Replacing the app starts with a code from it.
  const [replacement, setReplacement] = useState<G.AuthenticatorSetup | null>(null);
  const close = () => {
    setDialog(null);
    setCodes(null);
    setReplacement(null);
  };
  const google = useMutation({
    mutationFn: (v: boolean) => api.updateTwoFactor({ google: v }),
    onSuccess: (m) => queryClient.setQueryData(keys.me, m),
    onError: (e) => toast(e.message, "error"),
  });
  const linkedGoogle = me.identities.some((i) => i.provider === "google");
  return (
    <Section
      title="Two-factor sign-in"
      description={
        <>
          A code from an app on your phone after your password, so that a password alone can't sign anyone in.
          {t.required && " This instance requires it, or a passkey."}
        </>
      }
    >
      <div className="flex flex-col gap-6">
        <ul className="divide-y divide-line border-y border-line">
          <li className="flex flex-wrap items-center gap-3 py-3">
            <span
              className={cn(
                "flex size-8 shrink-0 items-center justify-center rounded-md border transition-colors",
                on ? "border-accent/30 bg-accent-soft text-accent" : "border-line bg-raised text-ink-3",
              )}
            >
              {on ? <ShieldCheck className="size-4" /> : <Shield className="size-4" />}
            </span>
            <div className="min-w-0 flex-1">
              <div className="flex min-w-0 items-center gap-2 text-[13.5px] text-ink">
                {on ? <AuthenticatorName name={t.authenticator_name} /> : "Authenticator app"}
              </div>
              <div className="text-[12px] text-ink-4">
                {on
                  ? `Set up ${ago(t.authenticator_since ?? 0)} · ${
                      t.authenticator_used_at
                        ? `last used ${ago(t.authenticator_used_at)}`
                        : "not used to sign in yet"
                    }`
                  : t.on
                    ? "Not set up: your passkeys are your second factor"
                    : "Off: your password alone signs you in"}
              </div>
            </div>
            {on ? (
              <div className="flex items-center gap-1">
                <Button size="sm" variant="ghost" onClick={() => setDialog("replace")}>
                  Replace
                </Button>
                {(!t.required || me.passkeys.length > 0) && (
                  <Button size="sm" variant="ghost" onClick={() => setDialog("off")}>
                    Turn off
                  </Button>
                )}
              </div>
            ) : (
              <Button size="sm" variant="primary" onClick={() => setDialog("setup")}>
                Set up
              </Button>
            )}
          </li>
          {on && (
            <li className="flex flex-wrap items-center gap-3 py-3">
              <span className="flex size-8 shrink-0 items-center justify-center rounded-md border border-line bg-raised text-ink-3">
                <ListOrdered className="size-4" />
              </span>
              <div className="min-w-0 flex-1">
                <div className="text-[13.5px] text-ink">Recovery codes</div>
                <div className={cn("text-[12px]", few ? "text-amber" : "text-ink-4")}>
                  {t.recovery_codes === 0
                    ? "None left: get new ones, or a lost phone locks you out"
                    : `${t.recovery_codes} of 10 left${few ? ": get new ones before they run out" : ", each signs you in once"}`}
                </div>
              </div>
              <Button size="sm" variant={few ? "secondary" : "ghost"} onClick={() => setDialog("codes")}>
                Get new codes
              </Button>
            </li>
          )}
        </ul>
        {(instance?.features.google || linkedGoogle) && (
          <ul className="divide-y divide-line border-y border-line">
            <SwitchRow
              label="Ask for a code when I sign in with Google too"
              hint={
                t.google_required
                  ? "This instance asks everyone who has two-factor."
                  : !on && t.google
                    ? "From when you set up an authenticator app."
                    : "Off: Google's own sign-in is enough."
              }
              checked={t.google || t.google_required}
              disabled={t.google_required || google.isPending}
              onChange={(v) => google.mutate(v)}
            />
          </ul>
        )}
      </div>

      <Dialog
        open={dialog === "setup" || dialog === "replace"}
        onClose={close}
        wide
        title={dialog === "replace" ? "Replace your authenticator app" : "Set up an authenticator app"}
        description={
          dialog === "replace"
            ? replacement
              ? "Set up the new app or phone. Codes from the old one stop working; your recovery codes stay."
              : "First, confirm it's you with a code from your current app, or a recovery code if the phone is gone."
            : "Then signing in with your password also takes the code the app shows."
        }
      >
        {dialog === "replace" && !replacement ? (
          <ConfirmWithCode
            action="Continue"
            onCancel={close}
            submit={(code) => api.startAuthenticator({ code })}
            onDone={setReplacement}
          />
        ) : (
          <AuthenticatorSetup
            me={me}
            started={replacement ?? undefined}
            onRestart={dialog === "replace" ? () => setReplacement(null) : undefined}
            onConfirmed={(m) => queryClient.setQueryData(keys.me, m)}
            onCancel={close}
            onDone={(r) => {
              close();
              toast(
                r.codes
                  ? "Two-factor sign-in is on"
                  : dialog === "replace"
                    ? "The new authenticator app is in use. Your recovery codes still work."
                    : "Two-factor sign-in is on. Your recovery codes still work.",
              );
            }}
          />
        )}
      </Dialog>

      <Dialog
        open={dialog === "off"}
        onClose={close}
        title={t.required ? "Remove your authenticator app?" : "Turn off two-factor sign-in?"}
        description={
          t.required
            ? "Your passkeys stay your second factor, and your recovery codes stop working. Enter a code to confirm it's you."
            : "Signing in will take your password alone, and your recovery codes stop working. Enter a code to confirm it's you."
        }
      >
        {dialog === "off" && (
          <ConfirmWithCode
            action="Turn off"
            danger
            onCancel={close}
            submit={(code) => api.removeAuthenticator({ code })}
            onDone={(m) => {
              queryClient.setQueryData(keys.me, m);
              close();
              toast("Two-factor sign-in is off");
            }}
          />
        )}
      </Dialog>

      <Dialog
        open={dialog === "codes"}
        onClose={close}
        wide={!!codes}
        title={codes ? "Your new recovery codes" : "Get new recovery codes?"}
        description={
          codes
            ? "Each signs you in once if you lose your phone. Keep them somewhere safe: they are shown only now."
            : "Your current recovery codes stop working. Enter a code from your authenticator app, or a recovery code, to confirm."
        }
      >
        {codes ? (
          <div className="flex flex-col gap-5">
            <RecoveryCodes codes={codes} email={me.user.email} />
            <div className="flex justify-end">
              <Button variant="primary" onClick={close}>
                Done
              </Button>
            </div>
          </div>
        ) : (
          dialog === "codes" && (
            <ConfirmWithCode
              action="Get new codes"
              onCancel={close}
              submit={(code) => api.newRecoveryCodes({ code })}
              onDone={(r) => {
                queryClient.setQueryData(keys.me, r.me);
                setCodes(r.codes);
              }}
            />
          )
        )}
      </Dialog>
    </Section>
  );
}

/**
 * The authenticator app's name, which its person can change in place: nothing
 * about the app's secret says which app keeps it, 1Password or another.
 */
function AuthenticatorName({ name }: { name?: string }) {
  const shown = name ?? "Authenticator app";
  const [editing, setEditing] = useState(false);
  const [draft, setDraft] = useState(name ?? "");
  const input = useRef<HTMLInputElement>(null);
  const rename = useMutation({
    mutationFn: (n: string) => api.updateTwoFactor({ authenticator_name: n }),
    onSuccess: (m) => {
      queryClient.setQueryData(keys.me, m);
      setEditing(false);
    },
    onError: (e) => toast(e.message, "error"),
  });
  useEffect(() => {
    if (editing) input.current?.select();
  }, [editing]);
  const stop = () => {
    setDraft(name ?? "");
    setEditing(false);
  };
  // An empty name goes back to "Authenticator app".
  const save = () => {
    const next = draft.trim();
    if (next === (name ?? "")) stop();
    else rename.mutate(next);
  };
  return editing ? (
    <form
      className="min-w-0 flex-1"
      onSubmit={(e) => {
        e.preventDefault();
        save();
      }}
    >
      <Input
        ref={input}
        value={draft}
        placeholder="Authenticator app"
        onChange={(e) => setDraft(e.target.value)}
        onBlur={save}
        onKeyDown={(e) => {
          if (e.key === "Escape") {
            // The field's own, not the page's or a dialog's.
            e.preventDefault();
            stop();
          }
        }}
        maxLength={60}
        aria-label="The authenticator app's name"
        className="h-8 max-w-xs"
        disabled={rename.isPending}
      />
    </form>
  ) : (
    <button
      type="button"
      className="group flex min-w-0 items-center gap-2 text-left"
      onClick={() => {
        setDraft(name ?? "");
        setEditing(true);
      }}
      title="Rename it"
    >
      <span className="truncate">{shown}</span>
      <span className="shrink-0 rounded-full bg-accent-soft px-1.5 py-0.5 text-[10.5px] font-medium text-ink">
        on
      </span>
      {/* Touch screens have no hover to show it on. */}
      <Pencil className="size-3 shrink-0 text-ink-4 opacity-0 transition-opacity group-hover:opacity-100 group-focus-visible:opacity-100 pointer-coarse:opacity-100" />
    </button>
  );
}

/** Where you are signed in: browsers and the CLI. Access tokens have a section of their own. */
function Devices() {
  const list = useQuery({ queryKey: keys.tokens, queryFn: api.tokens });
  const revoke = useMutation({
    mutationFn: (id: string) => api.deleteToken(id),
    onSuccess: () => list.refetch(),
  });
  const devices = list.data?.tokens.filter((t) => t.kind !== "pat") ?? [];
  return (
    <Section
      title="Devices"
      description="Everywhere you are signed in. Revoking signs that device out at once."
    >
      <div className="flex flex-col gap-5">
        <ul className="divide-y divide-line border-y border-line">
          {devices.map((t) => (
            <li key={t.id} className="flex items-center gap-3 py-3">
              <Tile on={t.current}>{t.kind === "web" ? <Monitor /> : <Terminal />}</Tile>
              <div className="min-w-0 flex-1">
                <div className="truncate text-[13.5px] text-ink">
                  {t.name ?? (t.kind === "web" ? browserName(t.user_agent) : "CLI")}
                  {t.current && <span className="ml-2 text-[11.5px] text-accent">this device</span>}
                </div>
                <div className="text-[12px] text-ink-4">
                  {t.kind === "cli" ? "fctl" : "Browser"} · active {ago(t.last_used_at ?? t.created_at)}
                </div>
              </div>
              {!t.current && (
                <Button size="sm" variant="ghost" onClick={() => revoke.mutate(t.id)}>
                  Revoke
                </Button>
              )}
            </li>
          ))}
        </ul>
        <div className="flex items-center gap-3 text-[12.5px] text-ink-3">
          {/* Under the rows' icons. */}
          <span className="flex w-8 shrink-0 justify-center">
            <Laptop className="size-4 text-ink-4" />
          </span>
          <span className="flex-1">
            Sign in the CLI with{" "}
            <code className="rounded bg-raised px-1 font-mono text-[11.5px]">
              fctl login {location.origin}
            </code>
          </span>
        </div>
      </div>
    </Section>
  );
}

function Appearance() {
  const [theme, setTheme] = useTheme();
  const [hours, setHours] = useHourCycle();
  return (
    <Section title="Appearance" description="How fctl looks in this browser.">
      <fieldset>
        <legend className="text-[13.5px] font-medium text-ink">Theme</legend>
        <div className="mt-3 grid max-w-[460px] grid-cols-2 gap-3">
          {(["dark", "light"] as const).map((t) => {
            const chosen = theme === t;
            return (
              <label
                key={t}
                className={cn(
                  "group cursor-pointer rounded-xl border p-1.5 transition-[border-color,box-shadow] duration-150",
                  chosen
                    ? "border-accent/60 shadow-[0_0_0_3px_var(--c-accent-soft)]"
                    : "border-line-strong hover:border-ink-4",
                  "has-focus-visible:outline-2 has-focus-visible:outline-offset-2 has-focus-visible:outline-accent",
                )}
              >
                <input
                  type="radio"
                  name="theme"
                  value={t}
                  checked={chosen}
                  onChange={() => setTheme(t)}
                  className="sr-only"
                />
                <ThemePicture theme={t} />
                <span className="flex items-center gap-2 px-1.5 pt-2.5 pb-1 text-[13px] text-ink">
                  <span
                    className={cn(
                      "flex size-3.5 items-center justify-center rounded-full border transition-colors",
                      chosen ? "border-accent" : "border-line-strong group-hover:border-ink-4",
                    )}
                  >
                    {chosen && <span className="size-1.5 rounded-full bg-accent" />}
                  </span>
                  {t === "dark" ? "Dark" : "Light"}
                  {t === "dark" ? (
                    <Moon className="ml-auto size-3.5 text-ink-4" />
                  ) : (
                    <Sun className="ml-auto size-3.5 text-ink-4" />
                  )}
                </span>
              </label>
            );
          })}
        </div>
      </fieldset>
      <ul className="mt-8 divide-y divide-line border-y border-line">
        <SwitchRow
          label="24-hour clock"
          hint={`Times read ${sampleTime(24)} rather than ${sampleTime(12)}. Until you change it here, it follows this browser's language.`}
          checked={hours === 24}
          onChange={(on) => setHours(on ? 24 : 12)}
        />
      </ul>
    </Section>
  );
}

/**
 * A small picture of fctl in a theme: a sidebar, a page and its primary button, in the theme's own
 * colors (the brand's, from the design package) whichever theme the page around it has.
 */
function ThemePicture({ theme }: { theme: "dark" | "light" }) {
  const dark = theme === "dark";
  const ink = dark ? palette.whiteish : palette.blackish;
  const canvas = themes[theme].canvas;
  const line = withAlpha(ink, dark ? 0.08 : 0.09);
  const faint = withAlpha(ink, dark ? 0.16 : 0.13);
  const accent = dark ? palette.yellow : palette.lime;
  const bar = (width: string, color = faint, height = 4) => (
    <span className="block rounded-full" style={{ width, height, background: color }} />
  );
  return (
    <span
      aria-hidden="true"
      className="flex aspect-[16/10] overflow-hidden rounded-lg"
      style={{ background: canvas, boxShadow: `inset 0 0 0 1px ${line}` }}
    >
      <span
        className="flex w-[32%] flex-col gap-[7px] px-2.5 py-3"
        style={{ background: withAlpha(ink, dark ? 0.03 : 0.02), borderRight: `1px solid ${line}` }}
      >
        <span className="mb-1 flex items-center gap-1">
          <span className="size-2 rounded-[3px]" style={{ background: palette.yellow }} />
          {bar("50%", withAlpha(ink, 0.5))}
        </span>
        {bar("80%")}
        <span className="-mx-1 rounded-[4px] px-1 py-[3px]" style={{ background: withAlpha(ink, 0.07) }}>
          {bar("70%", withAlpha(ink, 0.4))}
        </span>
        {bar("60%")}
        {bar("75%")}
      </span>
      <span className="flex flex-1 flex-col gap-[7px] p-3">
        {bar("55%", withAlpha(ink, 0.85), 7)}
        <span className="mt-1 block" />
        {bar("90%")}
        {bar("75%")}
        {bar("82%")}
        <span className="mt-auto block h-[11px] w-9 rounded-[3px]" style={{ background: accent }} />
      </span>
    </span>
  );
}

/** What the export holds. */
const EXPORT_HOLDS = [
  "Your account",
  "Sign-in methods and passkeys",
  "Two-factor sign-in",
  "Devices and tokens",
  "Claude connections, without their secrets",
  "Notification settings and browsers",
  "Projects",
  "Schedules you set up",
  "Sessions you started, with full transcripts",
  "Sessions you follow",
  "Your activity log",
];

function YourData({ me }: { me: G.Me }) {
  useTickAt(me.export?.next_at);
  const now = Date.now();
  const waiting = nextExport(me.export, now) !== undefined;
  const hint = exportHint(me.export, now);
  return (
    <Section
      title="Your data"
      description="A copy of everything this instance keeps about you, as one JSON file."
    >
      {/* Down one column, then the next. */}
      <ul className="gap-x-8 text-[13px] leading-snug text-ink-2 @xl:columns-2">
        {EXPORT_HOLDS.map((what) => (
          <li key={what} className="flex break-inside-avoid items-start gap-2 pb-2">
            <Check className="mt-px size-3.5 shrink-0 text-accent" />
            {what}
          </li>
        ))}
      </ul>
      <div className="mt-6 flex flex-wrap items-center gap-x-4 gap-y-2">
        <ExportLink
          waiting={waiting}
          className={cn(
            "inline-flex h-8.5 items-center gap-2 rounded-md border border-line-strong bg-raised px-3.5 text-[13.5px] font-medium transition-colors",
            waiting ? "cursor-not-allowed text-ink-4" : "text-ink hover:border-ink-4",
          )}
        >
          <Download className="size-3.5" />
          Export your data
        </ExportLink>
        {hint && <p className="max-w-xl text-[12.5px] leading-relaxed text-ink-4">{hint}</p>}
      </div>
      <div className="mt-5">
        <AccountDangerZone me={me} />
      </div>
    </Section>
  );
}

const EXPORT = "/api/v1/me/export";

/**
 * Downloads your data. Where sensitive actions ask you to confirm it's you,
 * it asks first: the browser makes the download, so it can't ask itself.
 * While you wait for your next export (`waiting`), it does nothing.
 */
function ExportLink({
  className,
  children,
  waiting,
}: {
  className: string;
  children: ReactNode;
  waiting?: boolean;
}) {
  const [busy, setBusy] = useState(false);
  return (
    <a
      href={waiting ? undefined : EXPORT}
      download={waiting ? undefined : true}
      aria-disabled={waiting || undefined}
      aria-busy={busy || undefined}
      className={className}
      onClick={(e) => {
        e.preventDefault();
        if (busy || waiting) return;
        setBusy(true);
        // As the instance has it now: the browser would save a refusal as the file.
        queryClient
          .fetchQuery({ queryKey: keys.me, queryFn: () => fetchMe(), staleTime: 0 })
          .then((me) => (me && nextExport(me.export) !== undefined ? false : confirmFirst()))
          .then((confirmed) => {
            if (!confirmed) return;
            const a = document.createElement("a");
            a.href = EXPORT;
            a.download = "";
            a.click();
            queryClient.setQueryData<G.Me | null>(keys.me, (m) => m && exported(m));
            // Then as the instance has it: it gives back an export that fails on its side.
            setTimeout(() => void queryClient.invalidateQueries({ queryKey: keys.me }), 5_000);
          })
          .catch((err: Error) => toast(err.message, "error"))
          .finally(() => setBusy(false));
      }}
    >
      {children}
    </a>
  );
}

function AccountDangerZone({ me }: { me: G.Me }) {
  const [open, setOpen] = useState(false);
  return (
    <DangerZone
      title="Delete your account"
      description="Deletes your account, addresses, sign-in methods, devices, tokens and Claude connections, and the workspaces only you are in, with their projects and sessions. This cannot be undone."
      action="Delete account…"
      onAction={() => setOpen(true)}
    >
      <DeleteAccount me={me} open={open} onClose={() => setOpen(false)} />
    </DangerZone>
  );
}

const sameEmail = (typed: string, email: string) => typed.trim().toLowerCase() === email.trim().toLowerCase();

function DeleteAccount({ me, open, onClose }: { me: G.Me; open: boolean; onClose: () => void }) {
  const [typed, setTyped] = useState("");
  const plan = useQuery({ queryKey: keys.accountDeletion, queryFn: api.accountDeletion, enabled: open });
  const remove = useMutation({
    mutationFn: () => api.deleteAccount({ email: typed }),
    onSuccess: async () => {
      forgetLeft();
      await forgetPush();
      await forgetDraft();
      queryClient.clear();
      forgetSidebar(); // last, so that no list that comes is kept again
      forgetLog();
      goTo("/login?deleted=1");
    },
  });
  useEffect(() => {
    if (!open) {
      setTyped("");
      remove.reset();
    }
  }, [open, remove.reset]);
  const p = plan.data;
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title="Delete your account?"
      description="This deletes your data for good. It cannot be undone."
    >
      {plan.isPending ? (
        <div className="flex justify-center py-6">
          <Spinner className="size-5" />
        </div>
      ) : plan.error ? (
        <Banner tone="error">{plan.error.message}</Banner>
      ) : p?.blocked ? (
        <div className="flex flex-col gap-5">
          <Banner tone="warn">{p.blocked}</Banner>
          <div className="flex justify-end">
            <Button onClick={onClose}>Close</Button>
          </div>
        </div>
      ) : (
        <form
          className="flex flex-col gap-5"
          onSubmit={(e) => {
            e.preventDefault();
            if (sameEmail(typed, me.user.email)) remove.mutate();
          }}
        >
          <ul className="flex list-disc flex-col gap-1.5 pl-5 text-[13.5px] leading-relaxed text-ink-2 marker:text-ink-4">
            <li>
              Your account <span className="text-ink">{me.user.email}</span>
              {me.emails.length > 1 && " and its other addresses"}, sign-in methods, devices and tokens, and
              Claude connections are deleted.
            </li>
            {p?.workspaces.map((w) => (
              <li key={w.id}>
                <span className="text-ink">{w.name}</span> is deleted: no one else is in it.
              </li>
            ))}
            {p?.projects.map((project) => (
              <li key={project.slug}>
                Its project <span className="text-ink">{project.name}</span> goes
                {project.sessions === 0
                  ? ""
                  : project.sessions === 1
                    ? " with its session and transcript"
                    : ` with its ${project.sessions} sessions and their transcripts`}
                .
              </li>
            ))}
            {p?.handovers.map((h) => (
              <li key={h.workspace.id}>
                <span className="text-ink">{h.to.name}</span> becomes the primary admin of{" "}
                <span className="text-ink">{h.workspace.name}</span>.
              </li>
            ))}
            {p && p.kept_sessions > 0 && (
              <li>
                {p.kept_sessions === 1 ? "The session" : `The ${p.kept_sessions} sessions`} you started in
                shared workspaces {p.kept_sessions === 1 ? "stays" : "stay"} there, shown as started by a
                deleted user.
              </li>
            )}
            <li>Repositories on GitHub are not touched.</li>
          </ul>
          <p className="text-[13px] leading-relaxed text-ink-3">
            {nextExport(me.export) === undefined ? (
              <>
                Want a copy first?{" "}
                <ExportLink className="text-ink underline decoration-accent underline-offset-2">
                  Export your data
                </ExportLink>
                .
              </>
            ) : (
              <>Want a copy first? {exportHint(me.export)}</>
            )}
          </p>
          <Field
            label={
              <>
                Type <span className="font-mono text-ink">{me.user.email}</span> to confirm
              </>
            }
            error={remove.error?.message}
          >
            {(id) => (
              <Input
                id={id}
                value={typed}
                onChange={(e) => setTyped(e.target.value)}
                autoComplete="off"
                autoCapitalize="off"
                spellCheck={false}
              />
            )}
          </Field>
          <div className="flex justify-end gap-2">
            <Button type="button" variant="ghost" onClick={onClose}>
              Cancel
            </Button>
            <Button
              type="submit"
              variant="danger"
              loading={remove.isPending}
              disabled={!sameEmail(typed, me.user.email)}
            >
              Delete my account
            </Button>
          </div>
        </form>
      )}
    </Dialog>
  );
}

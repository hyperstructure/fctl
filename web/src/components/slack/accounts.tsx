// Settings → Slack: who fctl knows you as in the Slack workspaces your workspaces work in (what you
// ask fctl's app there starts sessions as you), linking an account where it doesn't know you yet,
// the project your direct messages with fctl start sessions in, and unlinking. Where the instance
// asks people to confirm it's them before linking, the link comes back here first (`?slack=confirm`)
// and goes on once they have.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { Fingerprint } from "lucide-react";
import { useEffect, useRef, useState } from "react";
import {
  Avatar,
  Banner,
  Button,
  Dialog,
  Section,
  Select,
  type SelectOption,
  Skeleton,
  SlackIcon,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { api, clickConfirmed, goConfirmed, slackLinkUrl } from "@/lib/api";
import { keys, queryClient, useInstance, useProjects } from "@/lib/queries";
import { agoAfterVerb, linkOutcome, type SlackOutcome } from "@/lib/slack";

export interface SlackReturn {
  slack?: SlackOutcome;
  /** The Slack workspace, and the thread that asked to link an account there. */
  team?: string;
  thread?: string;
}

/** Your Slack accounts. */
export const useMySlack = (enabled = true) =>
  useQuery({ queryKey: keys.mySlack, queryFn: api.mySlack, enabled });

export function SlackAccounts({ returned }: { returned: SlackReturn }) {
  const { data: instance } = useInstance();
  const on = !!instance?.features.slack;
  const mine = useMySlack(on);
  const [dropping, setDropping] = useState<G.SlackAccount | null>(null);
  const unlink = useMutation({
    mutationFn: (team: string) => api.unlinkSlack(team),
    onSuccess: (m, team) => {
      queryClient.setQueryData(keys.mySlack, m);
      void queryClient.invalidateQueries({ queryKey: keys.notifications });
      void queryClient.invalidateQueries({ queryKey: ["workspace"] });
      setDropping(null);
      const name = m.teams?.find((t) => t.team === team)?.name;
      toast(name ? `Unlinked: what you ask fctl in ${name} starts nothing now` : "Unlinked");
    },
    onError: (e) => toast(e.message, "error"),
  });
  if (!on) return null;
  const m = mine.data;
  const accounts = m?.accounts ?? [];
  const linkedTeams = new Set(accounts.map((a) => a.team));
  // The Slack workspaces your workspaces work in, where fctl doesn't know you yet.
  const unlinked = (m?.teams ?? []).filter((t) => !linkedTeams.has(t.team));
  const outcome = returned.slack && linkOutcome(returned.slack);
  return (
    <Section
      title="Slack"
      description="Who fctl knows you as in Slack: what you ask fctl's app there starts sessions as you, on your Claude subscription, and your replies there reach your sessions' agents."
    >
      <div className="flex flex-col gap-5">
        {returned.slack === "confirm" && <Confirm returned={returned} teams={m?.teams} />}
        {outcome && <Banner tone={outcome.tone}>{outcome.text}</Banner>}
        {mine.isPending ? (
          <div className="flex flex-col gap-2">
            <Skeleton className="h-14" />
            <Skeleton className="h-14" />
          </div>
        ) : mine.isError ? (
          <Banner tone="error">
            Your Slack accounts didn't load: {mine.error.message}{" "}
            <button
              type="button"
              className="underline underline-offset-2"
              onClick={() => void mine.refetch()}
            >
              Try again
            </button>
          </Banner>
        ) : accounts.length === 0 && unlinked.length === 0 ? (
          <p className="border-y border-line py-4 text-[13.5px] leading-relaxed text-ink-3">
            None of your workspaces works in a Slack workspace yet. Once one's admins connect it, under the
            workspace's Integrations, you link your account here.
          </p>
        ) : (
          <ul className="divide-y divide-line border-y border-line">
            {accounts.map((a) => (
              <Account
                key={a.team}
                account={a}
                team={m?.teams?.find((t) => t.team === a.team)}
                onUnlink={() => setDropping(a)}
              />
            ))}
            {unlinked.map((t) => (
              <Unlinked key={t.team} team={t} />
            ))}
          </ul>
        )}
        {accounts.length > 0 && (
          <p className="text-[12.5px] leading-relaxed text-ink-4">
            Someone else's Slack account can't start sessions as you: fctl links only the account you sign in
            to Slack with{instance?.features.email ? ", and emails you when one is linked" : ""}.
          </p>
        )}
      </div>
      <Dialog
        open={!!dropping}
        onClose={() => setDropping(null)}
        title={`Unlink your account in ${dropping?.team_name ?? "Slack"}?`}
        description={
          dropping?.via === "email"
            ? `fctl forgets it. It knows you there again by your address the next time you ask fctl in ${dropping.team_name}, while its admins let it. Sessions already running carry on.`
            : "What you ask fctl there starts nothing, and your replies there don't reach agents, until you link it again. Sessions already running carry on."
        }
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDropping(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={unlink.isPending}
            onClick={() => dropping && unlink.mutate(dropping.team)}
          >
            Unlink
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

/**
 * On the way to link an account, where the instance asks people to confirm it's them first: it asks
 * at once, as the link was what they asked for, and goes on to Slack once they have. Should they
 * close the question, the card asks again.
 */
function Confirm({ returned, teams }: { returned: SlackReturn; teams?: G.SlackTeamRef[] }) {
  const href = slackLinkUrl({ team: returned.team, thread: returned.thread });
  const name = teams?.find((t) => t.team === returned.team)?.name;
  const [going, setGoing] = useState(false);
  const asked = useRef(false);
  const go = () => {
    setGoing(true);
    goConfirmed(href)
      .catch((e: Error) => toast(e.message, "error"))
      // Still here: they didn't confirm, or the browser stays while Slack loads.
      .finally(() => setTimeout(() => setGoing(false), 1500));
  };
  // biome-ignore lint/correctness/useExhaustiveDependencies: once, as the page opens on the way to link
  useEffect(() => {
    if (asked.current) return;
    asked.current = true;
    go();
  }, []);
  return (
    <div className="flex flex-wrap items-center gap-x-4 gap-y-3 rounded-xl border border-blue/25 bg-blue-soft px-4 py-3.5">
      <span
        aria-hidden="true"
        className="flex size-9 shrink-0 items-center justify-center rounded-full bg-surface text-blue"
      >
        <Fingerprint className="size-[18px]" />
      </span>
      <div className="min-w-0 flex-1 basis-64">
        <p className="text-[14px] font-medium text-ink">
          Confirm it's you to link your Slack account{name ? ` in ${name}` : ""}
        </p>
        <p className="mt-0.5 text-[12.5px] leading-relaxed text-ink-3">
          This instance asks before anyone links an account that starts sessions as them. Then Slack asks you
          to sign in{returned.thread ? ", and your request there starts" : ""}.
        </p>
      </div>
      <Button variant="primary" loading={going} onClick={go}>
        Confirm it's you
      </Button>
    </div>
  );
}

/**
 * One of your Slack accounts: whom fctl knows you as there, how (you linked it, or your address
 * matched), and the project your direct messages with fctl start sessions in.
 */
function Account({
  account: a,
  team,
  onUnlink,
}: {
  account: G.SlackAccount;
  team?: G.SlackTeamRef;
  onUnlink: () => void;
}) {
  const projects = useProjects().data?.projects;
  const save = useMutation({
    mutationFn: (dm_project: string) => api.updateMySlack(a.team, { dm_project }),
    onSuccess: (m, dm) => {
      queryClient.setQueryData(keys.mySlack, m);
      const p = m.accounts.find((x) => x.team === a.team)?.dm_project;
      toast(
        dm
          ? `Your direct messages with fctl in ${a.team_name} start sessions in ${p?.name ?? "it"} now`
          : `fctl asks which project for each of your direct messages in ${a.team_name} now`,
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  // The projects you start sessions in, of the workspaces that work in that Slack workspace.
  const workspaces = new Set((team?.workspaces ?? []).map((w) => w.id));
  const options: SelectOption[] = [
    {
      value: "",
      label: "Ask each time",
      description: "Unless you start sessions in one project alone",
    },
    ...(projects ?? [])
      .filter((p) => p.workspace && workspaces.has(p.workspace.id) && !p.archived_at && p.role !== "viewer")
      .map((p) => ({
        value: p.id,
        label: p.name,
        description: team && team.workspaces.length > 1 ? p.workspace?.name : undefined,
      })),
  ];
  // A project it keeps that isn't listed (you left it, say) stays named.
  if (a.dm_project && !options.some((o) => o.value === a.dm_project?.id))
    options.push({ value: a.dm_project.id, label: a.dm_project.name });
  const facts = [
    a.email,
    a.team_domain && `${a.team_domain}.slack.com`,
    a.via === "email"
      ? `matched by your address ${agoAfterVerb(a.linked_at)}`
      : `linked ${agoAfterVerb(a.linked_at)}`,
  ].filter(Boolean);
  return (
    <li className="flex flex-col gap-3 py-3.5">
      <div className="flex items-center gap-3">
        <span className="relative shrink-0">
          <Avatar user={{ id: a.slack_user, name: a.name, avatar_url: a.avatar_url }} size={32} />
          <span className="absolute -right-1 -bottom-1 flex size-4 items-center justify-center rounded-full bg-canvas">
            <SlackIcon className="size-3" />
          </span>
        </span>
        <div className="min-w-0 flex-1">
          <div className="truncate text-[13.5px] text-ink">
            {a.name}
            <span className="text-ink-4"> in {a.team_name}</span>
          </div>
          <div className="truncate text-[12px] text-ink-4">{facts.join(" · ")}</div>
        </div>
        <Button
          size="sm"
          variant="ghost"
          aria-label={`Unlink your account in ${a.team_name}`}
          onClick={onUnlink}
        >
          Unlink
        </Button>
      </div>
      {a.via === "email" && (
        <p className="text-[12.5px] leading-relaxed text-ink-3">
          fctl knows you there by your verified address, as{" "}
          {team?.workspaces.map((w) => w.name).join(" and ") ?? "your workspace"}'s admins let it. Link your
          account to be known there whatever your address.
        </p>
      )}
      <div className="flex flex-wrap items-center gap-x-2 gap-y-1.5 text-[13px] text-ink-2 sm:pl-[44px]">
        <span>Your direct messages with fctl start sessions in</span>
        <Select
          size="sm"
          aria-label={`The project your direct messages with fctl in ${a.team_name} start sessions in`}
          value={a.dm_project?.id ?? ""}
          options={options}
          disabled={save.isPending}
          onChange={(v) => save.mutate(v)}
        />
      </div>
    </li>
  );
}

/** A Slack workspace one of your workspaces works in, where fctl doesn't know you yet. */
function Unlinked({ team }: { team: G.SlackTeamRef }) {
  const href = slackLinkUrl({ team: team.team });
  return (
    <li className="flex flex-wrap items-center gap-x-3 gap-y-2 py-3.5">
      <span className="flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-md border border-line bg-raised">
        {team.icon_url ? (
          <img src={team.icon_url} alt="" className="size-full object-cover" referrerPolicy="no-referrer" />
        ) : (
          <SlackIcon className="size-4" />
        )}
      </span>
      <div className="min-w-0 flex-1 basis-48">
        <div className="truncate text-[13.5px] text-ink">{team.name}</div>
        <div className="text-[12px] leading-relaxed text-ink-4">
          {team.workspaces.map((w, i) => (
            <span key={w.id}>
              {i > 0 && (i === team.workspaces.length - 1 ? " and " : ", ")}
              <Link
                to="/w/$slug"
                params={{ slug: w.slug }}
                search={{ tab: "integrations", integration: "slack" }}
                className="text-ink-3 underline-offset-2 hover:text-ink hover:underline"
              >
                {w.name}
              </Link>
            </span>
          ))}{" "}
          {team.workspaces.length === 1 ? "works" : "work"} there: until you link your account, what you ask
          fctl there starts nothing.
        </div>
      </div>
      <a
        href={href}
        onClick={(e) => clickConfirmed(e, href)?.catch((err: Error) => toast(err.message, "error"))}
        className="inline-flex h-7 shrink-0 items-center gap-1.5 rounded-sm border border-line-strong bg-raised px-2.5 text-[12.5px] font-medium text-ink transition-colors hover:border-ink-4"
      >
        <SlackIcon className="size-3.5" />
        Link your account
      </a>
    </li>
  );
}

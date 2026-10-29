// A workspace's Linear workspaces: where fctl's app is installed for it, which of its projects take
// tickets from each, and connecting another (the workspace's admins, who are admins in Linear too).
// Until it works in one, the tab says so plainly, and what connecting one leads to.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowRight, AtSign, Check, ExternalLink, Link2, RotateCcw, Ticket, Trash2 } from "lucide-react";
import { type ReactNode, useId, useState } from "react";
import { DelegatedTicket } from "@/components/project/tickets";
import { Banner, Button, Dialog, LinearIcon, Skeleton, toast } from "@/components/ui";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { keys, queryClient, useProjects } from "@/lib/queries";
import { type LinearOutcome, linearOutcome } from "@/lib/tickets";

export function LinearTab({ w, returned }: { w: G.Workspace; returned?: LinearOutcome }) {
  const admin = w.role === "owner" || w.role === "admin";
  const linear = useQuery({ queryKey: keys.workspaceLinear(w.id), queryFn: () => workspaces.linear(w.id) });
  const connect = useMutation({
    mutationFn: () => workspaces.connectLinear(w.id),
    // On to Linear, which brings the browser back here.
    onSuccess: (link) => window.location.assign(link.url),
    onError: (e) => toast(e.message, "error"),
  });
  const [dropping, setDropping] = useState<G.LinearConnection | null>(null);
  const disconnect = useMutation({
    mutationFn: (organization: string) => workspaces.disconnectLinear(w.id, organization),
    onSuccess: (l) => {
      queryClient.setQueryData(keys.workspaceLinear(w.id), l);
      void queryClient.invalidateQueries({ queryKey: ["project-tickets"] });
      setDropping(null);
      toast("Disconnected");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const heading = useId();
  const l = linear.data;
  const outcome = returned && linearOutcome(returned);
  if (linear.isPending) return <Skeleton className="h-32" />;
  if (linear.isError) return <Banner tone="error">{linear.error.message}</Banner>;
  if (l && !l.configured)
    return (
      <Banner tone="info">
        This instance has no Linear app yet. Its admins set one up: the self-hosting guide says how.
      </Banner>
    );
  const connecting = connect.isPending || connect.isSuccess;
  const connections = l?.connections ?? [];
  const active = connections.filter((c) => c.status === "active");
  return (
    <div className="flex flex-col gap-10">
      {outcome && (
        <Banner tone={outcome.tone} className="-mb-4">
          {returned === "connected"
            ? `Linear is connected: fctl's app is installed in your Linear workspace. Choose the teams each project takes tickets from, in its Tickets tab.`
            : outcome.text}
        </Banner>
      )}
      <section aria-labelledby={heading} className="animate-rise">
        <h2
          id={heading}
          className="font-display text-[26px] leading-[1.15] font-medium tracking-[-0.018em] text-ink sm:text-[28px]"
        >
          Delegate Linear issues to fctl
        </h2>
        {/* What happens once it works in a Linear workspace, said as what connecting one does until then. */}
        <p className="mt-2 max-w-2xl text-[14.5px] leading-relaxed text-ink-3">
          {active.length === 0
            ? "Connect a Linear workspace, and people there delegate issues to fctl"
            : `People in ${active.length === 1 ? active[0]?.name : "these Linear workspaces"} delegate issues to fctl`}
          : each one starts a session of the project that takes the issue's team, as whoever delegated it, and
          the agent reports back in the issue.
        </p>
        {l && !l.webhook && (
          <Banner tone="warn" className="mt-4 max-w-2xl">
            This instance takes no Linear webhooks yet, so delegating tickets starts nothing. Its admins set
            LINEAR_WEBHOOK_SECRET.
          </Banner>
        )}
        {connections.length === 0 ? (
          <NotConnected w={w} admin={admin} connecting={connecting} onConnect={() => connect.mutate()} />
        ) : (
          <ul className="mt-6 flex flex-col gap-2.5">
            {connections.map((c) => (
              <Connection
                key={c.organization}
                w={w}
                c={c}
                admin={admin}
                connecting={connecting}
                onConnect={() => connect.mutate()}
                onDrop={() => setDropping(c)}
              />
            ))}
          </ul>
        )}
      </section>
      {connections.length === 0 ? (
        <OnceConnected />
      ) : admin ? (
        <section className="grid items-center gap-x-8 gap-y-4 md:grid-cols-[minmax(0,1fr)_auto]">
          <div>
            <h3 className="text-[13.5px] font-semibold text-ink">Connect another Linear workspace</h3>
            <p className="mt-1 max-w-2xl text-[13px] leading-relaxed text-ink-3">
              You don't need one unless {w.name}'s projects take tickets from another Linear workspace's
              teams. Linear asks one of its admins to install fctl's app there, and to choose the teams it may
              see.
            </p>
          </div>
          <div>
            <Button
              icon={<LinearIcon className="size-3.5" />}
              loading={connecting}
              onClick={() => connect.mutate()}
            >
              Connect Linear
            </Button>
          </div>
        </section>
      ) : (
        <p className="text-[13px] text-ink-4">{w.name}'s admins connect Linear workspaces.</p>
      )}
      <Dialog
        open={!!dropping}
        onClose={() => setDropping(null)}
        title={`Stop working in ${dropping?.name ?? ""}?`}
        description={`${w.name}'s projects take no more tickets from it, and their agents reach none of its tickets. Sessions it started carry on, without reporting back there. Once no fctl workspace works in it, fctl gives up its access there.`}
      >
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setDropping(null)}>
            Cancel
          </Button>
          <Button
            variant="danger"
            loading={disconnect.isPending}
            onClick={() => dropping && disconnect.mutate(dropping.organization)}
          >
            Disconnect
          </Button>
        </div>
      </Dialog>
    </div>
  );
}

/** A Linear workspace it works in: as whom, how it stands, and the projects that take its tickets. */
function Connection({
  w,
  c,
  admin,
  connecting,
  onConnect,
  onDrop,
}: {
  w: G.Workspace;
  c: G.LinearConnection;
  admin: boolean;
  connecting: boolean;
  onConnect: () => void;
  onDrop: () => void;
}) {
  return (
    <li className="flex items-start gap-3.5 rounded-xl border border-line bg-surface/60 px-4 py-3.5">
      <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-line bg-raised text-ink">
        {c.logo_url ? (
          <img src={c.logo_url} alt="" className="size-full object-cover" />
        ) : (
          <LinearIcon className="size-4" />
        )}
      </span>
      <div className="min-w-0 flex-1">
        {/* How it stands beside its name, on a phone too. */}
        <div className="flex items-center justify-between gap-3">
          <a
            href={`https://linear.app/${c.url_key}`}
            target="_blank"
            rel="noreferrer"
            className="inline-flex min-w-0 items-center gap-1.5 text-[14px] text-ink hover:underline"
          >
            <span className="truncate">{c.name}</span>
            <ExternalLink className="size-3 shrink-0 text-ink-4" />
          </a>
          <div className="-my-1 flex shrink-0 items-center gap-1">
            {c.status === "active" ? (
              <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
                <Check className="size-3.5" /> Connected
              </span>
            ) : admin ? (
              <Button
                size="sm"
                icon={<RotateCcw className="size-3.5" />}
                loading={connecting}
                onClick={onConnect}
              >
                Connect again
              </Button>
            ) : (
              <span className="text-[12.5px] text-amber">Access revoked</span>
            )}
            {admin && (
              <button
                type="button"
                aria-label={`Disconnect ${c.name}`}
                className="rounded p-1.5 text-ink-4 hover:text-danger"
                onClick={onDrop}
              >
                <Trash2 className="size-3.5" />
              </button>
            )}
          </div>
        </div>
        <div className="mt-0.5 text-[12px] text-ink-4">
          linear.app/{c.url_key}
          {/* Whom people there assign issues to and mention. */}
          {c.app_handle && (
            <>
              {" · as "}
              <span className="font-mono text-ink-3">@{c.app_handle}</span>
            </>
          )}
          {c.connected_by && ` · connected by ${c.connected_by.name}`}
        </div>
        {c.projects && c.projects.length > 0 ? (
          <ul className="mt-2 flex flex-wrap gap-1.5">
            {c.projects.map((p) => (
              <li key={p.slug}>
                <Link
                  to="/p/$slug"
                  params={{ slug: p.slug }}
                  search={{ tab: "tickets" }}
                  className="inline-flex items-center gap-1.5 rounded-full border border-line px-2 py-0.5 text-[12px] text-ink-2 transition-colors hover:bg-hover hover:text-ink"
                >
                  {p.name}
                  {/* Its teams' keys; its first alone from control planes before several. */}
                  {(p.team_keys?.length || p.team_key) && (
                    <span className="font-mono text-[11px] text-ink-4">
                      {p.team_keys?.length ? p.team_keys.join(" · ") : p.team_key}
                    </span>
                  )}
                  <ArrowRight className="size-3 text-ink-4" />
                </Link>
              </li>
            ))}
          </ul>
        ) : (
          <NoProjectYet w={w} />
        )}
      </div>
    </li>
  );
}

/** For a workspace that works in no Linear workspace yet: that it doesn't, and connecting one, or who does. */
function NotConnected({
  w,
  admin,
  connecting,
  onConnect,
}: {
  w: G.Workspace;
  admin: boolean;
  connecting: boolean;
  onConnect: () => void;
}) {
  return (
    <div className="mt-6 grid grid-cols-[auto_minmax(0,1fr)] items-start gap-x-4 rounded-xl border border-line bg-surface/60 p-4 sm:grid-cols-[auto_minmax(0,1fr)_auto] sm:items-center sm:p-5">
      <span
        aria-hidden="true"
        className="flex size-10 items-center justify-center rounded-lg border border-line bg-raised text-ink-3"
      >
        <LinearIcon className="size-[18px]" />
      </span>
      <div>
        <p className="text-[14.5px] font-medium text-ink">{w.name} isn't connected to Linear yet</p>
        <p className="mt-1 text-[13px] leading-relaxed text-ink-3">
          {admin
            ? "Connecting takes an admin of your Linear workspace: Linear asks them to install fctl's app there and choose the teams it may see."
            : `${w.name}'s admins connect it: ask one who is an admin of your Linear workspace too.`}
        </p>
      </div>
      {/* Under the words on a phone; beside them on wider screens. */}
      {admin && (
        <div className="col-start-2 mt-4 sm:col-start-3 sm:row-start-1 sm:mt-0">
          <Button
            variant="primary"
            icon={<LinearIcon className="size-3.5" />}
            loading={connecting}
            onClick={onConnect}
          >
            Connect Linear
          </Button>
        </div>
      )}
    </div>
  );
}

/** What follows connecting a Linear workspace, beside what a delegated ticket then looks like there. */
function OnceConnected() {
  const heading = useId();
  const steps: { icon: ReactNode; text: ReactNode }[] = [
    {
      icon: <Ticket />,
      text: "Each project chooses the Linear teams it takes tickets from, in its Tickets tab.",
    },
    {
      icon: <Link2 />,
      text: (
        <>
          People link their Linear account once, in{" "}
          <Link
            to="/settings"
            search={{ linked: undefined, error: undefined }}
            hash="linear"
            className="text-ink underline underline-offset-2"
          >
            Settings
          </Link>
          : what they delegate then starts sessions as them, on their Claude subscription.
        </>
      ),
    },
    {
      icon: <AtSign />,
      text: "In Linear, they assign an issue to fctl, or mention fctl in a comment on one. The session's steps, pull requests and replies show in the issue, and a reply there steers it.",
    },
  ];
  return (
    <section aria-labelledby={heading}>
      <h3 id={heading} className="text-[14px] font-semibold text-ink">
        Once Linear is connected
      </h3>
      <div className="mt-4 grid items-start gap-x-12 gap-y-8 md:grid-cols-[minmax(0,1fr)_320px]">
        <ol className="flex flex-col gap-3.5">
          {steps.map((s, i) => (
            <li key={i} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
              <span
                aria-hidden="true"
                className="flex size-6 shrink-0 items-center justify-center rounded-full border border-line bg-raised text-ink-3 [&_svg]:size-3.5"
              >
                {s.icon}
              </span>
              <span className="min-w-0 pt-0.5">{s.text}</span>
            </li>
          ))}
        </ol>
        <DelegatedTicket team="ENG" project="web" app="fctl" />
      </div>
    </section>
  );
}

/**
 * For a Linear workspace no project takes tickets from yet: the workspace's projects whose tracker
 * the caller chooses, each a click from its Tickets tab.
 */
function NoProjectYet({ w }: { w: G.Workspace }) {
  const projects = (useProjects().data?.projects ?? []).filter(
    (p) => p.workspace?.id === w.id && !p.archived_at && (p.role === "owner" || p.role === "maintainer"),
  );
  return (
    <div className="mt-1.5">
      <p className="text-[12.5px] text-ink-4">
        No project takes its tickets yet: choose Linear in a project's Tickets tab.
      </p>
      {projects.length > 0 && (
        <ul className="mt-2 flex flex-wrap gap-1.5">
          {projects.map((p) => (
            <li key={p.id}>
              <Link
                to="/p/$slug"
                params={{ slug: p.slug }}
                search={{ tab: "tickets" }}
                className="inline-flex items-center gap-1.5 rounded-full border border-dashed border-line-strong px-2 py-0.5 text-[12px] text-ink-3 transition-colors hover:border-solid hover:bg-hover hover:text-ink"
              >
                {p.name}
                <ArrowRight className="size-3 text-ink-4" />
              </Link>
            </li>
          ))}
        </ul>
      )}
    </div>
  );
}

// A workspace's Linear workspaces: where fctl's app is installed for it, which of its projects take
// tickets from each, and connecting another (the workspace's admins, who are admins in Linear too).

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowRight, Check, ExternalLink, RotateCcw, Trash2 } from "lucide-react";
import { useState } from "react";
import { Banner, Button, Dialog, LinearIcon, Skeleton, toast } from "@/components/ui";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { keys, queryClient } from "@/lib/queries";
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
  return (
    <div>
      {outcome && (
        <Banner tone={outcome.tone} className="mb-6">
          {returned === "connected"
            ? `Linear is connected: fctl's app is installed in your Linear workspace. Choose the team each project takes tickets from, in its Tickets tab.`
            : outcome.text}
        </Banner>
      )}
      <p className="max-w-2xl text-[14px] leading-relaxed text-ink-3">
        {w.name}'s projects take tickets from the Linear workspaces it works in. People there delegate an
        issue to fctl, or mention it, and fctl starts a session of the project that takes the issue's team, as
        whoever asked.
      </p>
      {l && !l.webhook && (
        <Banner tone="warn" className="mt-4 max-w-2xl">
          This instance takes no Linear webhooks yet, so delegating tickets starts nothing. Its admins set
          LINEAR_WEBHOOK_SECRET.
        </Banner>
      )}
      <ul className="mt-6 border-t border-line">
        {l?.connections.map((c) => (
          <li
            key={c.organization}
            className="flex flex-wrap items-start gap-x-3.5 gap-y-2 border-b border-line py-3.5"
          >
            <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center overflow-hidden rounded-lg border border-line bg-raised text-ink">
              {c.logo_url ? (
                <img src={c.logo_url} alt="" className="size-full object-cover" />
              ) : (
                <LinearIcon className="size-4" />
              )}
            </span>
            <div className="min-w-0 flex-1 basis-60">
              <a
                href={`https://linear.app/${c.url_key}`}
                target="_blank"
                rel="noreferrer"
                className="inline-flex items-center gap-1.5 text-[14px] text-ink hover:underline"
              >
                {c.name} <ExternalLink className="size-3 text-ink-4" />
              </a>
              <div className="mt-0.5 text-[12px] text-ink-4">
                linear.app/{c.url_key}
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
                        {p.team_key && <span className="font-mono text-[11px] text-ink-4">{p.team_key}</span>}
                        <ArrowRight className="size-3 text-ink-4" />
                      </Link>
                    </li>
                  ))}
                </ul>
              ) : (
                <p className="mt-1.5 text-[12.5px] text-ink-4">
                  No project takes its tickets yet: choose Linear in a project's Tickets tab.
                </p>
              )}
            </div>
            <div className="flex shrink-0 items-center gap-1">
              {c.status === "active" ? (
                <span className="inline-flex items-center gap-1.5 text-[12.5px] text-accent">
                  <Check className="size-3.5" /> Connected
                </span>
              ) : admin ? (
                <Button
                  size="sm"
                  icon={<RotateCcw className="size-3.5" />}
                  loading={connecting}
                  onClick={() => connect.mutate()}
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
                  onClick={() => setDropping(c)}
                >
                  <Trash2 className="size-3.5" />
                </button>
              )}
            </div>
          </li>
        ))}
        {l && l.connections.length === 0 && (
          <li className="py-6 text-[13.5px] text-ink-4">{w.name} works in no Linear workspace yet.</li>
        )}
      </ul>
      {admin ? (
        <section className="mt-10 grid items-center gap-x-8 gap-y-4 md:grid-cols-[minmax(0,1fr)_auto]">
          <div>
            <h3 className="text-[13.5px] font-semibold text-ink">
              Connect {l && l.connections.length > 0 ? "another" : "a"} Linear workspace
            </h3>
            <p className="mt-1 max-w-2xl text-[13px] leading-relaxed text-ink-3">
              Linear asks one of its admins to install fctl's app, and to choose the teams it may see. Linear
              then lists fctl as an agent people assign issues to and mention; fctl reads and writes the
              issues of the teams projects choose, in its own name.
            </p>
          </div>
          <Button
            icon={<LinearIcon className="size-3.5" />}
            loading={connecting}
            onClick={() => connect.mutate()}
          >
            Connect Linear
          </Button>
        </section>
      ) : (
        <p className="mt-8 text-[13px] text-ink-4">{w.name}'s admins connect Linear workspaces.</p>
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

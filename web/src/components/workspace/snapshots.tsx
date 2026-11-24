// Sessions' snapshots in a workspace (docs/architecture.md#session-snapshots): on its Runners tab,
// what a session takes with it to another runner, where it goes and how to keep it in a bucket of
// your own; in its settings, whether its sessions save one and for how long.

import { useMutation, useQuery } from "@tanstack/react-query";
import { Link } from "@tanstack/react-router";
import { ArrowDown, ArrowUpRight, Container, Database, Lock } from "lucide-react";
import { useState } from "react";
import { Button, CodeBox, Dialog, Section, Select, Switch, toast } from "@/components/ui";
import type * as G from "@/gen";
import { workspaces } from "@/lib/api";
import { cn } from "@/lib/cn";
import { bytes } from "@/lib/format";
import { siteUrl } from "@/lib/install";
import { keys, queryClient } from "@/lib/queries";
import { DEFAULT_SNAPSHOT_DAYS, daysWords, snapshotDays, usageWords } from "@/lib/snapshots";

const BUCKET_GUIDE = () => `${siteUrl()}/docs/self-hosting/#session-snapshots-in-a-bucket-of-your-own`;

/** What the workspace's sessions' snapshots take now, and whether the instance keeps any. */
function useSnapshots(w: G.Workspace) {
  return useQuery({
    queryKey: keys.workspaceSnapshots(w.id),
    queryFn: () => workspaces.snapshots(w.id),
  });
}

/** The workspace's days, from control planes that say them. */
const daysOf = (w: G.Workspace) => w.snapshot_days ?? DEFAULT_SNAPSHOT_DAYS;

/**
 * The Runners tab's last section: what a session takes with it to another runner, where it goes,
 * and how to keep it in a bucket of your own, which is set on the runners themselves.
 */
export function SnapshotsSection({ w, admin }: { w: G.Workspace; admin: boolean }) {
  const usage = useSnapshots(w);
  const u = usage.data;
  const days = daysOf(w);
  return (
    <section id="snapshots" className="mt-12 scroll-mt-6">
      <div className="grid gap-8 md:grid-cols-[minmax(0,1fr)_300px] md:gap-10">
        <div className="min-w-0">
          <h3 className="font-display text-xl text-ink">Sessions that resume anywhere</h3>
          <p className="mt-1 max-w-2xl text-[14px] leading-relaxed text-ink-3">
            As a session's sandbox stops, its agent saves a snapshot of its conversation and the work it
            hadn't pushed, so that the session resumes on any of {w.name}'s runners as it was. It encrypts the
            snapshot in the sandbox with a key of the session's own, which fctl keeps encrypted: wherever the
            snapshot is kept, nobody reads it without that key.
          </p>
          <p className="mt-4 text-[13.5px] leading-relaxed text-ink-2" aria-live="polite">
            {!u ? (
              <span className="text-ink-4">…</span>
            ) : !u.instance ? (
              <span className="text-amber">
                This instance keeps no snapshots: its admins turned them off. Until they turn them on, a
                session that resumes on another runner starts from its task and branch.
              </span>
            ) : w.snapshots ? (
              <>
                On, each kept {daysWords(days)} after it was saved: {usageWords(u)}.{" "}
                <SettingsLink w={w} admin={admin} />
              </>
            ) : (
              <>
                Off: a session that resumes on another runner starts from its task and branch.{" "}
                <SettingsLink w={w} admin={admin} />
              </>
            )}
          </p>
        </div>
        <SnapshotsDiagram usage={u} on={!!u?.instance && !!w.snapshots} />
      </div>
      <div className="mt-8">
        <h4 className="text-[14.5px] font-medium text-ink">To keep them in your own cloud</h4>
        <p className="mt-1 max-w-2xl text-[13.5px] leading-relaxed text-ink-3">
          Give runners a bucket of yours: their hosts put snapshots there with the machine's own credentials,
          and fctl keeps only where each is, its size and its checksum. A session then resumes with its
          snapshot on runners with the same bucket. On each runner:
        </p>
        <CodeBox className="mt-3" value="fctl host install --snapshot-bucket s3://your-bucket/fctl" />
        <p className="mt-2 max-w-2xl text-[12.5px] leading-relaxed text-ink-4">
          Already installed? Set <code className="font-mono text-[0.95em]">[snapshots] url</code> in its
          host.toml, then <code className="font-mono text-[0.95em]">fctl host restart</code>. The bucket needs
          a policy that lets the runners' role write there, and a lifecycle rule that deletes snapshots after{" "}
          {daysWords(days)}: fctl forgets them then, but can't delete them from your bucket.{" "}
          <a
            href={BUCKET_GUIDE()}
            target="_blank"
            rel="noreferrer"
            className="inline-flex items-center gap-0.5 text-accent hover:underline"
          >
            Setting up the bucket <ArrowUpRight className="size-3.5" />
          </a>
        </p>
      </div>
    </section>
  );
}

/** Where admins change it, or who can. */
function SettingsLink({ w, admin }: { w: G.Workspace; admin: boolean }) {
  if (!admin)
    return (
      <span className="text-ink-4">
        {w.primary ? `Its admins choose this in its settings (${w.primary.name} is its primary admin).` : ""}
      </span>
    );
  return (
    <Link
      to="/w/$slug"
      params={{ slug: w.slug }}
      search={{ tab: "settings" }}
      hash="snapshots"
      className="text-accent hover:underline"
    >
      Change in Settings
    </Link>
  );
}

/**
 * A small drawing of where snapshots go: the sandbox seals one with the session's key, and it goes
 * to fctl or to a bucket of your own, with what each holds of this workspace's now.
 */
function SnapshotsDiagram({ usage, on }: { usage?: G.WorkspaceSnapshots; on: boolean }) {
  const buckets = usage?.buckets ?? [];
  const kept = usage?.kept;
  return (
    <div
      aria-hidden
      className={cn(
        "flex select-none flex-col items-stretch gap-2 self-start rounded-xl border border-line bg-surface p-4",
        "shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)] transition-opacity",
        !on && "opacity-60",
      )}
    >
      <div className="rounded-lg border border-line bg-canvas px-3 py-2.5">
        <div className="flex items-center gap-2 text-[12.5px] font-medium text-ink-2">
          <Container className="size-3.5 text-ink-3" /> A session's sandbox
        </div>
        <div className="mt-2 flex flex-wrap gap-1">
          {["conversation", "uncommitted work", "unpushed commits"].map((x) => (
            <span key={x} className="rounded-full border border-line px-2 py-px text-[11px] text-ink-3">
              {x}
            </span>
          ))}
        </div>
        <div className="mt-2 flex items-center gap-1.5 text-[11px] text-ink-4">
          <Lock className="size-3" /> sealed with the session's own key
        </div>
      </div>
      <ArrowDown className="mx-auto size-4 text-ink-4" />
      <div className="flex flex-col gap-2">
        <Place
          title="fctl"
          facts={kept && kept.count > 0 ? `${kept.count} · ${bytes(kept.bytes)}` : "by default"}
          live={!kept || kept.count > 0 || buckets.length === 0}
        />
        {buckets.length > 0 ? (
          buckets
            .slice(0, 2)
            .map((b) => (
              <Place key={b.bucket} title={b.bucket} facts={`${b.count} · ${bytes(b.bytes)}`} live mono />
            ))
        ) : (
          <Place title="your bucket" facts="set on runners" />
        )}
      </div>
      {buckets.length > 2 && (
        <div className="text-center text-[11px] text-ink-4">and {buckets.length - 2} more buckets</div>
      )}
    </div>
  );
}

function Place({
  title,
  facts,
  live,
  mono,
}: {
  title: string;
  facts: string;
  live?: boolean;
  mono?: boolean;
}) {
  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-2 rounded-lg px-3 py-2",
        live ? "border border-line bg-canvas" : "border border-dashed border-line-strong",
      )}
    >
      <Database className={cn("size-3 shrink-0", live ? "text-ink-3" : "text-ink-4")} />
      <span
        className={cn(
          "min-w-0 flex-1 truncate text-[12px] font-medium",
          live ? "text-ink-2" : "text-ink-4",
          mono && "font-mono text-[11px]",
        )}
      >
        {title}
      </span>
      <span className="shrink-0 text-[11px] text-ink-4">{facts}</span>
    </div>
  );
}

/**
 * Whether the workspace's sessions save snapshots, and how long it keeps each: a section of its
 * settings, for its admins. Turning them off asks first: the ones fctl keeps are deleted.
 */
export function SnapshotSettings({ w }: { w: G.Workspace }) {
  const usage = useSnapshots(w);
  const u = usage.data;
  const [confirm, setConfirm] = useState(false);
  const days = daysOf(w);
  const save = useMutation({
    mutationFn: (b: G.UpdateWorkspaceRequest) => workspaces.update(w.id, b),
    onSuccess: (_, b) => {
      void queryClient.invalidateQueries({ queryKey: keys.workspace(w.slug) });
      void queryClient.invalidateQueries({ queryKey: keys.workspaces });
      void queryClient.invalidateQueries({ queryKey: keys.workspaceSnapshots(w.id) });
      setConfirm(false);
      toast(
        b.snapshots === false
          ? `Snapshots are off in ${w.name}: the ones fctl keeps go within the hour`
          : b.snapshots === true
            ? `${w.name}'s sessions save snapshots again, each kept ${daysWords(days)}`
            : `${w.name} keeps each snapshot ${daysWords(b.snapshot_days ?? days)}`,
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  const kept = u?.kept.count ?? 0;
  return (
    <Section
      id="snapshots"
      title="Session snapshots"
      description="What a session takes with it when it resumes on another runner."
    >
      <div className="flex flex-col gap-6">
        {u && !u.instance && (
          <p className="text-[12.5px] leading-relaxed text-amber">
            This instance keeps no snapshots: its admins turned them off. Until they turn them on, no session
            here saves one, whatever this says.
          </p>
        )}
        <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
          <span>
            Save sessions' snapshots
            <span className="block text-[12.5px] leading-relaxed text-ink-4">
              As a session's sandbox stops, its agent saves its conversation and the work it hadn't pushed,
              encrypted with a key of the session's own, so that it resumes on any runner as it was. Without
              them, a session that moves to another runner starts again from its task and branch.
            </span>
          </span>
          <Switch
            checked={!!w.snapshots}
            disabled={save.isPending}
            onChange={(v) => (v ? save.mutate({ snapshots: true }) : setConfirm(true))}
            label="Save sessions' snapshots"
          />
        </div>
        {w.snapshots && (
          <div className="flex items-center justify-between gap-4 text-[13.5px] text-ink-2">
            <span>
              Keep each one
              <span className="block text-[12.5px] leading-relaxed text-ink-4">
                A session keeps only its newest; past these days it is deleted, and a resume starts from the
                branch. Fewer days delete the older ones within the hour.
              </span>
            </span>
            <Select
              size="sm"
              aria-label="Days each snapshot is kept"
              className="shrink-0"
              value={String(days)}
              options={snapshotDays(days)}
              disabled={save.isPending}
              onChange={(v) => save.mutate({ snapshot_days: Number(v) })}
            />
          </div>
        )}
        {u && w.snapshots && (
          <p className="text-[12.5px] leading-relaxed text-ink-4">
            Now: {usageWords(u)}.{" "}
            <Link
              to="/w/$slug"
              params={{ slug: w.slug }}
              search={{ tab: "runners" }}
              hash="snapshots"
              className="text-accent hover:underline"
            >
              Where they go
            </Link>
          </p>
        )}
      </div>
      <Dialog
        open={confirm}
        onClose={() => setConfirm(false)}
        title="Turn session snapshots off?"
        description={`${
          kept > 0
            ? `The ${kept === 1 ? "snapshot" : `${kept} snapshots`} fctl keeps for ${w.name}'s sessions ${kept === 1 ? "is" : "are"} deleted within the hour`
            : `${w.name}'s sessions save no more snapshots`
        }, and a session that resumes on another runner starts from its task and branch. Snapshots in your runners' own buckets go by those buckets' rules.`}
      >
        <div className="mt-2 flex justify-end gap-2">
          <Button variant="ghost" onClick={() => setConfirm(false)}>
            Keep them
          </Button>
          <Button variant="danger" loading={save.isPending} onClick={() => save.mutate({ snapshots: false })}>
            Turn them off
          </Button>
        </div>
      </Dialog>
    </Section>
  );
}

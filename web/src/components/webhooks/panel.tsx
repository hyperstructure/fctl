// Webhooks: a workspace's (its Webhooks tab, for its admins) or a project's (its Settings →
// Webhooks, for its owners and maintainers). Before there are any, what they do and what fctl
// posts, drawn; then the list, each webhook in a line of how it stands, which opens its page
// (page.tsx). Adding one shows its secret once, with how to check what fctl posts.

import { useQuery } from "@tanstack/react-query";
import {
  ArrowRight,
  CircleCheck,
  Gauge,
  KeyRound,
  ListChecks,
  Plus,
  RotateCcw,
  ShieldCheck,
} from "lucide-react";
import { type ReactNode, useState } from "react";
import { Banner, Button, CodeBox, Skeleton } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { agoShortWords } from "@/lib/format";
import { keys, useProjects } from "@/lib/queries";
import { describeWebhook, hookState, hostOf, webhookTitle } from "@/lib/webhooks";
import { type HookScope, scopeId, scopeName, WebhookForm } from "./form";
import { WebhookPage } from "./page";
import { HookGlyph, StatePill, useEventCatalog } from "./parts";
import { VerifyHow } from "./secret";

/** The workspace's projects, which its webhooks may be narrowed to, and whose names they show. */
export function useWorkspaceProjects(workspaceId: string) {
  const all = useProjects().data?.projects ?? [];
  return all.filter((p) => p.workspace?.id === workspaceId);
}

export function WebhooksPanel({
  scope,
  open,
  onOpen,
  titled,
}: {
  scope: HookScope;
  /** The webhook whose page shows, from the address. */
  open?: string;
  onOpen: (id?: string) => void;
  /** It sits under a title and a lead of its own (a project's Settings), which its intro leaves out. */
  titled?: boolean;
}) {
  const id = scopeId(scope);
  const list = useQuery({
    queryKey: keys.webhooks(scope.kind, id),
    queryFn: () => (scope.kind === "workspace" ? api.workspaceWebhooks(id) : api.projectWebhooks(id)),
    // Deliveries go out on their own: how each stands shows within the half minute.
    refetchInterval: 30_000,
  });
  const catalog = useEventCatalog();
  const workspaceId = scope.kind === "workspace" ? scope.workspace.id : (scope.project.workspace?.id ?? "");
  const projects = useWorkspaceProjects(workspaceId);
  const [adding, setAdding] = useState(false);
  const [created, setCreated] = useState<G.CreatedWebhook>();
  // Its page opens on a test: "Send it a test", as a new webhook is added.
  const [testFirst, setTestFirst] = useState<string>();

  if (open)
    return (
      <WebhookPage
        id={open}
        scope={scope}
        projects={projects}
        startTest={testFirst === open}
        onBack={() => {
          setTestFirst(undefined);
          onOpen(undefined);
        }}
      />
    );
  if (list.isError) return <Banner tone="error">{list.error.message}</Banner>;
  if (catalog.isError) return <Banner tone="error">{catalog.error.message}</Banner>;
  if (!list.data || !catalog.data)
    return (
      <div className="space-y-3">
        <Skeleton className="h-14" />
        <Skeleton className="h-14" />
      </div>
    );
  const events = catalog.data.events;
  const own = list.data.webhooks.filter((w) =>
    scope.kind === "workspace" ? true : w.project?.id === scope.project.id,
  );
  const inherited = scope.kind === "project" ? list.data.webhooks.filter((w) => !w.project) : [];
  const full = list.data.count >= list.data.limit;
  const form = (
    <WebhookForm
      scope={scope}
      catalog={events}
      groups={catalog.data.groups}
      projects={projects}
      onDone={(r) => {
        setAdding(false);
        if (r?.created) setCreated(r.created);
      }}
    />
  );
  if (own.length === 0 && inherited.length === 0 && !adding && !created)
    return <Intro scope={scope} onAdd={() => setAdding(true)} full={full} titled={titled} />;
  return (
    <div className="flex flex-col gap-6">
      {created && (
        <Created
          created={created}
          scope={scope}
          onOpen={() => {
            setCreated(undefined);
            setTestFirst(created.webhook.id);
            onOpen(created.webhook.id);
          }}
          onDone={() => setCreated(undefined)}
        />
      )}
      {adding ? (
        form
      ) : (
        <Header
          webhooks={own}
          none={
            scope.kind === "project"
              ? `${scope.project.name} has no webhooks of its own yet.`
              : "No webhooks yet."
          }
          count={list.data.count}
          limit={list.data.limit}
          onAdd={() => {
            setCreated(undefined);
            setAdding(true);
          }}
        />
      )}
      {own.length > 0 && (
        <ul className="border-t border-line">
          {own.map((w) => (
            <Row key={w.id} webhook={w} catalog={events} projects={projects} onOpen={() => onOpen(w.id)} />
          ))}
        </ul>
      )}
      {inherited.length > 0 && scope.kind === "project" && (
        <section aria-labelledby="inherited-heading" className="mt-2">
          <h3 id="inherited-heading" className="text-[13px] font-semibold text-ink">
            {scope.project.workspace?.name ?? "The workspace"}'s, which send {scope.project.name}'s events too
          </h3>
          <p className="mt-1 text-[12.5px] text-ink-3">
            {inherited.some((w) => w.can_manage)
              ? "You manage them on the workspace's Webhooks tab."
              : "The workspace's admins manage them."}
          </p>
          <ul className="mt-3 border-t border-line">
            {inherited.map((w) => (
              <Row
                key={w.id}
                webhook={w}
                catalog={events}
                projects={projects}
                onOpen={w.can_manage ? () => onOpen(w.id) : undefined}
              />
            ))}
          </ul>
        </section>
      )}
    </div>
  );
}

/** How many there are, how many fail, and the button that adds another. */
function Header({
  webhooks,
  none,
  count,
  limit,
  onAdd,
}: {
  webhooks: G.Webhook[];
  /** What it says while there are none. */
  none: string;
  count: number;
  limit: number;
  onAdd: () => void;
}) {
  const failing = webhooks.filter((w) => hookState(w) === "failing").length;
  const off = webhooks.filter((w) => !w.enabled).length;
  const full = count >= limit;
  const facts = [
    `${webhooks.length} ${webhooks.length === 1 ? "webhook" : "webhooks"}`,
    failing > 0 ? `${failing} failing` : undefined,
    off > 0 ? `${off} off` : undefined,
  ].filter(Boolean);
  return (
    <div className="flex flex-wrap items-center justify-between gap-3">
      <p className="text-[13px] text-ink-3">{webhooks.length ? facts.join(" · ") : none}</p>
      <div className="flex items-center gap-3">
        {full && (
          <span className="text-[12.5px] text-ink-4">The workspace has {limit}, the most it may have</span>
        )}
        <Button size="sm" icon={<Plus className="size-3.5" />} onClick={onAdd} disabled={full}>
          Add a webhook
        </Button>
      </div>
    </div>
  );
}

/** A webhook in a line: how it stands, what it posts where, and how its deliveries went lately. */
function Row({
  webhook: w,
  catalog,
  projects,
  onOpen,
}: {
  webhook: G.Webhook;
  catalog: G.WebhookEventInfo[];
  projects: G.Project[];
  /** Opens its page; none for one the caller doesn't manage. */
  onOpen?: () => void;
}) {
  const state = hookState(w);
  const names = projects.filter((p) => w.projects.includes(p.id)).map((p) => p.name);
  const h = w.health;
  const facts = [
    w.project ? `${w.project.name}'s` : undefined,
    h && (h.delivered > 0 || h.failed > 0)
      ? `${h.delivered} delivered${h.failed ? `, ${h.failed} failed` : ""} in the last day`
      : undefined,
    h?.pending ? `${h.pending} waiting` : undefined,
    h?.last ? `last ${agoShortWords(h.last.at)}` : state === "new" ? "nothing sent yet" : undefined,
  ].filter(Boolean);
  const problem = w.disabled_reason
    ? `fctl turned it off. ${w.disabled_reason}`
    : state === "failing" && h?.failing_since
      ? `fctl's attempts have all failed since ${agoShortWords(h.failing_since)}${h.last?.status ? ` (HTTP ${h.last.status})` : ""}: it tries each again over about 21 hours.`
      : undefined;
  const body = (
    <>
      <span className="flex items-center gap-2">
        <span className="min-w-0 truncate text-[14.5px] text-ink">{webhookTitle(w)}</span>
        <StatePill state={state} />
      </span>
      <span className="mt-0.5 block text-[12.5px] leading-snug text-ink-2">
        {describeWebhook(w.url, w.events, names, w.started_by, catalog)}
      </span>
      {facts.length > 0 && <span className="mt-1 block text-[12px] text-ink-4">{facts.join(" · ")}</span>}
      {problem && (
        <span className="mt-1 block text-[12.5px] leading-relaxed text-amber [overflow-wrap:anywhere]">
          {problem}
        </span>
      )}
    </>
  );
  return (
    <li className="border-b border-line">
      {onOpen ? (
        <button
          type="button"
          onClick={onOpen}
          className="group flex w-full items-start gap-3.5 px-3 py-3.5 text-left transition-colors hover:bg-hover/30 focus-visible:bg-hover/30 focus-visible:outline-none"
        >
          <HookGlyph state={state} turnedOff={!!w.disabled_reason} />
          <span className="min-w-0 flex-1">{body}</span>
          <ArrowRight className="mt-2 size-3.5 shrink-0 text-ink-4 transition-transform duration-200 group-hover:translate-x-0.5" />
        </button>
      ) : (
        <div className="flex items-start gap-3.5 px-3 py-3.5">
          <HookGlyph state={state} turnedOff={!!w.disabled_reason} />
          <span className="min-w-0 flex-1">{body}</span>
        </div>
      )}
    </li>
  );
}

const POINTS: { key: string; icon: ReactNode; text: (where: string) => ReactNode }[] = [
  {
    key: "what",
    icon: <ListChecks />,
    text: () =>
      "You choose what it hears of: sessions starting, finishing a turn, asking something or failing, agents' pull requests, reviews and schedules.",
  },
  {
    key: "signed",
    icon: <ShieldCheck />,
    text: () =>
      "Each request is signed with a secret only you and fctl know, as Standard Webhooks sign them, and names the workspace and project it comes from: check both before acting on it.",
  },
  {
    key: "again",
    icon: <RotateCcw />,
    text: () =>
      "A request that fails goes again 7 times over about 21 hours, and its log keeps what went out, the newest 1,000 of the last 30 days, to read and resend.",
  },
  {
    key: "queue",
    icon: <Gauge />,
    text: () =>
      "It never holds a session up: fctl posts from a queue of its own, at most 120 requests a minute to each address.",
  },
];

/** What webhooks do, before there are any, with what fctl posts drawn beside it. */
/** What webhooks do here, in a sentence, for an intro or a page's lead. */
export function webhooksLead(scope: HookScope): string {
  return `When a session in ${scopeName(scope)} starts, finishes a turn, asks something or has its pull requests ready for review, fctl posts it to an address of yours as it happens, so that Slack, a deploy script or a service of your own acts on it.`;
}

function Intro({
  scope,
  onAdd,
  full,
  titled,
}: {
  scope: HookScope;
  onAdd: () => void;
  full: boolean;
  titled?: boolean;
}) {
  const where = scopeName(scope);
  const workspace = scope.kind === "workspace" ? scope.workspace : scope.project.workspace;
  return (
    <div className="flex animate-rise flex-col gap-10 py-2">
      <div className="grid items-center gap-x-12 gap-y-10 md:grid-cols-[minmax(0,1fr)_320px]">
        <div>
          {!titled && (
            <>
              <h2 className="font-display text-[28px] leading-[1.15] font-medium tracking-[-0.018em] text-ink">
                What agents do, posted to your tools
              </h2>
              <p className="mt-3 mb-6 max-w-xl text-[15px] leading-relaxed text-ink-3">
                {webhooksLead(scope)}
              </p>
            </>
          )}
          <ul className="flex max-w-xl flex-col gap-3">
            {POINTS.map((p) => (
              <li key={p.key} className="flex gap-3 text-[13.5px] leading-relaxed text-ink-2">
                <span aria-hidden="true" className="mt-[3px] shrink-0 text-ink-3 [&_svg]:size-4">
                  {p.icon}
                </span>
                <span className="min-w-0">{p.text(where)}</span>
              </li>
            ))}
          </ul>
          <div className="mt-8 flex flex-wrap items-center gap-x-4 gap-y-3">
            <Button
              variant="primary"
              size="lg"
              icon={<Plus className="size-4" />}
              onClick={onAdd}
              disabled={full}
            >
              Add a webhook
            </Button>
            <p className="max-w-sm text-[12.5px] leading-snug text-ink-3">
              {full
                ? `${workspace?.name ?? "The workspace"} has as many webhooks as it may have.`
                : scope.kind === "workspace"
                  ? `${where}'s admins add webhooks for all its projects, or some; each project's owners and maintainers add its own.`
                  : `Its owners and maintainers add its own; ${workspace?.name ?? "the workspace"}'s admins add ones for every project.`}
            </p>
          </div>
        </div>
        <Preview
          workspace={workspace?.slug ?? "acme"}
          project={scope.kind === "project" ? scope.project.slug : "web"}
        />
      </div>
    </div>
  );
}

/**
 * What fctl posts, drawn: a request as it goes out, its headers and the start of its payload, and
 * under it the log it lands in, one delivery that failed and went again. Made up, in the world the
 * app's previews share (an upload worker's retries), with this workspace's and project's slugs.
 */
function Preview({ workspace, project }: { workspace: string; project: string }) {
  const card = "overflow-hidden rounded-xl border border-line bg-surface";
  const key = "text-blue";
  const str = "text-ink-2";
  return (
    <div aria-hidden="true" className="relative select-none max-md:max-w-sm">
      <div className={cn(card, "shadow-[0_18px_50px_-24px_rgb(0_0_0/0.35)]")}>
        <div className="flex items-center gap-2 border-b border-line px-3.5 py-2.5">
          <span className="rounded bg-raised px-1.5 py-0.5 font-mono text-[10.5px] font-semibold text-ink-2">
            POST
          </span>
          <span className="min-w-0 flex-1 truncate font-mono text-[11.5px] text-ink-2">
            hooks.example.com/fctl
          </span>
          <span className="inline-flex items-center gap-1 font-mono text-[11px] text-checks-passed">
            <CircleCheck className="size-3" />
            200
          </span>
        </div>
        <div className="border-b border-line px-3.5 py-2 font-mono text-[10.5px] leading-[1.7] text-ink-4">
          <div>
            fctl-event: <span className="text-ink-2">session.ready_for_review</span>
          </div>
          <div>
            webhook-signature: <span className="text-ink-2">v1,K5oZfzN95Z9UVu1…</span>
          </div>
        </div>
        <pre className="m-0 overflow-hidden px-3.5 py-2.5 font-mono text-[10.5px] leading-[1.65] text-ink-4">
          {"{\n  "}
          <span className={key}>"type"</span>
          {": "}
          <span className={str}>"session.ready_for_review"</span>
          {",\n  "}
          <span className={key}>"workspace"</span>
          {": { "}
          <span className={key}>"slug"</span>
          {": "}
          <span className={str}>"{workspace}"</span>
          {" },\n  "}
          <span className={key}>"data"</span>
          {": {\n    "}
          <span className={key}>"session"</span>
          {": {\n      "}
          <span className={key}>"ref"</span>
          {": "}
          <span className={str}>"{project}#12"</span>
          {",\n      "}
          <span className={key}>"title"</span>
          {": "}
          <span className={str}>"Retry failed uploads…"</span>
          {"\n    },\n    "}
          <span className={key}>"ready"</span>
          {": [{ "}
          <span className={key}>"number"</span>
          {": "}
          <span className="text-pink">45</span>
          {" }]\n  }\n}"}
        </pre>
      </div>
      <div className={cn(card, "mt-3 shadow-[0_18px_50px_-30px_rgb(0_0_0/0.35)]")}>
        <div className="flex flex-col gap-1.5 px-3.5 py-2.5 text-[11.5px]">
          {[
            ["session.ready_for_review", `${project}#12 is ready for review`, "200", true],
            ["review.finished", `Review of #45 found 1 issue`, "200", true],
            ["session.question", `${project}#12 asks: Ship it?`, "503 → 200", false],
          ].map(([event, gist, status, first]) => (
            <div key={event as string} className="flex items-center gap-2">
              <CircleCheck className="size-3 shrink-0 text-checks-passed" />
              <span className="min-w-0 flex-1 truncate text-ink-2">{gist}</span>
              <span className={cn("shrink-0 font-mono", first ? "text-ink-4" : "text-amber")}>{status}</span>
            </div>
          ))}
        </div>
      </div>
    </div>
  );
}

/** A webhook just added: its secret, shown once here, and how a receiver checks what fctl posts. */
function Created({
  created,
  scope,
  onOpen,
  onDone,
}: {
  created: G.CreatedWebhook;
  scope: HookScope;
  onOpen: () => void;
  onDone: () => void;
}) {
  const w = created.webhook;
  const workspace =
    scope.kind === "workspace" ? scope.workspace.id : (scope.project.workspace?.id ?? w.workspace_id);
  return (
    <section
      aria-labelledby="created-heading"
      className="flex animate-rise flex-col gap-5 rounded-2xl border border-line bg-surface/70 p-4 shadow-[0_24px_60px_-40px_rgb(0_0_0/0.5)] sm:p-5"
    >
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex size-8 shrink-0 items-center justify-center rounded-full bg-accent-soft text-accent">
          <KeyRound className="size-4" aria-hidden="true" />
        </span>
        <div className="min-w-0">
          <h3 id="created-heading" className="text-[15px] font-semibold text-ink">
            Added {webhookTitle(w)}
          </h3>
          <p className="mt-1 text-[13px] leading-relaxed text-ink-3">
            Its first events go to {hostOf(w.url)} within half a minute.
          </p>
        </div>
      </div>
      <Banner tone="warn">
        Copy its signing secret now: it shows once here, and later only to those who manage the webhook, on
        its page. Whoever has it can sign requests as fctl, to your receiver.
      </Banner>
      <CodeBox value={created.secret} />
      <VerifyHow workspaceId={workspace} projectId={w.project?.id} />
      <div className="flex flex-wrap items-center justify-end gap-2 border-t border-line pt-4">
        <Button variant="ghost" onClick={onDone}>
          Done
        </Button>
        <Button variant="primary" icon={<ArrowRight className="size-3.5" />} onClick={onOpen}>
          Send it a test
        </Button>
      </div>
    </section>
  );
}

// A webhook's page: what it posts where and how it stands, what's wrong when something is (off,
// failing, events dropped) with the way out beside it, what went out to it (each request as fctl
// posted it, every attempt and the start of the answer, with Resend), a test of any event, its
// secret, and deleting it.

import { useInfiniteQuery, useMutation, useQuery } from "@tanstack/react-query";
import {
  ChevronDown,
  ChevronLeft,
  Ellipsis,
  FlaskConical,
  Pencil,
  Power,
  PowerOff,
  RotateCcw,
  Send,
  Trash2,
} from "lucide-react";
import { useState } from "react";
import { CodeBlock } from "@/components/markdown";
import {
  Avatar,
  Banner,
  Button,
  DangerZone,
  Dialog,
  IconButton,
  Menu,
  MenuItem,
  MenuSeparator,
  Section,
  SectionLayout,
  Select,
  type SelectOption,
  Skeleton,
  Spinner,
  toast,
} from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { agoShortWords, stamp, when } from "@/lib/format";
import { keys } from "@/lib/queries";
import {
  againIn,
  attemptWords,
  type DeliveryFilter,
  delivered,
  describeWebhook,
  hookState,
  pretty,
  statusText,
  webhookTitle,
} from "@/lib/webhooks";
import { type HookScope, refreshWebhooks, WebhookForm } from "./form";
import { DeliveryMark, deliveryWords, HookGlyph, StatePill, StatusCode, useEventCatalog } from "./parts";
import { SecretSection, VerifyHow } from "./secret";

export function WebhookPage({
  id,
  scope,
  projects,
  startTest,
  onBack,
}: {
  id: string;
  scope: HookScope;
  projects: G.Project[];
  /** It opens on Send a test: a webhook just added. */
  startTest?: boolean;
  onBack: () => void;
}) {
  const hook = useQuery({
    queryKey: keys.webhook(id),
    queryFn: () => api.webhook(id),
    refetchInterval: 15_000,
  });
  const catalog = useEventCatalog();
  const [editing, setEditing] = useState(false);
  const [testing, setTesting] = useState(!!startTest);
  const [deleting, setDeleting] = useState(false);
  const toggle = useMutation({
    mutationFn: (enabled: boolean) => api.updateWebhook(id, { enabled }),
    onSuccess: (w) => {
      refreshWebhooks(id);
      toast(w.enabled ? "On: it gets the next events it wants" : "Off: it gets nothing until it's turned on");
    },
    onError: (e) => toast(e.message, "error"),
  });
  const back = (
    <button
      type="button"
      onClick={onBack}
      className="-ml-1.5 inline-flex items-center gap-1 rounded-md px-1.5 py-1 text-[13px] text-ink-3 transition-colors hover:bg-hover hover:text-ink"
    >
      <ChevronLeft className="size-4" aria-hidden="true" />
      Webhooks
    </button>
  );
  if (hook.isError)
    return (
      <div className="flex flex-col items-start gap-4">
        {back}
        <Banner tone="error" className="self-stretch">
          {hook.error.message}
        </Banner>
      </div>
    );
  if (!hook.data || !catalog.data)
    return (
      <div className="flex flex-col gap-4">
        {back}
        <Skeleton className="h-16" />
        <Skeleton className="h-40" />
      </div>
    );
  const w = hook.data;
  const events = catalog.data.events;
  const state = hookState(w);
  const names = projects.filter((p) => w.projects.includes(p.id)).map((p) => p.name);
  const workspaceId = w.workspace_id;
  return (
    <div className="flex animate-rise flex-col gap-8">
      <div className="flex flex-col gap-4">
        {back}
        {editing ? (
          <WebhookForm
            scope={scope}
            webhook={w}
            catalog={events}
            groups={catalog.data.groups}
            projects={projects}
            onDone={(r) => {
              setEditing(false);
              if (r?.saved) toast("Saved: new events go by it within half a minute");
            }}
          />
        ) : (
          // On a phone its buttons go under it, rather than squeeze its name into a column.
          <header className="flex flex-wrap items-start gap-x-4 gap-y-3">
            <HookGlyph state={state} turnedOff={!!w.disabled_reason} />
            <div className="min-w-0 flex-1 max-sm:basis-[calc(100%-3rem)]">
              <div className="flex flex-wrap items-center gap-2">
                <h2 className="min-w-0 font-display text-[22px] leading-tight font-medium tracking-[-0.015em] text-ink [overflow-wrap:anywhere]">
                  {webhookTitle(w)}
                </h2>
                <StatePill state={state} />
              </div>
              <p className="mt-1 font-mono text-[12.5px] text-ink-3 [overflow-wrap:anywhere]">{w.url}</p>
              <p className="mt-2 max-w-2xl text-[13.5px] leading-relaxed text-ink-2">
                {describeWebhook(w.url, w.events, names, w.started_by, events)}
              </p>
              <p className="mt-1.5 flex flex-wrap items-center gap-x-1.5 text-[12px] text-ink-4">
                <span>{w.project ? `${w.project.name}'s own` : "The workspace's"}</span>
                {w.created_by && (
                  <>
                    <span aria-hidden="true">·</span>
                    <span className="inline-flex items-center gap-1">
                      added by <Avatar user={w.created_by} size={14} /> {w.created_by.name}
                    </span>
                  </>
                )}
                <span aria-hidden="true">·</span>
                <span title={stamp(w.created_at)}>{agoShortWords(w.created_at)}</span>
              </p>
            </div>
            <div className="flex shrink-0 items-center gap-1.5 max-sm:w-full max-sm:pl-12">
              <Button size="sm" icon={<FlaskConical className="size-3.5" />} onClick={() => setTesting(true)}>
                Send a test
              </Button>
              <Menu
                trigger={(open, menuOpen) => (
                  <IconButton
                    label="Webhook actions"
                    aria-haspopup="menu"
                    aria-expanded={menuOpen}
                    onClick={open}
                  >
                    <Ellipsis className="size-4" />
                  </IconButton>
                )}
              >
                {(close) => (
                  <>
                    <MenuItem
                      icon={<Pencil />}
                      onClick={() => {
                        close();
                        setEditing(true);
                      }}
                    >
                      Edit
                    </MenuItem>
                    <MenuItem
                      icon={w.enabled ? <PowerOff /> : <Power />}
                      onClick={() => {
                        close();
                        toggle.mutate(!w.enabled);
                      }}
                    >
                      {w.enabled ? "Turn off" : "Turn on"}
                    </MenuItem>
                    <MenuSeparator />
                    <MenuItem
                      danger
                      icon={<Trash2 />}
                      onClick={() => {
                        close();
                        setDeleting(true);
                      }}
                    >
                      Delete…
                    </MenuItem>
                  </>
                )}
              </Menu>
            </div>
          </header>
        )}
        <Problems webhook={w} onTurnOn={() => toggle.mutate(true)} turningOn={toggle.isPending} />
      </div>

      <Deliveries webhook={w} catalog={events} onTest={() => setTesting(true)} />

      <SectionLayout.Provider value="part">
        <Section
          title="Signing secret"
          description="fctl signs each request with it (webhook-signature), so that your receiver knows it's fctl's. Keep it where your receiver reads it, and nowhere else."
        >
          <div className="flex flex-col gap-6">
            <SecretSection webhook={w} />
            <VerifyHow workspaceId={workspaceId} projectId={w.project?.id} />
          </div>
        </Section>
      </SectionLayout.Provider>
      <DangerZone
        title="Delete this webhook"
        description="fctl posts nothing more to it, and forgets what went out to it and its secret. The sessions, reviews and schedules it heard of stay as they are."
        action="Delete…"
        onAction={() => setDeleting(true)}
      />

      <TestDialog webhook={w} catalog={events} open={testing} onClose={() => setTesting(false)} />
      <DeleteDialog webhook={w} open={deleting} onClose={() => setDeleting(false)} onDeleted={onBack} />
    </div>
  );
}

/** What's wrong with a webhook, when something is, with the way out beside it. */
function Problems({
  webhook: w,
  onTurnOn,
  turningOn,
}: {
  webhook: G.Webhook;
  onTurnOn: () => void;
  turningOn: boolean;
}) {
  const h = w.health;
  const resend = useMutation({
    mutationFn: () => api.resendFailedWebhookDeliveries(w.id, {}),
    onSuccess: (r) => {
      refreshWebhooks(w.id);
      toast(
        r.queued === 0
          ? "Nothing failed in the last day"
          : `Resending ${r.queued} ${r.queued === 1 ? "delivery" : "deliveries"}: they go in their turn`,
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <>
      {w.disabled_reason && (
        <Banner tone="warn">
          <span className="block">
            fctl turned it off {w.disabled_at ? when(w.disabled_at) : ""}. {w.disabled_reason}
          </span>
          <span className="mt-2 flex flex-wrap gap-2">
            <Button size="sm" variant="primary" loading={turningOn} onClick={onTurnOn}>
              Turn it on
            </Button>
          </span>
        </Banner>
      )}
      {!w.enabled && !w.disabled_reason && (
        <Banner tone="info">
          It's off: it gets nothing, and what happens meanwhile doesn't wait for it.
        </Banner>
      )}
      {w.enabled && h?.failing_since && (
        <Banner tone="warn">
          <span className="block">
            fctl's attempts have all failed since {when(h.failing_since)}
            {h.last ? `, the last with ${attemptWords(h.last)}` : ""}. It tries each delivery again 7 times
            over about 21 hours, and turns the webhook off once they have all failed for 3 days.
          </span>
          {h.failed > 0 && (
            <span className="mt-2 flex flex-wrap items-center gap-3">
              <Button
                size="sm"
                icon={<RotateCcw className="size-3.5" />}
                loading={resend.isPending}
                onClick={() => resend.mutate()}
              >
                Resend the ones that failed
              </Button>
              <span className="text-[12.5px] text-ink-3">Once your receiver works again.</span>
            </span>
          )}
        </Banner>
      )}
      {h && h.dropped > 0 && (
        <Banner tone="warn">
          {h.dropped} {h.dropped === 1 ? "event was" : "events were"} dropped since it last took one: 1,000
          deliveries waited for it already, so {h.dropped === 1 ? "it won't" : "they won't"} be sent.
        </Banner>
      )}
    </>
  );
}

const FILTERS: { value: DeliveryFilter; label: string }[] = [
  { value: "all", label: "All" },
  { value: "failed", label: "Failed" },
  { value: "pending", label: "Waiting" },
  { value: "delivered", label: "Delivered" },
];

/** What went out to a webhook in the last 30 days, newest first, narrowed down by state or event. */
function Deliveries({
  webhook: w,
  catalog,
  onTest,
}: {
  webhook: G.Webhook;
  catalog: G.WebhookEventInfo[];
  onTest: () => void;
}) {
  const [filter, setFilter] = useState<DeliveryFilter>("all");
  const [event, setEvent] = useState<string>("");
  const [open, setOpen] = useState<string>();
  const q = { state: filter === "all" ? undefined : filter, event: event || undefined };
  const list = useInfiniteQuery({
    queryKey: keys.webhookDeliveries(w.id, q),
    queryFn: ({ pageParam }) => api.webhookDeliveries(w.id, { ...q, before: pageParam, limit: 20 }),
    initialPageParam: undefined as string | undefined,
    getNextPageParam: (last) => last.next,
    refetchInterval: 10_000,
  });
  const deliveries = list.data?.pages.flatMap((p) => p.deliveries) ?? [];
  const h = w.health;
  const eventOptions: SelectOption[] = [
    { value: "", label: "Every event" },
    ...catalog.map((e) => ({ value: e.type, label: e.type, description: e.label })),
  ];
  return (
    <section aria-labelledby="deliveries-heading" className="flex flex-col gap-4">
      <div className="flex flex-wrap items-end justify-between gap-x-6 gap-y-3">
        {/* The summary takes a line for each sentence beside the filters, before they move below it. */}
        <div className="min-w-0 flex-1 basis-64">
          <h3 id="deliveries-heading" className="text-[15px] font-semibold tracking-[-0.005em] text-ink">
            Deliveries
          </h3>
          <p className="mt-1 text-[13px] leading-relaxed text-pretty text-ink-3">
            {h ? (
              <>
                {`${h.delivered} delivered${h.failed ? `, ${h.failed} failed` : ""} in the last day${h.pending ? `, ${h.pending} waiting now` : ""}.`}{" "}
                <span className="block">Its newest 1,000 of the last 30 days are kept.</span>
              </>
            ) : (
              "What went out to it in the last 30 days, its newest 1,000 at most."
            )}
          </p>
        </div>
        <div className="flex flex-wrap items-center gap-2">
          <div
            role="radiogroup"
            aria-label="Show"
            className="inline-flex rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)]"
          >
            {FILTERS.map((f) => (
              // biome-ignore lint/a11y/useSemanticElements: a segment of the pill, as a radio of the group.
              <button
                key={f.value}
                type="button"
                role="radio"
                aria-checked={filter === f.value}
                onClick={() => setFilter(f.value)}
                className={cn(
                  "h-7 rounded-full px-3 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
                  filter === f.value
                    ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25),inset_0_0_0_1px_var(--c-line-strong)]"
                    : "text-ink-3 hover:text-ink",
                )}
              >
                {f.label}
              </button>
            ))}
          </div>
          <Select
            size="sm"
            aria-label="Of the event"
            value={event}
            options={eventOptions}
            onChange={setEvent}
          />
        </div>
      </div>
      {list.isError ? (
        <Banner tone="error">{list.error.message}</Banner>
      ) : !list.data ? (
        <div className="space-y-2">
          <Skeleton className="h-11" />
          <Skeleton className="h-11" />
          <Skeleton className="h-11" />
        </div>
      ) : deliveries.length === 0 ? (
        <div className="rounded-xl border border-dashed border-line px-5 py-8 text-center">
          <p className="text-[14px] text-ink">
            {filter === "all" && !event
              ? "Nothing went out to it yet"
              : "No deliveries like that in the last 30 days"}
          </p>
          <p className="mx-auto mt-1 max-w-md text-[13px] leading-relaxed text-ink-3">
            {filter === "all" && !event
              ? "The next event it wants shows here as fctl posts it, with the answer it got. Send a test to see one now."
              : "Choose All, or every event, to see the rest."}
          </p>
          {filter === "all" && !event && (
            <Button size="sm" className="mt-4" icon={<FlaskConical className="size-3.5" />} onClick={onTest}>
              Send a test
            </Button>
          )}
        </div>
      ) : (
        <ul className="border-t border-line">
          {deliveries.map((d) => (
            <DeliveryRow
              key={d.id}
              webhook={w}
              delivery={d}
              open={open === d.id}
              onToggle={() => setOpen(open === d.id ? undefined : d.id)}
            />
          ))}
        </ul>
      )}
      {list.hasNextPage && (
        <div className="flex justify-center">
          <Button
            size="sm"
            variant="ghost"
            loading={list.isFetchingNextPage}
            onClick={() => void list.fetchNextPage()}
          >
            Show older
          </Button>
        </div>
      )}
    </section>
  );
}

function DeliveryRow({
  webhook: w,
  delivery: d,
  open,
  onToggle,
}: {
  webhook: G.Webhook;
  delivery: G.WebhookDelivery;
  open: boolean;
  onToggle: () => void;
}) {
  const panel = `delivery-${d.id}`;
  const facts = [
    d.type,
    d.attempts > 1 ? `${d.attempts} attempts` : undefined,
    d.state === "pending" && d.next_at ? `goes again ${againIn(d.next_at)}` : undefined,
    d.resends > 0 ? `resent ${d.resends === 1 ? "once" : `${d.resends} times`}` : undefined,
  ].filter(Boolean);
  return (
    <li className="border-b border-line">
      <button
        type="button"
        aria-expanded={open}
        aria-controls={panel}
        onClick={onToggle}
        className={cn(
          "flex w-full items-start gap-3 px-3 py-2.5 text-left transition-colors focus-visible:bg-hover/30 focus-visible:outline-none pointer-coarse:py-3",
          open ? "bg-hover/20" : "hover:bg-hover/30",
        )}
      >
        <span className="mt-0.5" title={deliveryWords(d)}>
          <DeliveryMark delivery={d} />
          <span className="sr-only">{deliveryWords(d)}</span>
        </span>
        <span className="min-w-0 flex-1">
          <span className="flex items-center gap-2">
            <span className="min-w-0 truncate text-[13.5px] text-ink">{d.gist}</span>
            {d.test && (
              <span className="shrink-0 rounded-full bg-raised px-1.5 text-[11px] text-ink-3">test</span>
            )}
          </span>
          <span className="mt-0.5 block font-mono text-[11.5px] text-ink-4">{facts.join(" · ")}</span>
        </span>
        <span className="flex shrink-0 items-center gap-3">
          <StatusCode status={d.last?.status} />
          <span
            className="w-16 text-right text-[12px] text-ink-4 tabular-nums max-sm:hidden"
            title={stamp(d.created_at)}
          >
            {agoShortWords(d.created_at)}
          </span>
          <ChevronDown
            className={cn("size-3.5 text-ink-4 transition-transform duration-200", open && "rotate-180")}
            aria-hidden="true"
          />
        </span>
      </button>
      {open && (
        <div id={panel} className="animate-rise px-3 pt-1 pb-5 sm:pl-[44px]">
          <DeliveryDetail webhook={w} id={d.id} />
        </div>
      )}
    </li>
  );
}

/** One delivery in full: its attempts, the request as fctl posted it and the last answer. */
function DeliveryDetail({ webhook: w, id }: { webhook: G.Webhook; id: string }) {
  const d = useQuery({
    queryKey: keys.webhookDelivery(w.id, id),
    queryFn: () => api.webhookDelivery(w.id, id),
  });
  const [view, setView] = useState<"request" | "response">("request");
  const resend = useMutation({
    mutationFn: () => api.resendWebhookDelivery(w.id, id),
    onSuccess: (r) => {
      refreshWebhooks(w.id);
      d.refetch();
      toast(
        delivered(r.last)
          ? `Resent: ${r.last ? attemptWords(r.last) : "delivered"}`
          : `Resent, and it failed: ${r.last ? attemptWords(r.last) : "no answer"}`,
        delivered(r.last) ? "ok" : "error",
      );
    },
    onError: (e) => toast(e.message, "error"),
  });
  if (d.isError) return <Banner tone="error">{d.error.message}</Banner>;
  if (!d.data)
    return (
      <div className="flex items-center gap-2 py-3 text-[13px] text-ink-3">
        <Spinner /> Reading it
      </div>
    );
  const x = d.data;
  const response = x.last?.response;
  const resendWhy = x.test
    ? "A test isn't resent: send another one."
    : x.resends >= 5
      ? "It was resent 5 times, the most a delivery is."
      : undefined;
  return (
    <div className="flex flex-col gap-4">
      <ol className="flex flex-col gap-1.5" aria-label="Attempts, newest first">
        {x.history.length === 0 && (
          <li className="text-[12.5px] text-ink-3">
            {x.last?.error ??
              (x.next_at ? `Its first attempt goes ${againIn(x.next_at)}.` : "No attempt yet.")}
          </li>
        )}
        {x.history.map((a, i) => (
          <li key={`${a.at}-${i}`} className="flex flex-wrap items-baseline gap-x-2 text-[12.5px]">
            <span
              className={cn(
                "size-1.5 shrink-0 translate-y-[-1px] self-center rounded-full",
                delivered(a) ? "bg-checks-passed" : "bg-danger",
              )}
              aria-hidden="true"
            />
            <span className="text-ink-4 tabular-nums" title={stamp(a.at)}>
              {when(a.at)}
            </span>
            <span className={delivered(a) ? "text-ink-2" : "text-danger"}>{attemptWords(a)}</span>
            {a.by && (
              <span className="text-ink-4">
                · {x.test ? "sent" : "resent"} by {a.by.name}
              </span>
            )}
          </li>
        ))}
        {x.state === "pending" && x.next_at && x.history.length > 0 && (
          <li className="text-[12.5px] text-amber">fctl tries it again {againIn(x.next_at)}.</li>
        )}
      </ol>
      <div>
        <div className="mb-2 flex flex-wrap items-center justify-between gap-2">
          <div
            role="tablist"
            aria-label="Show"
            className="inline-flex rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)]"
          >
            {(["request", "response"] as const).map((v) => (
              <button
                key={v}
                type="button"
                role="tab"
                aria-selected={view === v}
                onClick={() => setView(v)}
                className={cn(
                  "h-7 rounded-full px-3 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
                  view === v
                    ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25),inset_0_0_0_1px_var(--c-line-strong)]"
                    : "text-ink-3 hover:text-ink",
                )}
              >
                {v === "request" ? "Request" : "Answer"}
              </button>
            ))}
          </div>
          <Button
            size="sm"
            icon={<Send className="size-3.5" />}
            loading={resend.isPending}
            disabled={!!resendWhy}
            title={resendWhy ?? "Post it again now, as it was"}
            onClick={() => resend.mutate()}
          >
            Resend
          </Button>
        </div>
        {view === "request" ? (
          x.request ? (
            <div className="flex flex-col gap-2">
              <p className="font-mono text-[12px] text-ink-2 [overflow-wrap:anywhere]">
                POST {x.request.url || w.url}
              </p>
              {x.request.headers.length > 0 && (
                <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 rounded-md border border-line bg-sunken/50 px-3 py-2 font-mono text-[11.5px]">
                  {x.request.headers.map((h) => (
                    <div key={h.name} className="contents">
                      <dt className="text-ink-4">{h.name}</dt>
                      <dd className="text-ink-2 [overflow-wrap:anywhere]">{h.value}</dd>
                    </div>
                  ))}
                </dl>
              )}
              <CodeBlock code={pretty(x.request.body)} lang="json" label="JSON" />
            </div>
          ) : (
            <p className="text-[12.5px] text-ink-3">Not posted yet.</p>
          )
        ) : response ? (
          <div className="flex flex-col gap-2">
            <p className="font-mono text-[12px] text-ink-2">
              {x.last?.status ? statusText(x.last.status) : "No status"} · {x.last?.ms} ms
            </p>
            {response.headers.length > 0 && (
              <dl className="grid grid-cols-[auto_minmax(0,1fr)] gap-x-3 gap-y-0.5 rounded-md border border-line bg-sunken/50 px-3 py-2 font-mono text-[11.5px]">
                {response.headers.map((h, i) => (
                  <div key={`${h.name}-${i}`} className="contents">
                    <dt className="text-ink-4">{h.name}</dt>
                    <dd className="text-ink-2 [overflow-wrap:anywhere]">{h.value}</dd>
                  </div>
                ))}
              </dl>
            )}
            {response.body ? (
              <CodeBlock
                code={pretty(response.body)}
                lang={response.body.trim().startsWith("{") ? "json" : "text"}
              />
            ) : (
              <p className="text-[12.5px] text-ink-3">It answered with no body.</p>
            )}
            {response.truncated && <p className="text-[12px] text-ink-4">Its first 4 KB.</p>}
          </div>
        ) : (
          <p className="text-[12.5px] text-ink-3">
            {x.last?.error ? `No answer came: ${x.last.error}` : "No answer yet."}
          </p>
        )}
      </div>
    </div>
  );
}

/** Posts a made-up event of the kind chosen, now, and shows the answer. */
function TestDialog({
  webhook: w,
  catalog,
  open,
  onClose,
}: {
  webhook: G.Webhook;
  catalog: G.WebhookEventInfo[];
  open: boolean;
  onClose: () => void;
}) {
  const wanted = w.events.length ? catalog.filter((e) => w.events.includes(e.type)) : catalog;
  const [event, setEvent] = useState(wanted[0]?.type ?? "session.finished");
  const test = useMutation({
    mutationFn: () => api.testWebhook(w.id, { event }),
    onSuccess: () => refreshWebhooks(w.id),
  });
  const r = test.data;
  const options: SelectOption[] = catalog.map((e) => ({
    value: e.type,
    label: e.label,
    description: w.events.length && !w.events.includes(e.type) ? `${e.type} · it doesn't get these` : e.type,
  }));
  return (
    <Dialog
      open={open}
      onClose={() => {
        test.reset();
        onClose();
      }}
      title="Send a test"
      description={`fctl posts a made-up event of the kind you choose to ${w.url}, now, marked as a test: its fctl-test header, and "test": true in its payload.`}
    >
      <div className="flex flex-col gap-4">
        <div className="flex flex-wrap items-center gap-2">
          <div className="min-w-0 flex-1">
            <Select aria-label="Event" value={event} options={options} onChange={setEvent} />
          </div>
          <Button
            variant="primary"
            icon={<Send className="size-3.5" />}
            loading={test.isPending}
            onClick={() => test.mutate()}
          >
            Send
          </Button>
        </div>
        {test.error && <Banner tone="error">{test.error.message}</Banner>}
        {r && (
          <div className="flex flex-col gap-2" aria-live="polite">
            <p className={cn("text-[13.5px]", delivered(r.last) ? "text-ink" : "text-danger")}>
              {delivered(r.last)
                ? `Delivered: ${r.last ? attemptWords(r.last) : ""}.`
                : `Failed: ${r.last ? attemptWords(r.last) : "no answer"}.`}
            </p>
            {r.last?.response?.body && (
              <CodeBlock
                code={pretty(r.last.response.body)}
                lang={r.last.response.body.trim().startsWith("{") ? "json" : "text"}
              />
            )}
            <p className="text-[12.5px] text-ink-3">
              It's in the log below, with the request as fctl posted it.
            </p>
          </div>
        )}
      </div>
    </Dialog>
  );
}

function DeleteDialog({
  webhook: w,
  open,
  onClose,
  onDeleted,
}: {
  webhook: G.Webhook;
  open: boolean;
  onClose: () => void;
  onDeleted: () => void;
}) {
  const remove = useMutation({
    mutationFn: () => api.deleteWebhook(w.id),
    onSuccess: () => {
      refreshWebhooks();
      toast(`Deleted ${webhookTitle(w)}: fctl posts nothing more to it`);
      onClose();
      onDeleted();
    },
    onError: (e) => toast(e.message, "error"),
  });
  return (
    <Dialog
      open={open}
      onClose={onClose}
      title={`Delete ${webhookTitle(w)}?`}
      description="fctl posts nothing more to it, and forgets what went out to it and its secret. What's waiting for it goes nowhere. Turning it off instead keeps all of it."
    >
      <div className="flex justify-end gap-2">
        <Button variant="ghost" onClick={onClose}>
          Cancel
        </Button>
        <Button variant="danger" loading={remove.isPending} onClick={() => remove.mutate()}>
          Delete
        </Button>
      </div>
    </Dialog>
  );
}

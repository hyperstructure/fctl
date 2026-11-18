// What the webhooks' pages share: the catalog of events, how a webhook shows where it stands (its
// glyph and pill), and how a delivery or an attempt shows how it went.

import { useQuery } from "@tanstack/react-query";
import {
  CircleCheck,
  CircleDashed,
  CircleX,
  Clock3,
  TriangleAlert,
  Webhook as WebhookIcon,
} from "lucide-react";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { keys } from "@/lib/queries";
import { type HookState, STATE_WORDS } from "@/lib/webhooks";

/** Every event webhooks get: the same for everyone, so read once. */
export function useEventCatalog() {
  return useQuery({
    queryKey: keys.webhookEvents,
    queryFn: api.webhookEvents,
    staleTime: Number.POSITIVE_INFINITY,
  });
}

/**
 * A webhook's symbol, in the tone of how it stands: a green dot while it takes what it gets, amber
 * with a warning while it fails or once fctl turned it off, grey while off.
 */
export function HookGlyph({ state, turnedOff }: { state: HookState; turnedOff?: boolean }) {
  const trouble = state === "failing" || turnedOff;
  return (
    <span
      aria-hidden="true"
      className={cn(
        "relative mt-px flex size-8 shrink-0 items-center justify-center rounded-lg bg-raised shadow-[inset_0_0_0_1px_var(--c-line)] [&_svg]:size-4",
        trouble ? "text-amber" : state === "off" ? "text-ink-4" : "text-ink",
      )}
    >
      {trouble ? <TriangleAlert /> : <WebhookIcon />}
      {state === "ok" && (
        <span className="absolute -top-0.5 -right-0.5 size-2 rounded-full bg-checks-passed ring-2 ring-canvas" />
      )}
    </span>
  );
}

/** How a webhook stands, in a word, beside its name. */
export function StatePill({ state }: { state: HookState }) {
  if (state === "ok" || state === "new") return null;
  return (
    <span
      className={cn(
        "inline-flex h-5 shrink-0 items-center rounded-full px-2 text-[11.5px] font-medium",
        state === "failing" ? "bg-amber-soft text-amber" : "bg-raised text-ink-3",
      )}
    >
      {STATE_WORDS[state]}
    </span>
  );
}

/** How a delivery went, as a mark: taken, failed, waiting for a retry or for its first attempt. */
export function DeliveryMark({
  delivery: d,
  className,
}: {
  delivery: G.WebhookDelivery;
  className?: string;
}) {
  const c = cn("size-4 shrink-0", className);
  if (d.state === "delivered")
    return <CircleCheck className={cn(c, "text-checks-passed")} aria-hidden="true" />;
  if (d.state === "failed") return <CircleX className={cn(c, "text-danger")} aria-hidden="true" />;
  if (d.attempts > 0) return <Clock3 className={cn(c, "text-amber")} aria-hidden="true" />;
  return <CircleDashed className={cn(c, "text-ink-4")} aria-hidden="true" />;
}

/** The same, in words, for screen readers and titles. */
export function deliveryWords(d: G.WebhookDelivery): string {
  if (d.state === "delivered") return "Delivered";
  if (d.state === "failed") return "Failed";
  if (d.attempts > 0) return "Failed, waiting to go again";
  return "Waiting for its first attempt";
}

/** An answer's status code, in its tone: green for 2xx, red for the rest. */
export function StatusCode({ status, className }: { status?: number; className?: string }) {
  if (status === undefined)
    return <span className={cn("font-mono text-[12px] text-ink-4 tabular-nums", className)}>---</span>;
  const ok = status >= 200 && status < 300;
  return (
    <span
      className={cn(
        "font-mono text-[12px] tabular-nums",
        ok ? "text-checks-passed" : "text-danger",
        className,
      )}
    >
      {status}
    </span>
  );
}

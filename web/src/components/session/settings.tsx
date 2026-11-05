// The model and effort level a session's Claude works at, which its owner changes beside the
// composer, while it runs or before it resumes. A new model reads the whole conversation again
// without Claude's prompt cache, which belongs to the model it leaves, so the page asks first; so it
// does for an effort level where Claude sends the level for the whole conversation rather than with
// each turn, which rewrites the cache too.

import { useMutation } from "@tanstack/react-query";
import { useState } from "react";
import { Banner, Button, Dialog, EffortIcon, Select, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { costsCache, effortLabel, modelLabel, switchEfforts, switchModels } from "@/lib/models";
import { patchSession, queryClient } from "@/lib/queries";

/** One change: of the model, or of the effort level. */
export type SettingsChange = { model: string; effort?: undefined } | { effort: string; model?: undefined };

// Each list is one section, whose label says what it lists.
const MODEL = { key: "model", label: "Model" };
const EFFORT = { key: "effort", label: "Effort" };

/** What the effort picker says: Extra high, the longest, as "Extra effort". */
const effortText = (e: string) =>
  e ? `${e === "xhigh" ? "Extra" : effortLabel(e)} effort` : "Default effort";

/**
 * On a phone the two pickers make one control, joined where their borders meet, as the start
 * form's do. The one in use comes to the front, so its border shows whole.
 */
const joined =
  "max-sm:hover:z-10 max-sm:focus-visible:z-10 max-sm:aria-expanded:z-10 max-sm:active:scale-100";

/**
 * The model and effort pickers of a session's composer, for its owner. On a phone they show the
 * model's name and the effort level's symbol, in one control.
 */
export function SessionChoices({
  session,
  perTurnEffort,
  className,
}: {
  session: G.Session;
  /** Whether an effort change keeps Claude's prompt cache, as Claude Code last said. */
  perTurnEffort?: boolean;
  className?: string;
}) {
  const [asking, setAsking] = useState<SettingsChange>();
  const change = useMutation({
    mutationFn: (c: SettingsChange) => api.updateSession(session.id, c),
    onSuccess: (updated) => {
      patchSession(queryClient, updated);
      setAsking(undefined);
    },
  });
  // What was picked shows while it is sent.
  const sending = change.isPending ? change.variables : undefined;
  const model = sending?.model ?? session.model ?? "";
  const effort = sending?.effort ?? session.effort ?? "";
  const send = (c: SettingsChange) =>
    change.mutate(c, { onError: (e) => !asking && toast(e.message, "error") });
  const pick = (c: SettingsChange) => {
    if (c.model !== undefined ? c.model === model : c.effort === effort) return;
    change.reset();
    if (costsCache(c.model !== undefined ? "model" : "effort", perTurnEffort)) setAsking(c);
    else send(c);
  };
  return (
    <div className={cn("flex min-w-0 items-center gap-1.5 max-sm:gap-0", className)}>
      <Select
        size="sm"
        narrow="bare"
        className={cn("min-w-0 max-sm:max-w-24 max-sm:rounded-r-none", joined)}
        aria-label="Model"
        value={model}
        onChange={(m) => pick({ model: m })}
        options={switchModels(model).map((o) => ({
          value: o.value,
          label: o.value ? o.label : "Default model",
          short: o.value ? o.label : "Default",
          item: o.label,
          section: MODEL,
        }))}
      />
      <Select
        size="sm"
        narrow="bare"
        className={cn("max-sm:-ml-px max-sm:shrink-0 max-sm:rounded-l-none", joined)}
        aria-label="Effort"
        value={effort}
        onChange={(e) => pick({ effort: e })}
        options={switchEfforts(effort).map((o) => ({
          value: o.value,
          label: effortText(o.value),
          short: "",
          item: o.label,
          icon: <EffortIcon level={o.value} />,
          section: EFFORT,
        }))}
      />
      <CacheDialog
        change={asking}
        status={session.status}
        pending={change.isPending}
        error={asking ? change.error?.message : undefined}
        onConfirm={() => asking && send(asking)}
        onClose={() => {
          setAsking(undefined);
          change.reset();
        }}
      />
    </div>
  );
}

/** Asks before a change that makes Claude read the conversation again without its prompt cache. */
function CacheDialog({
  change,
  status,
  pending,
  error,
  onConfirm,
  onClose,
}: {
  change?: SettingsChange;
  status: G.SessionStatus;
  pending: boolean;
  error?: string;
  onConfirm: () => void;
  onClose: () => void;
}) {
  const c = change;
  const name = c?.model !== undefined ? modelLabel(c.model) : effortLabel(c?.effort ?? "");
  const step =
    status === "stopped" || status === "stopping" || status === "failed" ? "first step" : "next step";
  return (
    <Dialog
      open={!!change}
      onClose={onClose}
      title={c?.model !== undefined ? `Switch to ${name}?` : `Change the effort level to ${name}?`}
      description={
        c?.model !== undefined
          ? `Claude's prompt cache holds the conversation for the model it works on now. At its ${step}, ${name} reads the whole conversation again without it, which takes longer and uses more of the plan's limits than reading from the cache.`
          : `On this model Claude sends the effort level for the whole conversation, not with each turn. At its ${step} it reads the whole conversation again without its prompt cache, which takes longer and uses more of the plan's limits than reading from the cache.`
      }
    >
      <div className="flex flex-col gap-5">
        {error && <Banner tone="error">{error}</Banner>}
        <div className="flex justify-end gap-2">
          <Button variant="ghost" onClick={onClose}>
            Cancel
          </Button>
          <Button variant="primary" loading={pending} onClick={onConfirm}>
            {c?.model !== undefined ? `Switch to ${name}` : `Change to ${name}`}
          </Button>
        </div>
      </div>
    </Dialog>
  );
}

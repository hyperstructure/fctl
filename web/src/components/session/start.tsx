// Starting a session from a prompt, on the home page and on a project's page, with the model
// and effort level it asks for.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ArrowUp } from "lucide-react";
import { type ReactNode, useCallback, useLayoutEffect, useRef, useState } from "react";
import { EffortIcon, Select, Spinner, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { type Choice, effortOffers, modelOffers, type Offer, startChoice } from "@/lib/models";
import { keys } from "@/lib/queries";
import type { SentFiles } from "./attachments";

/** The files a start form holds (see `useAttachments`). */
export interface StartFiles {
  drafts: readonly unknown[];
  take: () => Promise<{ files: SentFiles; restore: () => void }>;
}

/** The model and effort level a session asks for as it starts (see `asked`). */
export type StartAsks = Pick<G.CreateSessionRequest, "model" | "effort">;

/**
 * Starts a session and opens it. While a start is in flight, `start` does nothing, so a double
 * click or a second ⌘↵ cannot create a duplicate. `starting` stays true until the session page
 * replaces the form, and turns false again if the start fails (attached files go back in the
 * form). A start needs a prompt or files; it waits for files still uploading. `onStarted` runs
 * once the session exists, and the session page opens when what it returns is done.
 */
export function useStartSession(onStarted?: (s: G.Session) => unknown) {
  const navigate = useNavigate();
  const client = useQueryClient();
  // isPending reaches the component a tick after mutate(); this closes that gap.
  const inFlight = useRef(false);
  const create = useMutation({
    mutationFn: async ({
      project,
      prompt,
      files,
      asks,
    }: {
      project: string;
      prompt: string;
      files?: StartFiles;
      asks?: StartAsks;
    }) => {
      const taken = files?.drafts.length ? await files.take() : undefined;
      const attachments = taken?.files.attachments.map((a) => a.id) ?? [];
      const body: G.CreateSessionRequest = { prompt };
      if (attachments.length) body.attachments = attachments;
      if (asks?.model) body.model = asks.model;
      if (asks?.effort) body.effort = asks.effort;
      try {
        const s = await api.createSession(project, body);
        for (const url of Object.values(taken?.files.previews ?? {})) URL.revokeObjectURL(url);
        return s;
      } catch (e) {
        taken?.restore();
        throw e;
      }
    },
    onSuccess: async (s) => {
      // The project's composers start on what it asked for next time (`last_model`).
      for (const queryKey of [keys.projects, keys.project(s.project_slug), keys.project(s.project_id)])
        void client.invalidateQueries({ queryKey });
      await onStarted?.(s);
      return navigate({ to: "/s/$id", params: { id: s.id } });
    },
    onError: (e) => {
      inFlight.current = false;
      toast(e.message, "error");
    },
  });
  return {
    starting: create.isPending || create.isSuccess,
    start: (project: string, prompt: string, files?: StartFiles, asks?: StartAsks) => {
      if (inFlight.current || (!prompt.trim() && !files?.drafts.length)) return;
      inFlight.current = true;
      create.mutate({ project, prompt, files, asks });
    },
  };
}

type ChoiceProject = Parameters<typeof startChoice>[0];

/**
 * The model and effort level a start form shows for `project` (see `startChoice`), and `pick`
 * for the pickers. What was picked (`picked`, starting as `kept`) stays when the project changes,
 * as far as the new one allows it.
 */
export function useStartChoice(project: ChoiceProject | undefined, kept?: Partial<Choice>) {
  const [picked, setPicked] = useState<Partial<Choice>>(() => ({ model: kept?.model, effort: kept?.effort }));
  const pick = useCallback((c: Partial<Choice>) => setPicked((p) => ({ ...p, ...c })), []);
  const choice = project ? startChoice(project, picked) : { model: "", effort: "" };
  return { choice, picked, pick };
}

// Each list is one section, whose label says what it lists.
const MODEL = { key: "model", label: "Model" };
const EFFORT = { key: "effort", label: "Effort" };

/** Where the project's own setting is, which sessions follow unless they ask for another. */
const theirs = (o: Offer) => (o.own ? "The project's default" : undefined);

/**
 * The model and effort level pickers of a start form, in its row of controls. Where the row is
 * narrow (below `lg`, with the sidebar beside it from `md`), they take a line of their own above
 * it, which needs the row to wrap.
 */
export function StartChoices({
  project,
  choice,
  onPick,
  disabled,
}: {
  project: ChoiceProject;
  choice: Choice;
  onPick: (c: Partial<Choice>) => void;
  disabled?: boolean;
}) {
  return (
    <div className="flex min-w-0 items-center gap-2 max-lg:order-first max-lg:w-full">
      <Select
        size="sm"
        aria-label="Model"
        value={choice.model}
        disabled={disabled}
        onChange={(model) => onPick({ model })}
        options={modelOffers(project, choice.model).map((o) => ({
          value: o.value,
          label: o.value ? o.label : "Default model",
          item: o.label,
          description: theirs(o),
          section: MODEL,
        }))}
      />
      <Select
        size="sm"
        aria-label="Effort"
        value={choice.effort}
        disabled={disabled}
        onChange={(effort) => onPick({ effort })}
        options={effortOffers(project, choice.effort).map((o) => ({
          value: o.value,
          label: o.value ? `${o.label} effort` : "Default effort",
          item: o.label,
          icon: <EffortIcon level={o.value} />,
          description: theirs(o),
          section: EFFORT,
        }))}
      />
    </div>
  );
}

// One label leaves quickly and the other rises in just after it, so the two barely overlap.
const label =
  "absolute inset-y-0 right-0 flex w-max items-center gap-2 transition-[opacity,translate,filter] ease-out";
const shown = "delay-50 duration-300 motion-reduce:delay-0";
const gone = "opacity-0 blur-[2px] duration-150";

/**
 * A start form's submit button. When `starting`, its label rises away, "Starting" and a spinner
 * rise in, and the button eases to its new width and stops taking presses.
 */
export function StartButton({
  starting,
  disabled,
  className,
  children,
}: {
  starting: boolean;
  disabled?: boolean;
  className?: string;
  children: ReactNode;
}) {
  const idle = useRef<HTMLSpanElement>(null);
  const busy = useRef<HTMLSpanElement>(null);
  // The labels are stacked out of flow, so the box holding them gets a measured width, which can
  // transition. Measured again when a label resizes (when the font loads, say). The computed width
  // keeps fractions of a pixel and ignores transforms such as a hover scale.
  const [widths, setWidths] = useState<[number, number]>();
  useLayoutEffect(() => {
    const width = (el: HTMLElement | null) => (el && Number.parseFloat(getComputedStyle(el).width)) || 0;
    const measure = () => {
      const w: [number, number] = [width(idle.current), width(busy.current)];
      setWidths((old) => (old && old[0] === w[0] && old[1] === w[1] ? old : w));
    };
    measure();
    const observer = new ResizeObserver(measure);
    for (const el of [idle.current, busy.current]) if (el) observer.observe(el);
    return () => observer.disconnect();
  }, []);
  return (
    <button
      type="submit"
      disabled={disabled || starting}
      className={cn(
        "flex h-9 shrink-0 items-center overflow-hidden rounded-full pr-3 pl-4 text-[13.5px] font-medium transition-all duration-200 ease-[var(--ease-spring)]",
        disabled ? "bg-raised text-ink-4" : "bg-ink text-canvas",
        starting && "cursor-progress",
        className,
      )}
    >
      <span
        className="relative self-stretch transition-[width] duration-300 ease-out"
        style={{ width: widths?.[starting ? 1 : 0] }}
      >
        <span
          ref={idle}
          aria-hidden={starting}
          className={cn(label, starting ? cn(gone, "-translate-y-1.5") : shown)}
        >
          {children} <ArrowUp className="size-4" />
        </span>
        <span
          ref={busy}
          aria-hidden={!starting}
          className={cn(label, starting ? shown : cn(gone, "translate-y-1.5"))}
        >
          Starting <Spinner className={cn("size-4", !starting && "[animation-play-state:paused]")} />
        </span>
      </span>
    </button>
  );
}

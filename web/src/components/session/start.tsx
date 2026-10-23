// Starting a session from a prompt, on the home page and on a project's page, with the model
// and effort level it asks for, and what its agent may do on GitHub.

import { useMutation, useQueryClient } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ArrowUp } from "lucide-react";
import { type ReactNode, useCallback, useLayoutEffect, useRef, useState } from "react";
import { GithubAccessIcon } from "@/components/github-access";
import { EffortIcon, Select, Spinner, toast } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { githubOffers, onGithub } from "@/lib/github-access";
import { type Choice, effortOffers, modelOffers, type Offer, startChoice } from "@/lib/models";
import { keys } from "@/lib/queries";
import type { SentFiles } from "./attachments";

/** The files a start form holds (see `useAttachments`). */
export interface StartFiles {
  drafts: readonly unknown[];
  take: () => Promise<{ files: SentFiles; restore: () => void }>;
}

/** The model, effort level and GitHub access a session asks for as it starts (see `asked`). */
export type StartAsks = Pick<G.CreateSessionRequest, "model" | "effort" | "github_access">;

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
      if (asks?.github_access) body.github_access = asks.github_access;
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

/** A project as a start form knows it: its repositories say whether its agents work on GitHub. */
type ChoiceProject = Parameters<typeof startChoice>[0] &
  Partial<Pick<G.Project, "repo" | "other_repos" | "all_repos_of">>;

/**
 * The model, effort level and GitHub access a start form shows for `project` (see
 * `startChoice`), and `pick` for the pickers. What was picked (`picked`, starting as `kept`) stays
 * when the project changes, as far as the new one allows it.
 */
export function useStartChoice(project: ChoiceProject | undefined, kept?: Partial<Choice>) {
  const [picked, setPicked] = useState<Partial<Choice>>(() => ({
    model: kept?.model,
    effort: kept?.effort,
    github: kept?.github,
  }));
  const pick = useCallback((c: Partial<Choice>) => setPicked((p) => ({ ...p, ...c })), []);
  const choice: Choice = project ? startChoice(project, picked) : { model: "", effort: "", github: "push" };
  return { choice, picked, pick };
}

// Each list is one section, whose label says what it lists.
const MODEL = { key: "model", label: "Model" };
const EFFORT = { key: "effort", label: "Effort" };
const GITHUB = { key: "github", label: "On GitHub" };

/** Where the project's own setting is, which sessions follow unless they ask for another. */
const theirs = (o: Offer) => (o.own ? "The project's default" : undefined);

/** What the effort picker says once a level is picked: Extra high, the longest, as "Extra effort". */
const effortText = (o: Offer) =>
  o.value ? `${o.value === "xhigh" ? "Extra" : o.label} effort` : "Default effort";

/**
 * On a phone the two pickers make one control, joined where their borders meet. The one in use
 * comes to the front, so its border shows whole; it doesn't shrink as it is pressed, which would
 * open a gap between them.
 */
const joined =
  "max-sm:hover:z-10 max-sm:focus-visible:z-10 max-sm:aria-expanded:z-10 max-sm:active:scale-100";

/**
 * The model, effort level and GitHub access pickers of a start form, in its row of controls; the
 * last only where the project's agents work on GitHub, where it shows just its symbol, so that
 * Start keeps its place at the end of the row. Where the row is narrow (from `sm` to `lg`, with the
 * sidebar beside it from `md`), they take a line of their own above it, which needs the row to
 * wrap. On a phone they show just the model's name and the symbols of the effort level and the
 * GitHub access, in one control, so that they fit in the row with the others; where it has too
 * little room, the model's name gives way, never the symbols.
 */
export function StartChoices({
  project,
  choice,
  onPick,
  disabled,
  className,
}: {
  project: ChoiceProject;
  choice: Choice;
  onPick: (c: Partial<Choice>) => void;
  disabled?: boolean;
  className?: string;
}) {
  const github = onGithub(project);
  return (
    <div
      className={cn(
        "flex min-w-0 items-center gap-2 max-sm:gap-0 sm:max-lg:order-first sm:max-lg:w-full",
        className,
      )}
    >
      <Select
        size="sm"
        narrow="bare"
        // On a phone a model named by its ID shows the start of it, so that the row keeps one line
        // with the GitHub picker and the buttons beside it, down to 360 px.
        className={cn("max-sm:max-w-24 max-sm:rounded-r-none", joined)}
        aria-label="Model"
        value={choice.model}
        disabled={disabled}
        onChange={(model) => onPick({ model })}
        options={modelOffers(project, choice.model).map((o) => ({
          value: o.value,
          label: o.value ? o.label : "Default model",
          short: o.value ? o.label : "Default",
          item: o.label,
          description: theirs(o),
          section: MODEL,
        }))}
      />
      <Select
        size="sm"
        narrow="bare"
        className={cn(
          "max-sm:-ml-px max-sm:shrink-0 max-sm:rounded-l-none",
          github && "max-sm:rounded-r-none",
          joined,
        )}
        aria-label="Effort"
        value={choice.effort}
        disabled={disabled}
        onChange={(effort) => onPick({ effort })}
        options={effortOffers(project, choice.effort).map((o) => ({
          value: o.value,
          label: effortText(o),
          short: "",
          item: o.label,
          icon: <EffortIcon level={o.value} />,
          description: theirs(o),
          section: EFFORT,
        }))}
      />
      {github && (
        <Select
          size="sm"
          narrow="bare"
          compact
          className={cn("max-sm:-ml-px max-sm:shrink-0 max-sm:rounded-l-none", joined)}
          aria-label="On GitHub"
          value={choice.github}
          disabled={disabled}
          onChange={(g) => onPick({ github: g })}
          options={githubOffers(project).map((l) => ({
            value: l.value,
            label: l.label,
            short: "",
            icon: <GithubAccessIcon access={l.value} />,
            description: l.description,
            section: GITHUB,
          }))}
        />
      )}
    </div>
  );
}

// One label leaves quickly and the other rises in just after it, so the two barely overlap.
const label =
  "absolute inset-y-0 right-0 flex w-max items-center gap-2 transition-[opacity,translate,filter] ease-out";
const shown = "delay-50 duration-300 motion-reduce:delay-0";
const gone = "opacity-0 blur-[2px] duration-150";
/** On a phone the words are for screen readers: the arrow and the spinner stand for them. */
const words = "max-sm:sr-only";

/**
 * A start form's submit button. When `starting`, its label rises away, "Starting" and a spinner
 * rise in, and the button eases to its new width and stops taking presses. On a phone it is a
 * circle around the arrow, or the spinner, so that it fits in the row with the form's lists.
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
        "flex h-9 shrink-0 items-center overflow-hidden rounded-full pr-3 pl-4 text-[13.5px] font-medium transition-all duration-200 ease-[var(--ease-spring)] max-sm:px-2.5",
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
          <span className={words}>{children}</span>
          <ArrowUp className="size-4" />
        </span>
        <span
          ref={busy}
          aria-hidden={!starting}
          className={cn(label, starting ? shown : cn(gone, "translate-y-1.5"))}
        >
          <span className={words}>Starting</span>
          <Spinner className={cn("size-4", !starting && "[animation-play-state:paused]")} />
        </span>
      </span>
    </button>
  );
}

// Starting a session from a prompt, on the home page and on a project's page.

import { useMutation } from "@tanstack/react-query";
import { useNavigate } from "@tanstack/react-router";
import { ArrowUp } from "lucide-react";
import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import { Spinner, toast } from "@/components/ui";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import type { SentFiles } from "./attachments";

/** The files a start form holds (see `useAttachments`). */
export interface StartFiles {
  drafts: readonly unknown[];
  take: () => Promise<{ files: SentFiles; restore: () => void }>;
}

/**
 * Starts a session and opens it. While a start is in flight, `start` does nothing, so a double
 * click or a second ⌘↵ cannot create a duplicate. `starting` stays true until the session page
 * replaces the form, and turns false again if the start fails (attached files go back in the
 * form). A start needs a prompt or files; it waits for files still uploading.
 */
export function useStartSession() {
  const navigate = useNavigate();
  // isPending reaches the component a tick after mutate(); this closes that gap.
  const inFlight = useRef(false);
  const create = useMutation({
    mutationFn: async ({
      project,
      prompt,
      files,
    }: {
      project: string;
      prompt: string;
      files?: StartFiles;
    }) => {
      const taken = files?.drafts.length ? await files.take() : undefined;
      const attachments = taken?.files.attachments.map((a) => a.id) ?? [];
      try {
        const s = await api.createSession(project, attachments.length ? { prompt, attachments } : { prompt });
        for (const url of Object.values(taken?.files.previews ?? {})) URL.revokeObjectURL(url);
        return s;
      } catch (e) {
        taken?.restore();
        throw e;
      }
    },
    onSuccess: (s) => navigate({ to: "/s/$id", params: { id: s.id } }),
    onError: (e) => {
      inFlight.current = false;
      toast(e.message, "error");
    },
  });
  return {
    starting: create.isPending || create.isSuccess,
    start: (project: string, prompt: string, files?: StartFiles) => {
      if (inFlight.current || (!prompt.trim() && !files?.drafts.length)) return;
      inFlight.current = true;
      create.mutate({ project, prompt, files });
    },
  };
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

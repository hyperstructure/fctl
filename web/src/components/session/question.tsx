// What a session's agent asks the people who follow it (`fctl ask`), until someone answers: the
// question, and the answers it offers to click, each sent as the reply as it is. Any message
// answers it, and it goes as the control plane says so.

import { MessageCircleQuestion } from "lucide-react";
import { useState } from "react";
import { Spinner, toast } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { agoShort } from "@/lib/format";
import { useMedia } from "@/lib/media";

/**
 * Where the window has room for the question above the message box, wherever the reader is in
 * the transcript: at least 768 px wide (md) and 512 px tall. On a phone, either way up, a long
 * question with its answers would cover most of the transcript there.
 */
const FLOATS = "(min-width: 48rem) and (min-height: 32rem)";

/**
 * Whether the question floats above the message box (`true`), or ends the transcript, scrolling
 * away with it, where the window has too little room for it to float.
 */
export function useQuestionFloats(): boolean {
  return useMedia(FLOATS);
}

export function QuestionCard({
  question,
  canAnswer,
  onAnswer,
  floating = true,
  className,
}: {
  question: G.SessionQuestion;
  /** The person may message the agent. */
  canAnswer: boolean;
  /** Sends `text` as their reply. */
  onAnswer: (text: string) => Promise<void>;
  /** Over the transcript, which scrolls under it, rather than in it. */
  floating?: boolean;
  className?: string;
}) {
  const [sending, setSending] = useState<string>();
  const options = question.options ?? [];
  const answer = async (text: string) => {
    if (sending) return;
    setSending(text);
    try {
      await onAnswer(text);
    } catch (e) {
      toast(e instanceof Error ? e.message : "Not sent", "error");
    } finally {
      setSending(undefined);
    }
  };
  return (
    <section
      aria-label="The agent asks you"
      className={cn(
        "animate-rise rounded-2xl border border-blue/30 p-4",
        floating ? "bg-surface/95 shadow-[0_10px_40px_rgb(0_0_0/0.22)] backdrop-blur-xl" : "bg-surface",
        "bg-[linear-gradient(to_bottom,var(--c-blue-soft),transparent_70%)]",
        className,
      )}
    >
      <div className="flex items-start gap-3">
        <span className="mt-0.5 flex size-7 shrink-0 items-center justify-center rounded-full bg-blue-soft text-blue">
          <MessageCircleQuestion className="size-4" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="flex items-baseline gap-2 text-[12px] font-medium text-blue">
            The agent asks you
            <span className="font-normal text-ink-4">{agoShort(question.asked_at)}</span>
          </p>
          <p className="mt-1 text-[15px] leading-relaxed whitespace-pre-line text-ink [overflow-wrap:anywhere]">
            {question.text}
          </p>
          {canAnswer && options.length > 0 && (
            <div className="mt-3 flex flex-wrap gap-2">
              {options.map((o) => (
                <button
                  key={o}
                  type="button"
                  disabled={!!sending}
                  onClick={() => void answer(o)}
                  className={cn(
                    "inline-flex min-h-8 max-w-full items-center gap-2 rounded-full border px-3.5 py-1 text-left text-[13px] transition-[background-color,border-color,color,transform] duration-150 active:scale-[0.97]",
                    sending === o
                      ? "border-blue/50 bg-blue-soft text-ink"
                      : "border-line-strong bg-raised text-ink-2 hover:border-blue/45 hover:bg-blue-soft hover:text-ink disabled:opacity-60",
                  )}
                >
                  {sending === o && <Spinner className="size-3" />}
                  <span className="[overflow-wrap:anywhere]">{o}</span>
                </button>
              ))}
            </div>
          )}
          <p className="mt-2.5 text-[12px] text-ink-4">
            {canAnswer
              ? options.length > 0
                ? "Or write your own answer below. Any reply answers it."
                : "Your next message answers it."
              : "The people who can message the agent answer it."}
          </p>
        </div>
      </div>
    </section>
  );
}

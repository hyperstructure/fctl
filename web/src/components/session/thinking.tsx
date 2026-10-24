// What the agent thought before it went on: a line that opens into it.

import { useId, useState } from "react";
import { ThoughtIcon } from "@/components/ui";
import { cn } from "@/lib/cn";
import { num } from "@/lib/format";
import { OpenLine, RailIcon, Thread } from "./lines";

/** What a thought that streams in says, with how far it got once Claude Code says: "Thinking · 1.2K tokens". */
export const thinkingNow = (tokens?: number) =>
  tokens ? `Thinking · ${num(tokens)} ${tokens === 1 ? "token" : "tokens"}` : "Thinking";

export function Thinking({
  text,
  open: shown,
  onToggle,
  nested,
  find,
}: {
  text: string;
  /** Whether it is open, when the run it is in keeps that. */
  open?: boolean;
  onToggle?: () => void;
  /** In a run or a subagent's work. */
  nested?: boolean;
  /** What finding in the transcript knows it as. */
  find?: string;
}) {
  const [own, setOwn] = useState(false);
  const open = shown ?? own;
  const id = useId();
  const icon = <ThoughtIcon className="size-3.5" />;
  return (
    <div>
      <OpenLine
        icon={
          nested ? (
            <span className="flex size-3.5 shrink-0 text-violet/80">{icon}</span>
          ) : (
            <RailIcon className="text-violet/80">{icon}</RailIcon>
          )
        }
        open={open}
        onToggle={onToggle ?? (() => setOwn(!open))}
        controls={id}
        nested={nested}
      >
        <span className="text-ink-3 transition-colors group-hover/line:text-ink-2">Thought it through</span>
      </OpenLine>
      {open && (
        <Thread id={id} nested={nested} tone="thought" className="mt-0.5 mb-2 animate-rise">
          <p
            data-find={find}
            className={cn(
              "py-1 leading-relaxed whitespace-pre-wrap wrap-anywhere text-ink-3 italic",
              nested ? "text-[13.5px]" : "text-[14px]",
            )}
          >
            {text}
          </p>
        </Thread>
      )}
    </div>
  );
}

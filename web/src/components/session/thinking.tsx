// What the agent thought before it went on: a line that opens into it.

import { BrainCircuit, ChevronRight } from "lucide-react";
import { useState } from "react";
import { cn } from "@/lib/cn";

export function Thinking({ text }: { text: string }) {
  const [open, setOpen] = useState(false);
  return (
    <div className="py-1">
      <button
        type="button"
        onClick={() => setOpen(!open)}
        className="flex items-center gap-2 text-[12.5px] text-violet/80 transition-colors hover:text-violet"
      >
        <BrainCircuit className="size-3.5" />
        Thought it through
        <ChevronRight className={cn("size-3 transition-transform", open && "rotate-90")} />
      </button>
      {open && (
        <div className="mt-2 ml-5.5 animate-rise border-l border-violet/25 pl-4 font-display text-[15px] leading-relaxed whitespace-pre-wrap text-ink-3 italic">
          {text}
        </div>
      )}
    </div>
  );
}

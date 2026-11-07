// The pieces of Settings → Claude's guides: numbered steps that tick off as they are done, and
// commands in a sentence.

import { Check } from "lucide-react";
import type { ReactNode } from "react";
import { cn } from "@/lib/cn";

/** A step of a guide: its number, or a check once it is done. */
export function Step({
  n,
  done,
  title,
  aside,
  children,
}: {
  n: number;
  done: boolean;
  title: ReactNode;
  aside?: ReactNode;
  children: ReactNode;
}) {
  return (
    <li className="group relative flex gap-4 pb-7 last:pb-0">
      {/* The line to the next step, behind the numbers. */}
      <span aria-hidden="true" className="absolute top-8 bottom-1.5 left-3 w-px bg-line group-last:hidden" />
      <span
        className={cn(
          "relative mt-px flex size-6 shrink-0 items-center justify-center rounded-full border font-mono text-[11.5px] transition-colors duration-300 motion-reduce:transition-none",
          done ? "border-accent bg-accent text-accent-ink" : "border-line-strong bg-canvas text-ink-3",
        )}
      >
        {done ? <Check className="size-3.5" aria-label="Done" /> : n}
      </span>
      <div className="min-w-0 flex-1">
        <div className="flex flex-wrap items-baseline justify-between gap-x-3 gap-y-1">
          <div className="text-[14px] font-medium text-ink">{title}</div>
          {aside}
        </div>
        <div className="mt-1.5 flex max-w-2xl flex-col gap-2.5 text-[13px] leading-relaxed text-ink-3">
          {children}
        </div>
      </div>
    </li>
  );
}

/** A command or path in a sentence, kept on one line: they are short, and a hyphen mustn't break them. */
export function Mono({ children }: { children: ReactNode }) {
  return (
    <code className="rounded bg-raised px-1 py-px font-mono text-[11.5px] whitespace-nowrap text-ink-2">
      {children}
    </code>
  );
}

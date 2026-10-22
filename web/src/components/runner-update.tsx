// On the home page, for the people who registered runners: which of theirs
// run an older fctl than the control plane, and the command that updates
// them, until they skip that version.

import { ChevronDown, CircleFadingArrowUp } from "lucide-react";
import { useId, useState } from "react";
import { Button, CodeBox } from "@/components/ui";
import { cn } from "@/lib/cn";
import { useInstance, useRunners } from "@/lib/queries";
import { outdated, useSkippedUpdate } from "@/lib/updates";

/** "a", "a and b", "a, b and c". */
function and(items: string[]): string {
  return items.length < 2 ? (items[0] ?? "") : `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}

const code = "rounded bg-raised px-1 font-mono text-[12px] text-ink-2";

/** Nothing while your runners online run the control plane's version of fctl, or you skipped it. */
export function RunnerUpdate({ user }: { user: string }) {
  const { data: instance } = useInstance();
  const runners = useRunners();
  const [skipped, skip] = useSkippedUpdate(user);
  const [open, setOpen] = useState(false);
  const title = useId();
  const how = useId();
  const version = instance?.version;
  const behind = outdated(runners.data?.runners ?? [], version);
  if (!version || !behind.length || skipped === version) return null;
  const one = behind.length === 1;
  return (
    <section
      aria-labelledby={title}
      className="animate-rise rounded-xl border border-blue/20 bg-blue-soft px-4 py-3.5"
    >
      <div className="flex flex-wrap items-center gap-4">
        <span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-blue/15 text-blue">
          <CircleFadingArrowUp className="size-4.5" />
        </span>
        <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
          <div id={title} className="text-[14.5px] text-ink">
            Update your {one ? "runner" : "runners"} to fctl {version}
          </div>
          <div className="mt-0.5 text-[12.5px] text-ink-3">
            {and(behind.map((r) => `${r.name} runs ${r.version}`))}
          </div>
        </div>
        {/* On a phone they take a line of their own. */}
        <div className="flex gap-2">
          <Button size="sm" variant="ghost" onClick={() => skip(version)}>
            Skip this version
          </Button>
          <Button
            size="sm"
            aria-expanded={open}
            aria-controls={how}
            onClick={() => setOpen(!open)}
            trailing={
              <ChevronDown
                className={cn("size-3.5 text-ink-3 transition-transform duration-200", open && "rotate-180")}
              />
            }
          >
            How to update
          </Button>
        </div>
      </div>
      <div
        id={how}
        hidden={!open}
        className="mt-3.5 animate-rise space-y-2 text-[13px] leading-relaxed text-ink-3 sm:ml-13"
      >
        <p>Run this on {and(behind.map((r) => r.name))}:</p>
        <CodeBox value="fctl self update" />
        <p>
          It installs the latest fctl and restarts the host service; the sessions running there carry on. This
          notice goes away once {one ? "it reconnects" : "they reconnect"} with the new version.
        </p>
        <p className="text-[12.5px] text-ink-4">
          Installed fctl with Homebrew? Run <code className={code}>brew upgrade fctl</code> instead, then{" "}
          <code className={code}>fctl host restart</code>.
        </p>
      </div>
    </section>
  );
}

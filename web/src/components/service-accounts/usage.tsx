// How a program uses a service account: starting a session for someone with fctl, curl or Node.js,
// on this instance, in one of the workspace's projects, with its secret where it was just made.

import { useState } from "react";
import { CodeBlock } from "@/components/markdown";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { useProjects } from "@/lib/queries";
import { ON_BEHALF_OF, USAGE_LANGUAGES, type UsageLanguage, usageSnippet } from "@/lib/service-accounts";

export function UsageHow({ workspace: w, secret }: { workspace: G.Workspace; secret?: string }) {
  const [language, setLanguage] = useState<UsageLanguage>("cli");
  const chosen = USAGE_LANGUAGES.find((l) => l.value === language) ?? USAGE_LANGUAGES[0];
  // One of its projects, by name, for the example: the one people see first.
  const project = (useProjects().data?.projects ?? []).find((p) => p.workspace?.id === w.id)?.slug ?? "web";
  return (
    <div className="flex flex-col gap-3">
      <div className="flex flex-wrap items-end justify-between gap-3">
        <div className="min-w-0">
          <h4 className="text-[13px] font-semibold text-ink">Start a session for someone</h4>
          <p className="mt-0.5 max-w-xl text-[12.5px] leading-relaxed text-ink-3">
            Each request names the person it's for, by their address or fctl id, in the{" "}
            <code className="font-mono text-[12px]">{ON_BEHALF_OF}</code> header (
            <code className="font-mono text-[12px]">--on-behalf-of</code> in the CLI). Without one, it reads
            the workspace's projects, sessions and schedules.
          </p>
        </div>
        <div
          role="tablist"
          aria-label="Language"
          className="inline-flex rounded-full bg-sunken/70 p-0.5 shadow-[inset_0_0_0_1px_var(--c-line)]"
        >
          {USAGE_LANGUAGES.map((l) => (
            <button
              key={l.value}
              type="button"
              role="tab"
              aria-selected={language === l.value}
              onClick={() => setLanguage(l.value)}
              className={cn(
                "h-7 rounded-full px-3 text-[12.5px] transition-[background-color,color,box-shadow] duration-150",
                language === l.value
                  ? "bg-raised text-ink shadow-[0_1px_2px_rgb(0_0_0/0.25),inset_0_0_0_1px_var(--c-line-strong)]"
                  : "text-ink-3 hover:text-ink",
              )}
            >
              {l.label}
            </button>
          ))}
        </div>
      </div>
      <CodeBlock
        code={usageSnippet(language, { origin: location.origin, project, person: w.email, secret })}
        lang={chosen?.lang ?? "bash"}
        label={chosen?.label}
      />
    </div>
  );
}

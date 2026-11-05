// On the home page, for the admins of workspaces where runners wait for their approval: how many,
// and where, with the way to the Runners tab that approves or rejects them.

import { Link } from "@tanstack/react-router";
import { ArrowRight, Hourglass } from "lucide-react";
import { useId } from "react";
import { Button } from "@/components/ui";
import { waitingByWorkspace } from "@/lib/enrollment";
import { useRunners, useWorkspaces } from "@/lib/queries";

/** "a", "a and b", "a, b and c". */
function and(items: string[]): string {
  return items.length < 2 ? (items[0] ?? "") : `${items.slice(0, -1).join(", ")} and ${items.at(-1)}`;
}

/** Nothing while no runner waits in a workspace you are an admin of. */
export function RunnerApprovals() {
  const runners = useRunners().data?.runners ?? [];
  const workspaces = useWorkspaces().data?.workspaces ?? [];
  const title = useId();
  const waiting = waitingByWorkspace(runners, workspaces);
  const first = waiting[0];
  if (!first) return null;
  const n = waiting.reduce((sum, w) => sum + w.runners.length, 0);
  const names = waiting.flatMap((w) => w.runners.map((r) => r.name));
  const where = and(waiting.map((w) => w.workspace.name));
  return (
    <section
      aria-labelledby={title}
      // Narrower than 36rem inside, the button takes a line of its own under the text.
      className="@container animate-rise rounded-xl border border-amber/25 bg-amber-soft px-4 py-3.5"
    >
      <div className="flex flex-col gap-3.5 @xl:flex-row @xl:items-center @xl:gap-4">
        <div className="flex min-w-0 flex-1 items-start gap-3 @xl:items-center @xl:gap-4">
          <span className="flex size-9 shrink-0 items-center justify-center rounded-full bg-amber/15 text-amber">
            <Hourglass className="size-4" />
          </span>
          <div className="min-w-0 flex-1 [overflow-wrap:anywhere]">
            <div id={title} className="text-[14.5px] text-ink">
              {n === 1 ? "A runner waits for your approval" : `${n} runners wait for your approval`}
            </div>
            <div className="mt-0.5 text-[12.5px] text-ink-3">
              {`${names.length > 3 ? `${names.slice(0, 3).join(", ")} and ${names.length - 3} more` : and(names)} in ${where}: ${n === 1 ? "it runs" : "they run"} nothing until approved`}
            </div>
          </div>
        </div>
        <Link
          to="/w/$slug"
          params={{ slug: first.workspace.slug }}
          search={{ tab: "runners" }}
          className="shrink-0"
        >
          <Button size="sm" className="@max-xl:h-9" trailing={<ArrowRight className="size-3.5 text-ink-3" />}>
            Review
          </Button>
        </Link>
      </div>
    </section>
  );
}

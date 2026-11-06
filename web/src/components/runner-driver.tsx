// Under a runner in the Runners tab and the admin's list: why the driver its
// host uses can't start sandboxes, as its host says (fctl can't use Docker,
// say), with the command that fixes it.

import type * as G from "@/gen";
import { driverProblem } from "@/lib/format";

/** Nothing while its driver is ready, or while it is offline. */
export function DriverLine({ runner: r }: { runner: G.Runner }) {
  const why = driverProblem(r);
  if (!why) return null;
  return (
    <div className="mt-0.5 text-[12px] text-amber [overflow-wrap:anywhere]">
      Its {r.driver} driver isn't ready: <Ticks text={why} />. Sessions that start on it fail until it is.
    </div>
  );
}

/** Text with its `commands` in the code font. */
function Ticks({ text }: { text: string }) {
  return (
    <>
      {text.split("`").map((part, i) =>
        i % 2 ? (
          <code key={i} className="font-mono text-[0.92em]">
            {part}
          </code>
        ) : (
          part
        ),
      )}
    </>
  );
}

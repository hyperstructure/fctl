// The new session page's sign that the agent is at work, in place of a spinner: a small square of
// dots, as the agent's orb is drawn in, with a wave of light running through them from one corner
// to the other, in the text's color. Every one on the page runs in step with the others
// (lib/shimmer.ts); where motion is reduced, the dots hold still, brightest in the corner the wave
// starts from (styles/app.css).

import type { CSSProperties } from "react";
import { cn } from "@/lib/cn";

/** The dots, on a 16-unit square, each with how far it is from the corner the wave starts in. */
const DOTS = [3, 8, 13].flatMap((y, row) => [3, 8, 13].map((x, col) => ({ x, y, i: row + col })));

export function DotWave({ className, label }: { className?: string; label?: string }) {
  return (
    <svg
      viewBox="0 0 16 16"
      className={cn("dot-wave size-3.5 shrink-0", className)}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      {DOTS.map((d) => (
        <circle key={`${d.x}-${d.y}`} cx={d.x} cy={d.y} r="1.85" style={{ "--i": d.i } as CSSProperties} />
      ))}
    </svg>
  );
}

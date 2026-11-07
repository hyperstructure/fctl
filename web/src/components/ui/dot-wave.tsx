// The new session page's sign that the agent is at work, in place of a spinner: a small square of
// dots, as the agent's orb is drawn in, with a wave of light running through them from one corner
// to the other in the brand's yellow, pink and blue. Every one on the page runs in step with the
// others (lib/shimmer.ts); where motion is reduced, the dots hold still, brightest in the corner
// the wave starts from (styles/app.css).

import type { CSSProperties } from "react";
import { useId } from "react";
import { cn } from "@/lib/cn";

/** The dots, on a 16-unit square, each with how far it is from the corner the wave starts in. */
const DOTS = [3, 8, 13].flatMap((y, row) => [3, 8, 13].map((x, col) => ({ x, y, i: row + col })));

export function DotWave({ className, label }: { className?: string; label?: string }) {
  // An id `url(#…)` can name: React's have characters it can't.
  const id = `dot-wave-${useId().replace(/[^\w-]/g, "")}`;
  return (
    <svg
      viewBox="0 0 16 16"
      className={cn("dot-wave size-3.5 shrink-0", className)}
      role={label ? "img" : undefined}
      aria-label={label}
      aria-hidden={label ? undefined : true}
    >
      <defs>
        <linearGradient id={id} x1="1" y1="1" x2="15" y2="15" gradientUnits="userSpaceOnUse">
          <stop offset="0" style={{ stopColor: "var(--wave-a)" }} />
          <stop offset="0.55" style={{ stopColor: "var(--wave-b)" }} />
          <stop offset="1" style={{ stopColor: "var(--wave-c)" }} />
        </linearGradient>
      </defs>
      {DOTS.map((d) => (
        <circle
          key={`${d.x}-${d.y}`}
          cx={d.x}
          cy={d.y}
          r="1.85"
          fill={`url(#${id})`}
          style={{ "--i": d.i } as CSSProperties}
        />
      ))}
    </svg>
  );
}

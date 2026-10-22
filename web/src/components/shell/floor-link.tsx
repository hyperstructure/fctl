// The Agent Floor's row in the sidebar. With a mouse over it, the row powers on into the
// conduit (@hyperstructure/design): the foundry squeezed into a bar, with a thread lit for
// each session at work and a packet sent down one for each thing their agents do. Moving
// along it parts the threads, and a click sends a burst. On touch screens it is a row like
// the others.

import { type ConduitHandle, conduit, DISSOLVE_MS, LANES } from "@hyperstructure/design/gl/conduit";
import type { FoundryLane } from "@hyperstructure/design/gl/foundry";
import { Link } from "@tanstack/react-router";
import { type PointerEvent, useEffect, useMemo, useRef, useState } from "react";
import { AgentFloorIcon, GLCanvas } from "@/components/ui";
import { cn } from "@/lib/cn";
import { feed } from "@/lib/feed";

/** The lanes of n lit threads, spread evenly over the bar. */
export function litLanes(n: number): number[] {
  const count = Math.min(n, LANES);
  return Array.from({ length: count }, (_, i) => Math.round(((i + 0.5) * LANES) / count - 0.5));
}

/** Which of the lit lanes a session's packets go down: the same one each time. */
export function laneOf(sessionId: string, lanes: number[]): number | undefined {
  let h = 0;
  for (let i = 0; i < sessionId.length; i++) h = (h * 31 + sessionId.charCodeAt(i)) | 0;
  return lanes[Math.abs(h) % Math.max(lanes.length, 1)];
}

export function FloorLink({
  active,
  working,
  onClick,
}: {
  active: boolean;
  /** Sessions at work, each a lit thread. */
  working: number;
  onClick: () => void;
}) {
  const scene = useRef<ConduitHandle | null>(null);
  const over = useRef(false);
  // The scene is made at the first hover with a mouse, and kept. It shows while the pointer
  // is over the row and until it has dissolved, and is out of the page (display: none, so
  // that it stops drawing) the rest of the time.
  const [made, setMade] = useState(false);
  const [lit, setLit] = useState(false);
  const [shown, setShown] = useState(false);
  const lanes = useMemo(() => litLanes(working), [working]);
  // The lit threads, which the scene also takes when it is made.
  const threads = useRef(new Map<number, FoundryLane>());
  useEffect(() => {
    threads.current = new Map(lanes.map((l) => [l, { lit: 1, color: 1, energy: 0.3 }]));
    scene.current?.setLanes(threads.current);
  }, [lanes]);

  useEffect(() => {
    if (!made) return;
    const off = feed.onPulse((p) => {
      const lane = p.kind === "delta" ? undefined : laneOf(p.sessionId, lanes);
      if (lane !== undefined) scene.current?.pulse(lane, p.weight);
    });
    return () => {
      off();
    };
  }, [made, lanes]);

  useEffect(() => {
    if (lit) return;
    const t = setTimeout(() => setShown(false), DISSOLVE_MS + 40);
    return () => clearTimeout(t);
  }, [lit]);

  const enter = (e: PointerEvent) => {
    if (e.pointerType !== "mouse") return;
    over.current = true;
    setMade(true);
    setShown(true);
    setLit(true);
    scene.current?.show(true);
  };
  const leave = () => {
    if (!over.current) return;
    over.current = false;
    setLit(false);
    scene.current?.pointerLeave();
    scene.current?.show(false);
  };
  const move = (e: PointerEvent<HTMLElement>) => {
    if (!over.current) return;
    const r = e.currentTarget.getBoundingClientRect();
    if (r.width && r.height)
      scene.current?.pointer((e.clientX - r.left) / r.width, (e.clientY - r.top) / r.height);
  };

  return (
    <Link
      to="/"
      onClick={() => {
        scene.current?.burst();
        onClick();
      }}
      onPointerEnter={enter}
      onPointerMove={move}
      onPointerLeave={leave}
      data-lit={lit || undefined}
      className={cn(
        "group relative isolate flex h-8 items-center gap-2.5 overflow-hidden rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
        active
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "text-ink-3 hover:bg-hover/60 hover:text-ink",
      )}
    >
      {made && (
        <span aria-hidden="true" className={cn("absolute inset-0 -z-10", !shown && "hidden")}>
          <GLCanvas
            scene={() => {
              const h = conduit();
              scene.current = h;
              h.setLanes(threads.current);
              if (over.current) h.show(true);
              return h.scene;
            }}
            // A scene pixel to about one and a half CSS pixels (two on standard screens), as the
            // foundry has them on high-density screens.
            options={{ pixel: Math.max(2, Math.round(window.devicePixelRatio * 1.5)) }}
          />
        </span>
      )}
      <span
        className={cn(
          "transition-colors [&_svg]:size-4",
          active || lit ? "text-accent" : "text-ink-4 group-hover:text-ink-3",
        )}
      >
        <AgentFloorIcon live />
      </span>
      {/* Semibold whether the bar is lit or not. */}
      <span className={cn("font-semibold", shown && "floor-label")}>Agent Floor</span>
    </Link>
  );
}

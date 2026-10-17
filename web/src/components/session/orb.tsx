import { cssColor } from "@hyperstructure/design/gl/core";
import { type OrbState, orb } from "@hyperstructure/design/gl/orb";
import { useRef } from "react";
import { GLCanvas } from "@/components/ui";
import type * as G from "@/gen";

const PALETTE: Record<G.SessionStatus, [string, string, number]> = {
  queued: ["--c-ink-3", "--c-blue", 0.45],
  provisioning: ["--c-ink-2", "--c-blue", 0.25],
  preparing: ["--c-ink-2", "--c-highlight", 0.15],
  running: ["--c-highlight", "--c-pink", 0],
  idle: ["--c-blue", "--c-pink", 0.15],
  paused: ["--c-pink", "--c-blue", 0.3],
  stopping: ["--c-ink-3", "--c-blue", 0.5],
  stopped: ["--c-ink-3", "--c-ink-4", 0.75],
  failed: ["--c-danger", "--c-ink-4", 0.4],
};

/** The agent's orb: its color is the session's state, its motion the agent's activity. */
export function AgentOrb({
  status,
  activity,
  size = 44,
}: {
  status?: G.SessionStatus;
  activity: number;
  size?: number;
}) {
  const state = useRef<OrbState>({ a: [0.3, 0.9, 0.8], b: [0.5, 0.7, 1], energy: 0, dim: 0 });
  const [a, b, dim] = PALETTE[status ?? "queued"];
  const busy =
    status === "running"
      ? Math.min(1, 0.35 + activity / 40)
      : status === "provisioning" || status === "preparing"
        ? 0.3
        : 0.06;
  state.current = {
    a: cssColor(a, [0.3, 0.9, 0.8]),
    b: cssColor(b, [0.5, 0.7, 1]),
    energy: busy,
    dim,
  };
  return (
    <div className="relative shrink-0" style={{ width: size, height: size }}>
      <GLCanvas scene={() => orb(() => state.current)} options={{ pixel: 2, fps: 30 }} />
    </div>
  );
}

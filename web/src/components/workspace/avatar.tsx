// A workspace's avatar: its uploaded image, or its initials on its color, in
// a rounded square (people's avatars are round).

import { palette } from "@hyperstructure/design/brand";
import { useState } from "react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";

/** Each color's fill and the ink on it (the darker ink reads best on all three brand colors). */
export const WORKSPACE_COLORS: Record<G.WorkspaceColor, { fill: string; ink: string; label: string }> = {
  yellow: { fill: palette.yellow, ink: palette.blackish, label: "Yellow" },
  pink: { fill: palette.pink, ink: palette.blackish, label: "Pink" },
  blue: { fill: palette.blue, ink: palette.blackish, label: "Blue" },
  ink: { fill: palette.blackish, ink: palette.whiteish, label: "Ink" },
};

/** "hyperstructure" → "H", "Side project" → "SP". */
export function workspaceInitials(name: string): string {
  const words = name
    .trim()
    .split(/[\s\-_.]+/)
    .filter(Boolean);
  const letters = words.slice(0, 2).map((w) => [...w][0] ?? "");
  return letters.join("").toUpperCase() || "?";
}

export function WorkspaceAvatar({
  workspace,
  size = 20,
  className,
  title = true,
}: {
  workspace: Pick<G.WorkspaceRef, "name" | "color" | "avatar_url">;
  size?: number;
  className?: string;
  /** Show its name on hover. */
  title?: boolean;
}) {
  const [broken, setBroken] = useState<string>();
  const radius = Math.max(4, Math.round(size * 0.24));
  const style = { width: size, height: size, borderRadius: radius };
  const cls = cn(
    "inline-flex shrink-0 items-center justify-center overflow-hidden select-none",
    // A hairline keeps the ink square on the dark canvas, and white images on the light one.
    "shadow-[inset_0_0_0_1px_var(--c-line-strong)]",
    className,
  );
  if (workspace.avatar_url && broken !== workspace.avatar_url) {
    return (
      <span className={cls} style={style} title={title ? workspace.name : undefined}>
        <img
          src={workspace.avatar_url}
          alt=""
          className="size-full object-cover"
          onError={() => setBroken(workspace.avatar_url)}
        />
      </span>
    );
  }
  const c = WORKSPACE_COLORS[workspace.color] ?? WORKSPACE_COLORS.yellow;
  const initials = workspaceInitials(workspace.name);
  return (
    <span
      className={cls}
      title={title ? workspace.name : undefined}
      style={{
        ...style,
        background: c.fill,
        color: c.ink,
        fontSize: Math.max(8, size * (initials.length > 1 ? 0.38 : 0.48)),
      }}
    >
      <span className="font-display leading-none font-semibold tracking-[-0.02em]">{initials}</span>
    </span>
  );
}

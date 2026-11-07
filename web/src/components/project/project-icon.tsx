// A project's icon: its IconMCP glyph in its color, alone or on a soft square (a tile), which
// headers and rails show. lib/project-icons.ts picks it.

import { createElement, type ReactNode, useId } from "react";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import type { IconElement } from "@/lib/project-icon-data";
import { type Fits, ICON_COLORS, type IconChoice, iconDef, iconOf } from "@/lib/project-icons";

function draw(els: IconElement[], mask: string, prefix = ""): ReactNode[] {
  return els.map(([tag, attrs, ...kids], i) => {
    const props: Record<string, string | number> = { key: `${prefix}${i}` };
    for (const [k, v] of Object.entries(attrs))
      props[k] = typeof v === "string" && v.includes("$m") ? v.replace("$m", mask) : v;
    return createElement(tag, props, ...draw(kids, mask, `${prefix}${i}.`));
  });
}

/** A glyph alone, `size` pixels square, in `color` (or the text's). */
export function Glyph({
  name,
  style = "outline",
  color,
  size = 16,
  className,
}: {
  name: string;
  style?: G.IconStyle;
  color?: G.WorkspaceColor;
  size?: number;
  className?: string;
}) {
  // A mask's id, unique on the page (useId's own has characters url() doesn't take).
  const mask = `pi${useId().replace(/[^a-zA-Z0-9_-]/g, "")}`;
  const def = iconDef(name);
  // Strokes about 1.25 px wide at sidebar sizes, a little bolder as the glyph grows.
  const stroke = ((1.25 + Math.max(0, size - 16) * 0.03) * 16) / size;
  return (
    <svg
      viewBox="0 0 16 16"
      width={size}
      height={size}
      fill="none"
      stroke="currentColor"
      strokeWidth={stroke}
      strokeLinecap="round"
      strokeLinejoin="round"
      aria-hidden="true"
      className={cn("shrink-0", className)}
      style={color ? { color: ICON_COLORS[color].ink } : undefined}
    >
      {def && draw(def[style], mask)}
    </svg>
  );
}

/**
 * An icon as a project shows it. A tile is `tile` pixels square, with the glyph on its soft color;
 * without one, the glyph is `size` pixels.
 */
export function ProjectIcon({
  icon,
  size = 16,
  tile,
  className,
  muted,
}: {
  icon: IconChoice;
  size?: number;
  tile?: number;
  className?: string;
  /** In a fainter ink, as something that ended. */
  muted?: boolean;
}) {
  if (!tile)
    return (
      <Glyph
        name={icon.name}
        style={icon.style}
        color={muted ? undefined : icon.color}
        size={size}
        className={cn(muted && "text-ink-4", className)}
      />
    );
  const c = ICON_COLORS[icon.color];
  return (
    <span
      className={cn("inline-flex shrink-0 items-center justify-center", className)}
      style={{
        width: tile,
        height: tile,
        borderRadius: Math.max(5, Math.round(tile * 0.28)),
        background: c.soft,
        color: c.ink,
        // A hairline keeps the ink tile's square on both themes.
        boxShadow: icon.color === "ink" ? "inset 0 0 0 1px var(--c-line)" : undefined,
      }}
    >
      <Glyph name={icon.name} style={icon.style} size={Math.round(tile * 0.6)} />
    </span>
  );
}

/** A project's own icon, or the one that fits it. */
export function ProjectMark({
  project,
  size,
  tile,
  className,
  muted,
}: {
  project: Fits & { icon?: G.ProjectIcon };
  size?: number;
  tile?: number;
  className?: string;
  muted?: boolean;
}) {
  return <ProjectIcon icon={iconOf(project)} size={size} tile={tile} className={className} muted={muted} />;
}

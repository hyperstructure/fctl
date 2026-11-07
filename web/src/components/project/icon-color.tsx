// A project icon's color of its own: the swatch after the workspace colors that holds it, and how
// the icon shows in each theme in the popover where it's chosen (components/ui/color-popover.tsx).

import type { CSSProperties, RefObject } from "react";
import { RAINBOW } from "@/components/ui/color-popover";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { colorName, shades, showHex, THEME_BACKGROUNDS } from "@/lib/color";
import { Glyph } from "./project-icon";

/**
 * The swatch for a color of its own, a radio among the workspace colors': a hue wheel until
 * there is one, then that color, in its theme's shade, inside a ring of the wheel.
 */
export function OwnSwatch({
  hex,
  ink,
  checked,
  open,
  popover,
  onClick,
  onEscape,
  ref,
}: {
  /** The color of its own the picker remembers, if any. */
  hex?: string;
  /** How that color draws in this theme (`iconPaint`). */
  ink?: { color: string; className?: string; style?: CSSProperties };
  checked: boolean;
  open: boolean;
  /** The popover's id. */
  popover: string;
  onClick: (byKeyboard: boolean) => void;
  onEscape: () => void;
  ref: RefObject<HTMLButtonElement | null>;
}) {
  const label = hex ? `Custom color: ${colorName(hex)}, ${showHex(hex)}` : "Custom color";
  return (
    // biome-ignore lint/a11y/useSemanticElements: a swatch, as a radio of the group.
    <button
      ref={ref}
      type="button"
      role="radio"
      aria-checked={checked}
      aria-label={label}
      aria-controls={open ? popover : undefined}
      title={hex ? `${showHex(hex)}: change it` : "Custom color"}
      // Enter and Space click with no detail; a pointer's click counts its presses.
      onClick={(e) => onClick(e.detail === 0)}
      onKeyDown={(e) => {
        // With the focus still here, Escape closes the popover rather than the picker.
        if (e.key === "Escape" && open) {
          e.preventDefault();
          onEscape();
        }
      }}
      className={cn(
        "group/own flex size-8 items-center justify-center rounded-lg transition-[box-shadow,background-color] duration-150 max-sm:size-[30px]",
        checked || open ? "bg-raised shadow-[inset_0_0_0_1.5px_var(--c-ink-3)]" : "hover:bg-hover",
      )}
    >
      <span className="relative size-4 rounded-full transition-transform duration-200 ease-[var(--ease-spring)] group-hover/own:rotate-45 motion-reduce:transition-none">
        {/* The wheel, whole until there's a color, then a ring around it. */}
        <span
          aria-hidden="true"
          className="absolute inset-0 rounded-full"
          style={{
            background: hex
              ? `conic-gradient(${RAINBOW})`
              : `radial-gradient(closest-side, rgb(255 255 255 / 0.9), rgb(255 255 255 / 0) 85%), conic-gradient(${RAINBOW})`,
            mask: hex ? "radial-gradient(closest-side, transparent 64%, #000 68%)" : undefined,
            opacity: hex ? 1 : 0.85,
          }}
        />
        {hex && ink && (
          <span
            aria-hidden="true"
            className={cn("absolute inset-[4px] rounded-full", ink.className)}
            style={{ ...ink.style, background: ink.color }}
          />
        )}
      </span>
    </button>
  );
}

/**
 * The icon in each theme, as it will show there, and why it isn't quite the color chosen where
 * it isn't: a shade that reads.
 */
export function IconThemes({ hex, glyph }: { hex: string; glyph: { name: string; style: G.IconStyle } }) {
  const { light, dark } = shades(hex);
  const darker = light.ink !== hex;
  const lighter = dark.ink !== hex;
  const why =
    darker && lighter
      ? "A shade for each theme, to read on both"
      : darker
        ? "Darker in the light theme, to read on white"
        : lighter
          ? "Lighter in the dark theme, to read on black"
          : "As it is, in both themes";
  return (
    <div className="flex items-center gap-2.5">
      <div
        className="flex h-8 shrink-0 overflow-hidden rounded-md shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
        // The words beside it say what it shows.
        aria-hidden="true"
      >
        {(
          [
            ["light", light, THEME_BACKGROUNDS.light],
            ["dark", dark, "#1c1c1c"],
          ] as const
        ).map(([theme, shade, bg]) => (
          <span
            key={theme}
            className="flex w-9 items-center justify-center"
            style={{ background: bg, color: shade.ink }}
            title={`In the ${theme} theme: ${showHex(shade.ink)}`}
          >
            <Glyph name={glyph.name} style={glyph.style} size={16} />
          </span>
        ))}
      </div>
      <p className="min-w-0 text-[11.5px] leading-snug text-ink-4">{why}</p>
    </div>
  );
}

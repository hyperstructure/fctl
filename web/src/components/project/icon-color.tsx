// A project icon's color of its own: the swatch after the workspace colors that holds it, and the
// popover where it's chosen. Saturation and brightness in a square, the hue on a strip, its hex,
// the eyedropper where the browser has one, and the colors the workspace's other projects have.
// The icon takes each change as it's made; nothing is kept until the picker saves. The keyboard
// does it all: arrows move the thumbs (Shift by ten), Enter in the hex closes, and so does Escape,
// which leaves the picker open.

import { Pipette } from "lucide-react";
import {
  type CSSProperties,
  type KeyboardEvent,
  type PointerEvent,
  type ReactNode,
  type RefObject,
  useEffect,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { followViewport, placeList } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import {
  colorName,
  type Hsv,
  hexToHsv,
  hsvToHex,
  parseHex,
  shades,
  showHex,
  THEME_BACKGROUNDS,
} from "@/lib/color";
import { Glyph } from "./project-icon";

/** The hue wheel's colors, round the strip and the swatch. */
const RAINBOW = "#ff0000, #ffff00, #00ff00, #00ffff, #0000ff, #ff00ff, #ff0000";

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
 * Puts the popover under its swatch, as a Select's list goes, but inside the dialog it's in where
 * it fits: it ends where the dialog's content does rather than past its edge.
 */
function place(anchor: HTMLElement, el: HTMLElement) {
  placeList(anchor, el);
  const dialog = anchor.closest("dialog")?.getBoundingClientRect();
  if (!dialog) return;
  const left = el.getBoundingClientRect().left;
  const end = dialog.right - 24 - el.offsetWidth;
  if (left <= end) return;
  const moved = Math.max(Math.max(8, dialog.left + 24), end);
  el.style.left = `${moved}px`;
  const r = anchor.getBoundingClientRect();
  el.style.transformOrigin = `${Math.round(r.left + r.width / 2 - moved)}px ${el.style.transformOrigin.split(" ")[1] ?? "top"}`;
}

/** A color other projects in the workspace have, and their names. */
export interface UsedColor {
  hex: string;
  names: string[];
}

/**
 * The popover where the color is chosen, under its swatch (over it when there's more room
 * there), in the top layer over the picker's dialog. It stays until Escape, Enter in the hex, a
 * press outside it, or the focus leaving it.
 */
export function ColorPopover({
  id,
  open,
  anchor,
  hex,
  glyph,
  used,
  workspace,
  focusHex,
  onChange,
  onClose,
}: {
  id: string;
  open: boolean;
  anchor: RefObject<HTMLElement | null>;
  hex: string;
  /** The icon, for how it shows in each theme. */
  glyph: { name: string; style: G.IconStyle };
  used: UsedColor[];
  /** The workspace's name, over the colors its projects have. */
  workspace?: string;
  /** Focuses the hex as it opens: opened from the keyboard, it's where to type. */
  focusHex?: boolean;
  onChange: (hex: string) => void;
  /** `refocus`: the focus goes back to the swatch, as it does after Escape. */
  onClose: (refocus: boolean) => void;
}) {
  // Mounted from opening until its closing animation ends.
  const [mounted, setMounted] = useState(open);
  if (open && !mounted) setMounted(true);
  // Each opening starts afresh from the color, even one while the last is still closing.
  const [wasOpen, setWasOpen] = useState(open);
  const [opening, setOpening] = useState(0);
  if (open !== wasOpen) {
    setWasOpen(open);
    if (open) setOpening((n) => n + 1);
  }
  const box = useRef<HTMLDivElement>(null);

  useLayoutEffect(() => {
    const el = box.current;
    const a = anchor.current;
    if (!mounted || !el || !a) return;
    try {
      el.showPopover();
    } catch {
      // Without the Popover API it's a fixed element where it is.
    }
    place(a, el);
    let frame = 0;
    const unfollow = followViewport((e) => {
      if (e.target instanceof Node && el.contains(e.target)) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => place(a, el));
    });
    return () => {
      cancelAnimationFrame(frame);
      unfollow();
    };
  }, [mounted, anchor]);

  // A press anywhere else closes it; on the swatch, the swatch does.
  useEffect(() => {
    if (!open) return;
    const outside = (e: globalThis.PointerEvent) => {
      const t = e.target as Node;
      if (box.current?.contains(t) || anchor.current?.contains(t)) return;
      onClose(false);
    };
    document.addEventListener("pointerdown", outside, true);
    return () => document.removeEventListener("pointerdown", outside, true);
  }, [open, anchor, onClose]);

  // The closing animation unmounts it; this is for when it can't run.
  useEffect(() => {
    if (open || !mounted) return;
    const t = window.setTimeout(() => setMounted(false), 300);
    return () => window.clearTimeout(t);
  }, [open, mounted]);

  if (!mounted) return null;
  return (
    <div
      ref={box}
      id={id}
      role="dialog"
      aria-label="Custom color"
      popover="manual"
      data-state={open ? "open" : "closed"}
      onKeyDown={(e) => {
        if (e.key === "Escape") {
          // The popover closes, not the picker under it.
          e.preventDefault();
          e.stopPropagation();
          onClose(true);
        }
      }}
      onBlur={(e) => {
        const next = e.relatedTarget as Node | null;
        if (open && next && !box.current?.contains(next) && !anchor.current?.contains(next)) onClose(false);
      }}
      onAnimationEnd={(e) => {
        if (e.target === e.currentTarget && !open) setMounted(false);
      }}
      className={cn(
        "fixed inset-auto m-0 w-[248px] overflow-y-auto overscroll-contain p-3 text-ink pointer-coarse:w-[272px]",
        "rounded-xl border border-line-strong bg-raised/95 backdrop-blur-xl",
        "shadow-[0_18px_50px_-12px_rgb(0_0_0/0.5),0_2px_8px_-2px_rgb(0_0_0/0.18)]",
        "data-[state=closed]:pointer-events-none data-[state=closed]:animate-listbox-out data-[state=open]:animate-listbox-in",
      )}
    >
      <Editor
        key={opening}
        hex={hex}
        glyph={glyph}
        used={used}
        workspace={workspace}
        focusHex={focusHex}
        onChange={onChange}
        onDone={() => onClose(true)}
      />
    </div>
  );
}

function Editor({
  hex,
  glyph,
  used,
  workspace,
  focusHex,
  onChange,
  onDone,
}: {
  hex: string;
  glyph: { name: string; style: G.IconStyle };
  used: UsedColor[];
  workspace?: string;
  focusHex?: boolean;
  onChange: (hex: string) => void;
  onDone: () => void;
}) {
  // The thumbs' places: the hue stays where it is while the color is gray, which has none.
  const [hsv, setHsv] = useState<Hsv>(() => hexToHsv(hex));
  const [text, setText] = useState(() => hex.slice(1));
  const hexField = useRef<HTMLInputElement>(null);

  const set = (next: Hsv) => {
    const h = hsvToHex(next);
    setHsv(next);
    setText(h.slice(1));
    if (h !== hex) onChange(h);
  };
  const setHex = (h: string) => {
    setHsv((old) => hexToHsv(h, old));
    setText(h.slice(1));
    if (h !== hex) onChange(h);
  };

  // biome-ignore lint/correctness/useExhaustiveDependencies: as it opens only
  useEffect(() => {
    if (focusHex) hexField.current?.focus();
  }, []);

  const eyedropper = typeof window !== "undefined" && "EyeDropper" in window;
  const pick = async () => {
    const Ctor = (
      window as unknown as { EyeDropper?: new () => { open: () => Promise<{ sRGBHex: string }> } }
    ).EyeDropper;
    if (!Ctor) return;
    try {
      const { sRGBHex } = await new Ctor().open();
      const h = parseHex(sRGBHex) ?? rgbText(sRGBHex);
      if (h) setHex(h);
    } catch {
      // Escape, or the browser refused: nothing changes.
    }
  };

  return (
    <div className="flex flex-col gap-3">
      <Area hsv={hsv} onChange={set} />
      <Hue hsv={hsv} onChange={set} />
      <div className="flex items-center gap-2">
        <label className="flex h-8 min-w-0 flex-1 items-center gap-1 rounded-md border border-line-strong bg-sunken/60 px-2.5 font-mono text-[12.5px] transition-colors focus-within:border-ink-4 pointer-coarse:h-9">
          <span className="text-ink-4">#</span>
          <input
            ref={hexField}
            value={text.toUpperCase()}
            aria-label="Hex color"
            inputMode="text"
            autoComplete="off"
            autoCapitalize="characters"
            spellCheck={false}
            maxLength={7}
            onFocus={(e) => e.currentTarget.select()}
            onChange={(e) => {
              // Only hex digits go in, so a pasted #FF8800 or "ff 88 00" reads as one.
              const digits = e.target.value
                .replace(/[^0-9a-f]/gi, "")
                .slice(0, 6)
                .toLowerCase();
              setText(digits);
              // Six digits are a color as they're typed; three only once they're done.
              if (digits.length === 6) {
                const h = parseHex(digits);
                if (h) setHex(h);
              }
            }}
            onBlur={() => {
              // Three digits are a color once they're done; fewer go back to the color it is.
              const h = parseHex(text);
              if (h) setHex(h);
              else setText(hex.slice(1));
            }}
            onKeyDown={(e) => {
              if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
                e.preventDefault();
                const h = parseHex(text);
                if (h) setHex(h);
                else setText(hex.slice(1));
                onDone();
              }
            }}
            className="min-w-0 flex-1 bg-transparent text-ink uppercase tracking-[0.04em] outline-none"
          />
          <span
            aria-hidden="true"
            className="size-3.5 shrink-0 rounded-[4px] shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
            style={{ background: hex }}
          />
        </label>
        {eyedropper && (
          <button
            type="button"
            onClick={pick}
            aria-label="Pick a color from the screen"
            title="Pick a color from the screen"
            className="flex size-8 shrink-0 items-center justify-center rounded-md border border-line-strong text-ink-3 transition-colors hover:bg-hover hover:text-ink active:scale-95"
          >
            <Pipette className="size-3.5" />
          </button>
        )}
      </div>
      <Themes hex={hex} glyph={glyph} />
      {used.length > 0 && (
        <fieldset className="-mx-3 border-t border-line px-3 pt-2.5">
          {/* Floated, so that it sits inside the fieldset's line rather than across it. */}
          <legend className="float-left mb-2 w-full truncate text-[11.5px] text-ink-4">
            {workspace ? `In ${workspace}` : "In this workspace"}
          </legend>
          <div className="clear-left flex flex-wrap gap-1.5">
            {used.map((u) => (
              <button
                key={u.hex}
                type="button"
                aria-label={`${colorName(u.hex)}, ${showHex(u.hex)}, as ${u.names.join(", ")} has`}
                aria-pressed={u.hex === hex}
                title={`${showHex(u.hex)}: ${u.names.join(", ")}`}
                onClick={() => setHex(u.hex)}
                className={cn(
                  "size-6 rounded-full transition-[box-shadow,transform] duration-150 active:scale-90 pointer-coarse:size-8",
                  u.hex === hex
                    ? "shadow-[inset_0_0_0_1px_rgb(0_0_0/0.15),0_0_0_2px_var(--c-raised),0_0_0_3.5px_var(--c-ink-3)]"
                    : "shadow-[inset_0_0_0_1px_rgb(0_0_0/0.15)] hover:scale-110",
                )}
                style={{ background: u.hex }}
              />
            ))}
          </div>
        </fieldset>
      )}
    </div>
  );
}

/** `rgb(r, g, b)`, which some browsers' eyedroppers give, as hex. */
function rgbText(s: string): string | undefined {
  const m = s.match(/(\d+)\D+(\d+)\D+(\d+)/);
  if (!m) return;
  const [r, g, b] = [m[1], m[2], m[3]].map(Number);
  return parseHex(
    [r, g, b]
      .map((x) =>
        Math.min(255, x ?? 0)
          .toString(16)
          .padStart(2, "0"),
      )
      .join(""),
  );
}

const clamp = (x: number, lo = 0, hi = 1) => Math.max(lo, Math.min(hi, x));

/** Where a pointer is across `el`, from 0 to 1 each way. */
function at(e: PointerEvent, el: HTMLElement): [number, number] {
  const r = el.getBoundingClientRect();
  return [clamp((e.clientX - r.left) / r.width), clamp((e.clientY - r.top) / r.height)];
}

/** Arrows' steps: a hundredth, or a tenth with Shift. */
const stepOf = (e: KeyboardEvent) => (e.shiftKey ? 0.1 : 0.01);

/** A thumb that follows the pointer while it's pressed, on `children`'s track. */
function Track({
  label,
  value,
  max = 100,
  text,
  onPoint,
  onKey,
  className,
  style,
  children,
}: {
  label: string;
  /** Where the thumb is, from 0 to `max`. */
  value: number;
  max?: number;
  text: string;
  onPoint: (x: number, y: number) => void;
  onKey: (e: KeyboardEvent) => boolean;
  className?: string;
  style?: CSSProperties;
  children: ReactNode;
}) {
  const [dragging, setDragging] = useState(false);
  return (
    <div
      role="slider"
      tabIndex={0}
      aria-label={label}
      aria-valuemin={0}
      aria-valuemax={max}
      aria-valuenow={Math.round(value)}
      aria-valuetext={text}
      data-dragging={dragging || undefined}
      onPointerDown={(e) => {
        if (e.button !== 0) return;
        e.preventDefault();
        e.currentTarget.focus();
        e.currentTarget.setPointerCapture(e.pointerId);
        setDragging(true);
        onPoint(...at(e, e.currentTarget));
      }}
      onPointerMove={(e) => {
        if (e.currentTarget.hasPointerCapture(e.pointerId)) onPoint(...at(e, e.currentTarget));
      }}
      onPointerUp={() => setDragging(false)}
      onPointerCancel={() => setDragging(false)}
      onLostPointerCapture={() => setDragging(false)}
      onKeyDown={(e) => {
        if (onKey(e)) e.preventDefault();
      }}
      className={cn("group/track relative touch-none select-none outline-none", className)}
      style={style}
    >
      {children}
    </div>
  );
}

const thumb =
  "pointer-events-none absolute -translate-x-1/2 -translate-y-1/2 rounded-full border-2 border-white " +
  "shadow-[0_0_0_1px_rgb(0_0_0/0.2),0_1px_4px_rgb(0_0_0/0.35)] transition-[scale,box-shadow] duration-150 " +
  "group-data-dragging/track:scale-110 " +
  "group-focus-visible/track:shadow-[0_0_0_1px_rgb(0_0_0/0.2),0_0_0_4px_var(--c-ink-3),0_1px_4px_rgb(0_0_0/0.35)]";

/** Saturation across, brightness up. */
function Area({ hsv, onChange }: { hsv: Hsv; onChange: (hsv: Hsv) => void }) {
  return (
    <Track
      label="Saturation and brightness"
      value={hsv.s * 100}
      text={`${colorName(hsvToHex(hsv))}: saturation ${Math.round(hsv.s * 100)}%, brightness ${Math.round(hsv.v * 100)}%`}
      onPoint={(x, y) => onChange({ ...hsv, s: x, v: 1 - y })}
      onKey={(e) => {
        const d = stepOf(e);
        if (e.key === "ArrowLeft") onChange({ ...hsv, s: clamp(hsv.s - d) });
        else if (e.key === "ArrowRight") onChange({ ...hsv, s: clamp(hsv.s + d) });
        else if (e.key === "ArrowUp") onChange({ ...hsv, v: clamp(hsv.v + d) });
        else if (e.key === "ArrowDown") onChange({ ...hsv, v: clamp(hsv.v - d) });
        else return false;
        return true;
      }}
      // Shorter on a short screen, so that the rest shows under it.
      className="h-36 cursor-crosshair rounded-lg pointer-coarse:h-40 [@media(max-height:600px)]:h-28"
      style={{
        background: `linear-gradient(to top, #000, transparent), linear-gradient(to right, #fff, ${hsvToHex({ h: hsv.h, s: 1, v: 1 })})`,
      }}
    >
      {/* A hairline keeps its edge on the popover in both themes. */}
      <span className="pointer-events-none absolute inset-0 rounded-lg shadow-[inset_0_0_0_1px_rgb(0_0_0/0.12)]" />
      <span
        className={cn(thumb, "size-4 pointer-coarse:size-6")}
        style={{ left: `${hsv.s * 100}%`, top: `${(1 - hsv.v) * 100}%`, background: hsvToHex(hsv) }}
      />
    </Track>
  );
}

/** The hue, round the wheel from red to red. */
function Hue({ hsv, onChange }: { hsv: Hsv; onChange: (hsv: Hsv) => void }) {
  const deg = Math.round(hsv.h);
  return (
    <Track
      label="Hue"
      value={hsv.h}
      max={359}
      text={`${deg}°, ${colorName(hsvToHex({ h: hsv.h, s: 1, v: 1 }))}`}
      onPoint={(x) => onChange({ ...hsv, h: Math.min(359.9, x * 360) })}
      onKey={(e) => {
        const d = e.shiftKey ? 10 : 1;
        const h = Math.round(hsv.h);
        if (e.key === "ArrowLeft" || e.key === "ArrowDown") onChange({ ...hsv, h: Math.max(0, h - d) });
        else if (e.key === "ArrowRight" || e.key === "ArrowUp") onChange({ ...hsv, h: Math.min(359, h + d) });
        else if (e.key === "Home") onChange({ ...hsv, h: 0 });
        else if (e.key === "End") onChange({ ...hsv, h: 359 });
        else return false;
        return true;
      }}
      // Taller than it looks, so that it's easy to catch.
      className="-my-1.5 h-6 cursor-pointer pointer-coarse:-my-1 pointer-coarse:h-8"
    >
      <span
        className="pointer-events-none absolute inset-x-0 top-1/2 h-3 -translate-y-1/2 rounded-full shadow-[inset_0_0_0_1px_rgb(0_0_0/0.12)] pointer-coarse:h-4"
        style={{ background: `linear-gradient(to right, ${RAINBOW})` }}
      />
      <span
        className={cn(thumb, "top-1/2 size-4 pointer-coarse:size-6")}
        style={{ left: `${(hsv.h / 360) * 100}%`, background: hsvToHex({ h: hsv.h, s: 1, v: 1 }) }}
      />
    </Track>
  );
}

/**
 * The icon in each theme, as it will show there, and why it isn't quite the color chosen where
 * it isn't: a shade that reads.
 */
function Themes({ hex, glyph }: { hex: string; glyph: { name: string; style: G.IconStyle } }) {
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

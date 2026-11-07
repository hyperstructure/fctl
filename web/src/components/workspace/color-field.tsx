// A workspace's color, which its avatar shows until it has an image: one of the brand's four, saved
// as it's clicked, or one of its own, chosen in a popover (components/ui/color-popover.tsx) and
// saved as the popover closes. Until then the avatars here show it as it's dragged, and the
// popover offers the colors of their own its projects have, so that it can match them.

import { type KeyboardEvent, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { ColorPopover, RAINBOW } from "@/components/ui/color-popover";
import { toast } from "@/components/ui/feedback";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { colorName, showHex, THEME_BACKGROUNDS } from "@/lib/color";
import { BRAND_HEX, FIRST_OWN, ownColors } from "@/lib/project-icons";
import { keys, queryClient } from "@/lib/queries";
import { WORKSPACE_COLORS, WorkspaceAvatar } from "./avatar";

/** The change of color a press saves. */
export type ColorChange = Pick<G.UpdateWorkspaceRequest, "color" | "hex">;

const SELECTED = "shadow-[0_0_0_2px_var(--c-accent)]";
const UNSELECTED = "hover:shadow-[0_0_0_1px_var(--c-line-strong)]";

export function WorkspaceColorField({
  w,
  save,
  onPreview,
}: {
  w: Pick<G.Workspace, "id" | "name" | "color" | "hex" | "avatar_url">;
  /** Saves a change; it settles once the workspace has it, and throws when the control plane refuses it. */
  save: (change: ColorChange) => Promise<unknown>;
  /** The color of its own being chosen and not saved yet, for avatars elsewhere to show; none once there's none. */
  onPreview?: (hex: string | undefined) => void;
}) {
  const [editing, setEditing] = useState<false | "pointer" | "keyboard">(false);
  // The color in the popover, and whether it's one to show and save: a person chose it.
  const [draft, setDraft] = useState<string>();
  const [chosen, setChosen] = useState(false);
  // The color of its own, kept while a brand color is tried, to go back to.
  const [own, setOwn] = useState(w.hex);
  const [had, setHad] = useState(w.hex);
  if (w.hex !== had) {
    setHad(w.hex);
    if (w.hex) setOwn(w.hex);
  }
  const shown = chosen && draft ? draft : w.hex;
  const swatch = useRef<HTMLButtonElement>(null);
  const brand = useRef<HTMLDivElement>(null);
  const popoverId = useId();
  // Each opening, so that a save of the last one's color doesn't clear this one's as it settles.
  const opening = useRef(0);

  useEffect(() => onPreview?.(chosen ? draft : undefined), [chosen, draft, onPreview]);

  // One save after another, so that the last color chosen is the one kept.
  const queue = useRef<Promise<unknown>>(Promise.resolve());
  const run = useCallback(
    (change: ColorChange) => {
      const next = queue.current.then(() => save(change)).catch((e: Error) => toast(e.message, "error"));
      queue.current = next;
      return next;
    },
    [save],
  );

  const close = useCallback(
    (refocus: boolean, pressed?: Node) => {
      setEditing(false);
      if (refocus) swatch.current?.focus();
      const closed = opening.current;
      const done = () => {
        if (opening.current !== closed) return;
        setChosen(false);
        setDraft(undefined);
      };
      // A press on a brand color chooses that one instead.
      if (!chosen || !draft || draft === w.hex || (pressed && brand.current?.contains(pressed)))
        return done();
      void run({ hex: draft }).finally(done);
    },
    [chosen, draft, w.hex, run],
  );

  // The colors of their own its projects have, as the sidebar's list knows them.
  const used = useMemo(() => {
    const all = queryClient.getQueryData<G.ProjectList>(keys.projects)?.projects ?? [];
    return ownColors(all.filter((p) => p.workspace?.id === w.id)).slice(0, 12);
  }, [w.id]);

  const open = (byKeyboard: boolean) => {
    if (editing) return close(byKeyboard);
    opening.current += 1;
    // It starts from the color of its own it had, else from its brand color.
    const start = own ?? (w.color === "ink" ? FIRST_OWN : BRAND_HEX[w.color]);
    setDraft(start);
    // One it had before comes back at once, as a brand color does.
    setChosen(start !== w.hex && own !== undefined);
    setEditing(byKeyboard ? "keyboard" : "pointer");
  };
  // The swatch shows the color of its own it has, or had: one press brings it back.
  const tile = shown ?? own;
  const label = tile ? `Custom color: ${colorName(tile)}, ${showHex(tile)}` : "Custom color";

  return (
    <fieldset>
      <legend className="text-[12.5px] font-medium text-ink-2">Color</legend>
      <div className="mt-2 flex gap-2">
        <div ref={brand} className="contents">
          {(Object.keys(WORKSPACE_COLORS) as G.WorkspaceColor[]).map((c) => (
            <button
              key={c}
              type="button"
              aria-pressed={!shown && w.color === c}
              aria-label={WORKSPACE_COLORS[c].label}
              onClick={() => void run({ color: c })}
              className={cn(
                "rounded-lg p-1 transition-shadow duration-150",
                !shown && w.color === c ? SELECTED : UNSELECTED,
              )}
            >
              <WorkspaceAvatar workspace={{ name: w.name, color: c }} size={26} title={false} />
            </button>
          ))}
        </div>
        <button
          ref={swatch}
          type="button"
          aria-pressed={!!shown}
          aria-expanded={!!editing}
          aria-haspopup="dialog"
          aria-controls={editing ? popoverId : undefined}
          aria-label={label}
          title={tile ? `${showHex(tile)}: change it` : "Custom color"}
          // Enter and Space click with no detail; a pointer's click counts its presses.
          onClick={(e) => open(e.detail === 0)}
          onKeyDown={(e: KeyboardEvent) => {
            // With the focus still here, Escape closes the popover.
            if (e.key === "Escape" && editing) {
              e.preventDefault();
              close(true);
            }
          }}
          className={cn(
            "group/own relative rounded-lg p-1 transition-shadow duration-150",
            shown || editing ? SELECTED : UNSELECTED,
          )}
        >
          {tile ? (
            <>
              <WorkspaceAvatar
                workspace={{ name: w.name, color: w.color, hex: tile }}
                size={26}
                title={false}
              />
              {/* The wheel, small at its corner, says it's the one to change. */}
              <span
                aria-hidden="true"
                className="absolute right-0 bottom-0 size-2.5 rounded-full shadow-[0_0_0_2px_var(--c-canvas)] transition-transform duration-200 ease-[var(--ease-spring)] group-hover/own:rotate-90 motion-reduce:transition-none"
                style={{ background: `conic-gradient(${RAINBOW})` }}
              />
            </>
          ) : (
            <Wheel size={26} />
          )}
        </button>
        <ColorPopover
          id={popoverId}
          open={!!editing}
          anchor={swatch}
          hex={draft ?? own ?? FIRST_OWN}
          preview={<AvatarThemes name={w.name} hex={draft ?? own ?? FIRST_OWN} />}
          used={used}
          workspace={w.name}
          focusHex={editing === "keyboard"}
          onChange={(hex) => {
            setDraft(hex);
            setOwn(hex);
            setChosen(true);
          }}
          onClose={close}
        />
      </div>
      {w.avatar_url && (
        <p className="mt-2 text-[12px] text-ink-4">
          Its image shows instead, until you use its initials again.
        </p>
      )}
    </fieldset>
  );
}

/** A tile of the hue wheel, as big as an avatar: the swatch for a color of its own, until there's one. */
export function Wheel({ size }: { size: number }) {
  return (
    <span
      aria-hidden="true"
      className="block shrink-0 shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
      style={{
        width: size,
        height: size,
        // As round at its corners as an avatar of its size.
        borderRadius: Math.max(4, Math.round(size * 0.24)),
        background: `radial-gradient(closest-side, rgb(255 255 255 / 0.85), rgb(255 255 255 / 0) 90%), conic-gradient(${RAINBOW})`,
      }}
    />
  );
}

/**
 * The avatar on each theme's background, where it keeps the color as chosen: unlike an icon's, it
 * fills a tile, which its hairline keeps apart from the background, and its initials take the ink
 * that reads on it.
 */
export function AvatarThemes({ name, hex }: { name: string; hex: string }) {
  return (
    <div className="flex items-center gap-2.5">
      <div
        className="flex h-8 shrink-0 overflow-hidden rounded-md shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
        // The words beside it say what it shows.
        aria-hidden="true"
      >
        {(
          [
            ["light", THEME_BACKGROUNDS.light],
            ["dark", "#1c1c1c"],
          ] as const
        ).map(([theme, bg]) => (
          <span key={theme} className="flex w-9 items-center justify-center" style={{ background: bg }}>
            <WorkspaceAvatar workspace={{ name, color: "yellow", hex }} size={20} title={false} />
          </span>
        ))}
      </div>
      <p className="min-w-0 text-[11.5px] leading-snug text-ink-4">As it is, in both themes</p>
    </div>
  );
}

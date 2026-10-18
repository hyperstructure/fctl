// A select in the app's own style. The trigger is a button with the keys of the
// WAI-ARIA select-only combobox; its list opens in the top layer, so it shows
// above dialogs and outside anything that clips. The list grows out of the
// trigger, a highlight glides after the pointer and the arrow keys, the picked
// option flashes before the list closes, and the new value rolls into place.

import {
  type CSSProperties,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { cn } from "@/lib/cn";

export interface SelectOption<T extends string = string> {
  value: T;
  /** Shown in the trigger and the list; typing its first letters jumps to it. */
  label: string;
  /** A quieter second line, in the list only. */
  description?: ReactNode;
  icon?: ReactNode;
  disabled?: boolean;
}

export interface SelectProps<T extends string> {
  value: T | undefined;
  options: readonly SelectOption<T>[];
  onChange: (value: T) => void;
  /** `md` is a form field beside an Input; `sm` sits in a row or a toolbar. */
  size?: "sm" | "md";
  id?: string;
  disabled?: boolean;
  /** Shown when no option has the value. */
  placeholder?: string;
  className?: string;
  "aria-label"?: string;
}

type Phase = "closed" | "open" | "closing";

/** How long the picked option flashes before the list closes, as menus do on macOS. */
const FLASH_MS = 160;
/** Letters typed within this long of each other make one search. */
const TYPEAHEAD_MS = 700;
/** Between the trigger and the list, and between the list and the window's edges. */
const GAP = 6;
const MARGIN = 8;
const MAX_HEIGHT = 384;

const sizes = {
  md:
    "h-9.5 gap-2 rounded-md border border-line-strong bg-sunken/60 pr-2.5 pl-3 text-[14px] text-ink " +
    "hover:border-ink-4/60 active:scale-[0.99] " +
    "focus-visible:border-accent/70 focus-visible:bg-sunken focus-visible:shadow-[0_0_0_3px_var(--c-accent-soft)] " +
    "aria-expanded:border-accent/70 aria-expanded:bg-sunken aria-expanded:shadow-[0_0_0_3px_var(--c-accent-soft)]",
  sm:
    "h-7 gap-1.5 rounded-md border border-line px-2 text-[12.5px] text-ink-2 " +
    "hover:border-line-strong hover:bg-raised hover:text-ink " +
    "aria-expanded:border-line-strong aria-expanded:bg-raised aria-expanded:text-ink " +
    "focus-visible:border-accent/70 focus-visible:shadow-[0_0_0_3px_var(--c-accent-soft)] active:scale-[0.97]",
};

export function Select<T extends string>({
  value,
  options,
  onChange,
  size = "md",
  id,
  disabled,
  placeholder,
  className,
  "aria-label": label,
}: SelectProps<T>) {
  const [phase, setPhase] = useState<Phase>("closed");
  const [active, setActive] = useState(-1);
  const [picked, setPicked] = useState<T | null>(null);
  const trigger = useRef<HTMLButtonElement>(null);
  const list = useRef<HTMLDivElement>(null);
  const highlight = useRef<HTMLDivElement>(null);
  const items = useRef<(HTMLDivElement | null)[]>([]);
  const timer = useRef(0);
  /** A mouse press that opened the list is held: releasing it over an option picks it. */
  const pressed = useRef(false);
  /** The keys moved the highlight, so the list scrolls to it. */
  const reveal = useRef(false);
  const typed = useRef({ text: "", at: 0 });
  const uid = useId();
  const listId = `${uid}-list`;
  const optionId = (i: number) => `${uid}-${i}`;
  const selected = options.findIndex((o) => o.value === value);
  const current = options[selected];
  const mounted = phase !== "closed";
  const open = phase === "open";

  const show = (at: number) => {
    if (disabled) return;
    window.clearTimeout(timer.current);
    setPicked(null);
    setActive(at);
    reveal.current = at >= 0;
    setPhase("open");
  };
  const hide = () => {
    window.clearTimeout(timer.current);
    setPhase((p) => (p === "open" ? "closing" : p));
  };
  const done = () => {
    setPhase("closed");
    setActive(-1);
    setPicked(null);
  };
  const pick = (i: number) => {
    const o = options[i];
    if (!o || o.disabled || picked !== null) return;
    setActive(i);
    setPicked(o.value);
    if (o.value !== value) onChange(o.value);
    const flash = matchMedia("(prefers-reduced-motion: reduce)").matches ? 0 : FLASH_MS;
    timer.current = window.setTimeout(() => setPhase("closing"), flash);
  };
  const move = (i: number) => {
    reveal.current = true;
    setActive(i);
  };
  const search = (key: string, from: number) => {
    const now = Date.now();
    const t = typed.current;
    t.text = now - t.at > TYPEAHEAD_MS ? key : t.text + key;
    t.at = now;
    return typeahead(
      options.map((o) => (o.disabled ? null : o.label)),
      t.text,
      from,
    );
  };

  // The list joins the top layer and sits by its trigger, following it as the page scrolls.
  // It closes when the trigger loses focus, or on a press elsewhere (iOS keeps focus on taps).
  useLayoutEffect(() => {
    const el = list.current;
    const t = trigger.current;
    if (!mounted || !el || !t) return;
    try {
      el.showPopover();
    } catch {
      // Without the Popover API the list is a fixed element where it is.
    }
    place(t, el);
    const chosen = el.querySelector<HTMLElement>('[aria-selected="true"]');
    if (chosen) el.scrollTop = chosen.offsetTop - (el.clientHeight - chosen.offsetHeight) / 2;
    let frame = 0;
    const follow = (e: Event) => {
      if (e.target === el) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(() => place(t, el));
    };
    const outside = (e: PointerEvent) => {
      const target = e.target as Node;
      if (el.contains(target) || t.contains(target)) return;
      window.clearTimeout(timer.current);
      setPhase((p) => (p === "open" ? "closing" : p));
    };
    window.addEventListener("resize", follow);
    window.addEventListener("scroll", follow, true);
    document.addEventListener("pointerdown", outside, true);
    return () => {
      cancelAnimationFrame(frame);
      window.removeEventListener("resize", follow);
      window.removeEventListener("scroll", follow, true);
      document.removeEventListener("pointerdown", outside, true);
    };
  }, [mounted]);

  // The highlight glides between options; it appears in place when it was hidden.
  // biome-ignore lint/correctness/useExhaustiveDependencies: measures the DOM after the list mounts
  useLayoutEffect(() => {
    const h = highlight.current;
    const box = list.current;
    if (!h || !box) return;
    const el = active >= 0 ? items.current[active] : null;
    if (!el) {
      h.style.opacity = "0";
      return;
    }
    h.style.transitionProperty = h.style.opacity === "1" ? "translate, height, opacity" : "opacity";
    h.style.translate = `0 ${el.offsetTop}px`;
    h.style.height = `${el.offsetHeight}px`;
    h.style.opacity = "1";
    if (reveal.current) {
      reveal.current = false;
      const top = el.offsetTop - 4;
      const bottom = el.offsetTop + el.offsetHeight + 4;
      if (top < box.scrollTop) box.scrollTop = top;
      else if (bottom > box.scrollTop + box.clientHeight) box.scrollTop = bottom - box.clientHeight;
    }
  }, [active, mounted]);

  // The closing animation ends the list; this is for when it can't run.
  useEffect(() => {
    if (phase !== "closing") return;
    const t = window.setTimeout(() => {
      setPhase("closed");
      setActive(-1);
      setPicked(null);
    }, 400);
    return () => window.clearTimeout(t);
  }, [phase]);

  useEffect(() => {
    if (disabled) setPhase((p) => (p === "open" ? "closing" : p));
  }, [disabled]);

  useEffect(() => () => window.clearTimeout(timer.current), []);

  const onKeyDown = (e: KeyboardEvent<HTMLButtonElement>) => {
    const { key } = e;
    const char = key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey;
    if (!open) {
      if (key === "ArrowDown" || key === "ArrowUp" || key === "Enter" || key === " ") {
        e.preventDefault();
        show(selected >= 0 ? selected : step(options, -1, key === "ArrowUp" ? -1 : 1));
      } else if (key === "Home" || key === "End") {
        e.preventDefault();
        show(step(options, -1, key === "Home" ? 1 : -1));
      } else if (char) {
        e.preventDefault();
        const found = search(key, selected);
        show(found >= 0 ? found : selected);
      }
      return;
    }
    if (key === "Escape") {
      // Closes the list, not the dialog it is in.
      e.preventDefault();
      e.stopPropagation();
      hide();
      return;
    }
    if (key === "Tab") {
      hide();
      return;
    }
    if (picked !== null) {
      e.preventDefault();
      return;
    }
    const typing = Date.now() - typed.current.at < TYPEAHEAD_MS;
    if (key === "ArrowDown" || key === "ArrowUp") {
      e.preventDefault();
      if (key === "ArrowUp" && e.altKey) pick(active);
      else move(step(options, active, key === "ArrowDown" ? 1 : -1));
    } else if (key === "PageDown" || key === "PageUp") {
      e.preventDefault();
      move(step(options, active, key === "PageDown" ? 10 : -10));
    } else if (key === "Home" || key === "End") {
      e.preventDefault();
      move(step(options, -1, key === "Home" ? 1 : -1));
    } else if (key === "Enter" || (key === " " && !typing)) {
      e.preventDefault();
      if (active >= 0) pick(active);
      else hide();
    } else if (char) {
      e.preventDefault();
      const found = search(key, active);
      if (found >= 0) move(found);
    }
  };

  return (
    <>
      <button
        ref={trigger}
        id={id}
        type="button"
        role="combobox"
        aria-haspopup="listbox"
        aria-expanded={open}
        aria-controls={mounted ? listId : undefined}
        aria-activedescendant={open && active >= 0 ? optionId(active) : undefined}
        aria-label={label}
        disabled={disabled}
        className={cn(
          "group/select relative inline-flex min-w-0 shrink-0 items-center text-left outline-none select-none",
          "transition-[border-color,box-shadow,background-color,color,scale] duration-150 ease-out",
          "disabled:pointer-events-none disabled:opacity-60",
          sizes[size],
          className,
        )}
        onPointerDown={(e) => {
          // Touch and pens open on click, so a swipe that starts here still scrolls the page.
          if (e.pointerType !== "mouse" || e.button !== 0 || e.ctrlKey) return;
          // Focus stays here, where the keys work (Safari doesn't focus buttons on click).
          e.preventDefault();
          e.currentTarget.focus({ preventScroll: true, focusVisible: false });
          pressed.current = true;
          const release = () => setTimeout(() => (pressed.current = false));
          window.addEventListener("pointerup", release, { once: true, capture: true });
          if (open) hide();
          else show(-1);
        }}
        onClick={(e) => {
          if (pressed.current) return;
          e.currentTarget.focus({ preventScroll: true, focusVisible: e.detail === 0 });
          if (open) hide();
          // A click without a pointer comes from assistive technology: start on the selection.
          else show(e.detail === 0 ? selected : -1);
        }}
        onKeyDown={onKeyDown}
        onBlur={hide}
      >
        {current?.icon && <span className="shrink-0 text-ink-3 [&_svg]:size-3.5">{current.icon}</span>}
        <Value
          text={current?.label ?? (value || placeholder || "")}
          index={selected}
          // An empty value is a placeholder, as an option with value="" is in HTML.
          muted={!value}
          sizes={placeholder ? [...options.map((o) => o.label), placeholder] : options.map((o) => o.label)}
        />
        <Chevron />
      </button>
      {mounted && (
        <div
          ref={list}
          id={listId}
          role="listbox"
          popover="manual"
          aria-label={label}
          data-state={open ? "open" : "closed"}
          className={cn(
            "fixed inset-auto m-0 w-max max-w-[min(22rem,calc(100vw-16px))] overflow-y-auto overscroll-contain",
            "rounded-lg border border-line-strong bg-raised/95 p-1 text-ink outline-none backdrop-blur-xl",
            "shadow-[0_18px_50px_-12px_rgb(0_0_0/0.5),0_2px_8px_-2px_rgb(0_0_0/0.18)]",
            "data-[state=closed]:pointer-events-none data-[state=closed]:animate-listbox-out data-[state=open]:animate-listbox-in",
          )}
          // Keep focus on the trigger. (Cancelling pointerdown instead would cancel taps in WebKit.)
          onMouseDown={(e) => e.preventDefault()}
          onPointerLeave={() => picked === null && setActive(-1)}
          onAnimationEnd={(e) => {
            if (e.target === e.currentTarget && phase === "closing") done();
          }}
        >
          <div
            ref={highlight}
            aria-hidden="true"
            className={cn(
              "pointer-events-none absolute inset-x-1 top-0 rounded-md bg-line-strong/70 opacity-0",
              "duration-200 ease-[var(--ease-spring)]",
              picked !== null && "animate-listbox-flash",
            )}
          />
          {options.map((o, i) => {
            const checked = picked !== null ? o.value === picked : i === selected;
            return (
              // biome-ignore lint/a11y/useKeyWithClickEvents: the keys are handled on the trigger
              <div
                key={o.value}
                ref={(el) => {
                  items.current[i] = el;
                }}
                id={optionId(i)}
                role="option"
                aria-selected={checked}
                aria-disabled={o.disabled || undefined}
                tabIndex={-1}
                style={{ "--i": i } as CSSProperties}
                className={cn(
                  "relative flex cursor-default items-start gap-2.5 rounded-md py-1.5 pr-2 pl-2.5 pointer-coarse:py-2.5",
                  "transition-colors duration-150",
                  o.disabled ? "text-ink-4" : i === active || checked ? "text-ink" : "text-ink-2",
                )}
                onPointerMove={() => {
                  if (!o.disabled && picked === null && i !== active) setActive(i);
                }}
                onPointerUp={(e) => {
                  if (pressed.current && e.pointerType === "mouse") pick(i);
                }}
                onClick={() => pick(i)}
              >
                {o.icon && <span className="mt-0.5 shrink-0 text-ink-3 [&_svg]:size-3.5">{o.icon}</span>}
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] leading-5">{o.label}</span>
                  {o.description && (
                    <span className="block pb-0.5 text-[12px] leading-[1.45] text-ink-3">
                      {o.description}
                    </span>
                  )}
                </span>
                <Check
                  draw={picked === o.value}
                  className={cn(
                    "mt-[3px] size-3.5 shrink-0 text-accent transition-opacity duration-150",
                    checked ? "opacity-100" : "opacity-0",
                  )}
                />
              </div>
            );
          })}
        </div>
      )}
    </>
  );
}

/** The trigger's text: as wide as the longest of `sizes`, rolling up or down to a new value. */
function Value({
  text,
  index,
  muted,
  sizes,
}: {
  text: string;
  index: number;
  muted: boolean;
  sizes: string[];
}) {
  const [shown, setShown] = useState({ text, index, muted });
  const [roll, setRoll] = useState<{ from: string; muted: boolean; dir: 1 | -1 } | null>(null);
  if (shown.text !== text) {
    setShown({ text, index, muted });
    setRoll({ from: shown.text, muted: shown.muted, dir: index < shown.index ? -1 : 1 });
  }
  const dir = roll ? ({ "--roll": roll.dir } as CSSProperties) : undefined;
  return (
    <span className="grid min-w-0 flex-1 grid-cols-1 overflow-hidden">
      {sizes.map((l, i) => (
        <span key={i} aria-hidden="true" className="invisible col-start-1 row-start-1 truncate">
          {l}
        </span>
      ))}
      {roll && (
        <span
          key={`${roll.from}-out`}
          aria-hidden="true"
          style={dir}
          className={cn("col-start-1 row-start-1 animate-roll-out truncate", roll.muted && "text-ink-4")}
        >
          {roll.from}
        </span>
      )}
      <span
        key={text}
        style={dir}
        className={cn("col-start-1 row-start-1 truncate", roll && "animate-roll-in", muted && "text-ink-4")}
        onAnimationEnd={() => setRoll(null)}
      >
        {text}
      </span>
    </span>
  );
}

/** A chevron whose arms swing from pointing down to pointing up while the list is open. */
function Chevron() {
  const arm =
    "origin-center transition-[rotate] duration-300 ease-[var(--ease-spring)] [transform-box:fill-box]";
  return (
    <svg
      viewBox="0 0 16 16"
      aria-hidden="true"
      className="ml-auto size-3.5 shrink-0 overflow-visible text-ink-4 transition-colors duration-150 group-hover/select:text-ink-3 group-aria-expanded/select:text-ink-2"
    >
      <g fill="none" stroke="currentColor" strokeWidth={1.6} strokeLinecap="round">
        <path d="M4 6.5l4 4" className={cn(arm, "group-aria-expanded/select:-rotate-90")} />
        <path d="M12 6.5l-4 4" className={cn(arm, "group-aria-expanded/select:rotate-90")} />
      </g>
    </svg>
  );
}

/** A check that draws itself when `draw` turns on. */
function Check({ draw, className }: { draw: boolean; className?: string }) {
  return (
    <svg viewBox="0 0 16 16" aria-hidden="true" className={className}>
      <path
        d="M3.5 8.5l3 3 6-7"
        fill="none"
        stroke="currentColor"
        strokeWidth={1.75}
        strokeLinecap="round"
        strokeLinejoin="round"
        pathLength={1}
        className={cn("[stroke-dasharray:1]", draw && "animate-draw")}
      />
    </svg>
  );
}

/** Puts the list under its trigger, or over it when there is more room above. */
function place(trigger: HTMLElement, list: HTMLElement) {
  const r = trigger.getBoundingClientRect();
  const { clientWidth: vw, clientHeight: vh } = document.documentElement;
  list.style.minWidth = `${Math.min(r.width, vw - 2 * MARGIN)}px`;
  const height = list.scrollHeight + list.offsetHeight - list.clientHeight;
  const below = vh - r.bottom - GAP - MARGIN;
  const above = r.top - GAP - MARGIN;
  const down = height <= below || below >= above;
  list.style.maxHeight = `${Math.max(0, Math.min(down ? below : above, MAX_HEIGHT))}px`;
  list.style.top = down ? `${r.bottom + GAP}px` : "auto";
  list.style.bottom = down ? "auto" : `${vh - r.top + GAP}px`;
  const width = list.offsetWidth;
  let left = r.left;
  if (left + width > vw - MARGIN) left = r.right - width;
  left = Math.max(MARGIN, Math.min(left, vw - MARGIN - width));
  list.style.left = `${left}px`;
  // It grows out of the trigger.
  list.style.transformOrigin = `${Math.round(r.left + r.width / 2 - left)}px ${down ? "top" : "bottom"}`;
  list.style.setProperty("--listbox-y", down ? "-4px" : "4px");
}

/**
 * The option `delta` steps from `from` (-1 for none: the first or the last),
 * passing over disabled ones and stopping at the ends.
 */
export function step(options: readonly { disabled?: boolean }[], from: number, delta: number): number {
  const ok = (i: number) => options[i] !== undefined && !options[i]?.disabled;
  const dir = delta > 0 ? 1 : -1;
  const n = options.length;
  const target = from < 0 ? (dir > 0 ? 0 : n - 1) : Math.min(n - 1, Math.max(0, from + delta));
  for (let i = target; i >= 0 && i < n; i += dir) if (ok(i)) return i;
  for (let i = target; i >= 0 && i < n; i -= dir) if (ok(i)) return i;
  return from;
}

/**
 * The option whose label starts with what was typed, searching from `from`
 * and wrapping. Typing one letter again moves on to the next option with it.
 * Disabled options are `null`.
 */
export function typeahead(labels: readonly (string | null)[], text: string, from: number): number {
  const t = text.toLowerCase();
  const again = [...t].every((c) => c === t[0]);
  const needle = again ? t.slice(0, 1) : t;
  const start = again ? from + 1 : Math.max(from, 0);
  for (let k = 0; k < labels.length; k++) {
    const i = (start + k) % labels.length;
    if (labels[i]?.toLowerCase().startsWith(needle)) return i;
  }
  return -1;
}

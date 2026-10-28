// The field that adds repositories to a project, in its Repositories tab: it lists every
// repository of the account and the repositories the workspace may use, to tick several of, and
// shows the ticked ones in the field. One typed whole (owner/name, or its address on github.com)
// that isn't on the list can be ticked too.

import { Building2, Lock, X } from "lucide-react";
import {
  type CSSProperties,
  Fragment,
  type KeyboardEvent,
  type ReactNode,
  useEffect,
  useId,
  useLayoutEffect,
  useRef,
  useState,
} from "react";
import { GithubIcon, placeList, SelectChevron, step } from "@/components/ui";
import { cn } from "@/lib/cn";
import { CHOICES_SHOWN, type RepoChoice } from "@/lib/repos";

type Phase = "closed" | "open" | "closing";

/** GitHub's names ignore case. */
const same = (a: string, b: string) => a.toLowerCase() === b.toLowerCase();
const enabled = (c: RepoChoice) => !c.on && !c.elsewhere;

export function RepoPicker({
  id,
  choices,
  chosen,
  onChange,
  query,
  onQuery,
  loaded,
  status,
  placeholder,
  invalid,
  describedBy,
}: {
  /** The text field's, for its label. */
  id: string;
  /** What the list offers for `query` and `chosen` (`repoChoices`). */
  choices: readonly RepoChoice[];
  /** The ticked ones, as `repoValue`s, in the order they were ticked. */
  chosen: readonly string[];
  onChange: (chosen: string[]) => void;
  /** What's typed, which narrows the list down. */
  query: string;
  onQuery: (query: string) => void;
  /** The repositories the workspace may use have loaded: one typed that isn't among them says so. */
  loaded: boolean;
  /** A line under the list, such as that it's still loading. */
  status?: ReactNode;
  placeholder: string;
  invalid?: boolean;
  describedBy?: string;
}) {
  const [phase, setPhase] = useState<Phase>("closed");
  // The highlighted choice, by value; until the keys or the pointer move it, the closest match
  // to what's typed (`auto`).
  const [active, setActive] = useState<string | null>(null);
  const [auto, setAuto] = useState(false);
  const [drawn, setDrawn] = useState<string | null>(null);
  const [said, setSaid] = useState("");
  const box = useRef<HTMLDivElement>(null);
  const input = useRef<HTMLInputElement>(null);
  const popup = useRef<HTMLDivElement>(null);
  const listbox = useRef<HTMLDivElement>(null);
  const highlight = useRef<HTMLDivElement>(null);
  const items = useRef<(HTMLDivElement | null)[]>([]);
  /** The keys moved the highlight, so the list scrolls to it. */
  const reveal = useRef(false);
  const uid = useId();
  const listId = `${uid}-list`;
  const optionId = (i: number) => `${uid}-${i}`;

  const shown = choices.slice(0, CHOICES_SHOWN);
  const more = choices.length - shown.length;
  const typing = query.trim() !== "";
  const at =
    active !== null
      ? shown.findIndex((c) => c.value === active && enabled(c))
      : auto && typing
        ? shown.findIndex(enabled)
        : -1;
  const ticked = (c: RepoChoice) => !!c.on || chosen.some((v) => same(v, c.value));
  // Accounts head their choices when there are several, until something is typed.
  const sections = !typing && new Set(shown.map((c) => c.owner.toLowerCase())).size > 1;
  const notes = !!status || more > 0;
  const mounted = phase !== "closed" && (shown.length > 0 || notes);
  const open = phase === "open" && mounted;
  const steps = shown.map((c) => ({ disabled: !enabled(c) }));

  const show = () => setPhase("open");
  const hide = () => setPhase((p) => (p === "open" ? "closing" : p));
  const done = () => {
    setPhase("closed");
    setActive(null);
    setAuto(false);
  };
  const move = (i: number) => {
    reveal.current = true;
    setActive(shown[i]?.value ?? null);
  };
  const toggle = (c: RepoChoice) => {
    if (!enabled(c)) return;
    const on = chosen.some((v) => same(v, c.value));
    onChange(on ? chosen.filter((v) => !same(v, c.value)) : [...chosen, c.value]);
    setDrawn(on ? null : c.value);
    setSaid(on ? `${c.value} no longer chosen` : `${c.value} chosen`);
    // What was typed found it; the next Enter adds what's chosen.
    if (typing) {
      onQuery("");
      setActive(null);
      setAuto(false);
    }
  };
  const remove = (value: string) => {
    onChange(chosen.filter((v) => v !== value));
    setSaid(`${value} no longer chosen`);
  };

  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    const { key } = e;
    if (key === "ArrowDown" || key === "ArrowUp") {
      e.preventDefault();
      if (e.altKey) {
        if (key === "ArrowDown") show();
        else hide();
      } else if (!open) {
        show();
        move(step(steps, -1, key === "ArrowDown" ? 1 : -1));
      } else move(step(steps, at, key === "ArrowDown" ? 1 : -1));
    } else if ((key === "PageDown" || key === "PageUp") && open) {
      e.preventDefault();
      move(step(steps, at, key === "PageDown" ? 10 : -10));
    } else if (key === "Enter") {
      const c = open && at >= 0 ? shown[at] : undefined;
      // Without a highlighted choice, Enter adds what's chosen: the form takes it.
      if (c) {
        e.preventDefault();
        toggle(c);
      } else hide();
    } else if (key === "Escape") {
      if (open) {
        // Closes the list, not the dialog it is in.
        e.preventDefault();
        e.stopPropagation();
        hide();
      } else if (query) {
        e.preventDefault();
        onQuery("");
      }
    } else if (key === "Backspace" && !query) {
      const last = chosen.at(-1);
      if (last) {
        e.preventDefault();
        remove(last);
      }
    } else if (key === "Tab") hide();
  };

  // The list joins the top layer, as wide as the field and under it (over it when there is more
  // room above), and follows it as the page scrolls or the field grows. It closes on a press
  // elsewhere, which iOS doesn't blur the field for.
  const put = () => {
    const el = popup.current;
    const b = box.current;
    const list = listbox.current;
    if (!el || !b) return;
    el.style.width = `${Math.min(b.offsetWidth, document.documentElement.clientWidth - 16)}px`;
    // As tall as it would be with every choice in sight: the list scrolls inside it, and clearing
    // its max-height to measure that would scroll the list back to its top.
    placeList(b, el, el.offsetHeight + (list ? list.scrollHeight - list.clientHeight : 0));
  };
  // biome-ignore lint/correctness/useExhaustiveDependencies: listens while the list is mounted
  useLayoutEffect(() => {
    const el = popup.current;
    const b = box.current;
    if (!mounted || !el || !b) return;
    try {
      el.showPopover();
    } catch {
      // Without the Popover API the list is a fixed element where it is.
    }
    let frame = 0;
    const follow = (e: Event) => {
      if (e.target instanceof Node && el.contains(e.target)) return;
      cancelAnimationFrame(frame);
      frame = requestAnimationFrame(put);
    };
    const outside = (e: PointerEvent) => {
      const target = e.target as Node;
      if (el.contains(target) || b.contains(target)) return;
      hide();
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
  // The list changes as people type and choose, and the field grows with what's chosen.
  useLayoutEffect(() => {
    if (mounted) put();
  });

  // The highlight glides between choices, as in a Select; it appears in place when it was hidden.
  useLayoutEffect(() => {
    const h = highlight.current;
    const list = listbox.current;
    if (!h || !list) return;
    const el = at >= 0 ? items.current[at] : null;
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
      if (top < list.scrollTop) list.scrollTop = top;
      else if (bottom > list.scrollTop + list.clientHeight) list.scrollTop = bottom - list.clientHeight;
    }
  });

  // The closing animation ends the list; this is for when it can't run.
  useEffect(() => {
    if (phase !== "closing") return;
    const t = window.setTimeout(() => {
      setPhase("closed");
      setActive(null);
      setAuto(false);
    }, 400);
    return () => window.clearTimeout(t);
  }, [phase]);

  return (
    <>
      {/* A press anywhere on the field opens or closes the list; the keys are the text field's. */}
      {/* biome-ignore lint/a11y/noStaticElementInteractions: the text field in it takes the keys */}
      {/* biome-ignore lint/a11y/useKeyWithClickEvents: the text field in it takes the keys */}
      <div
        ref={box}
        className={cn(
          "group/select relative flex min-h-9.5 w-full min-w-0 cursor-text flex-wrap items-center gap-1 rounded-md border border-line-strong bg-sunken/60 py-[5px] pr-8 pl-1.5",
          "transition-[border-color,box-shadow,background-color] duration-150",
          "hover:border-ink-4/60 focus-within:border-accent/70 focus-within:bg-sunken focus-within:shadow-[0_0_0_3px_var(--c-accent-soft)]",
          "data-invalid:border-danger/60 data-invalid:focus-within:shadow-[0_0_0_3px_var(--c-danger-soft)]",
        )}
        data-invalid={invalid || undefined}
        // Focus stays in the text field.
        onMouseDown={(e) => {
          if (e.target !== input.current) e.preventDefault();
        }}
        onClick={(e) => {
          if (e.target instanceof Element && e.target.closest("button")) return;
          input.current?.focus();
          if (e.target === input.current) {
            if (!open) show();
          } else if (open) hide();
          else show();
        }}
      >
        {chosen.map((v) => (
          <span
            key={v}
            className="inline-flex h-6.5 max-w-full min-w-0 animate-pop items-center rounded-[5px] border border-line bg-hover pl-2 font-mono text-[12.5px] text-ink"
          >
            <span className="truncate">{v}</span>
            <button
              type="button"
              tabIndex={-1}
              aria-label={`Remove ${v} from the choice`}
              className="grid size-6 shrink-0 place-items-center rounded-[5px] text-ink-4 transition-colors hover:text-ink"
              onClick={() => remove(v)}
            >
              <X className="size-3" />
            </button>
          </span>
        ))}
        <input
          ref={input}
          id={id}
          type="text"
          role="combobox"
          aria-expanded={open}
          aria-controls={mounted ? listId : undefined}
          aria-autocomplete="list"
          aria-activedescendant={open && at >= 0 ? optionId(at) : undefined}
          aria-invalid={invalid || undefined}
          aria-describedby={describedBy}
          value={query}
          onChange={(e) => {
            onQuery(e.target.value);
            setActive(null);
            setAuto(true);
            show();
          }}
          onKeyDown={onKeyDown}
          onBlur={hide}
          placeholder={chosen.length ? undefined : placeholder}
          autoComplete="off"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          className="h-6.5 min-w-[8ch] flex-1 bg-transparent px-1.5 font-mono text-[13px] text-ellipsis text-ink outline-none placeholder:font-sans placeholder:text-[14px] placeholder:text-ink-4"
        />
        {/* By the first line, as what's chosen fills more. */}
        <SelectChevron open={open} className="absolute top-[11px] right-2.5" />
      </div>
      {mounted && (
        // biome-ignore lint/a11y/noStaticElementInteractions: it keeps focus in the text field
        <div
          ref={popup}
          popover="manual"
          data-state={open ? "open" : "closed"}
          className={cn(
            "fixed inset-auto m-0 flex max-w-[calc(100vw-16px)] flex-col overflow-hidden p-0",
            "rounded-lg border border-line-strong bg-raised/95 text-ink outline-none backdrop-blur-xl",
            "shadow-[0_18px_50px_-12px_rgb(0_0_0/0.5),0_2px_8px_-2px_rgb(0_0_0/0.18)]",
            "data-[state=closed]:pointer-events-none data-[state=closed]:animate-listbox-out data-[state=open]:animate-listbox-in",
          )}
          // Keep focus in the text field. (Cancelling pointerdown instead would cancel taps in WebKit.)
          onMouseDown={(e) => e.preventDefault()}
          onAnimationEnd={(e) => {
            if (e.target === e.currentTarget && phase === "closing") done();
          }}
        >
          <div
            ref={listbox}
            id={listId}
            role="listbox"
            aria-multiselectable="true"
            aria-label="Repositories"
            data-state={open ? "open" : "closed"}
            className={cn(
              "relative min-h-0 flex-1 overflow-y-auto overscroll-contain p-1",
              !shown.length && "hidden",
            )}
          >
            <div
              ref={highlight}
              aria-hidden="true"
              className="pointer-events-none absolute inset-x-1 top-0 rounded-md bg-line-strong/70 opacity-0 duration-200 ease-[var(--ease-spring)]"
            />
            {shown.map((c, i) => {
              const off = !enabled(c);
              const on = ticked(c);
              const heading = sections && !same(c.owner, shown[i - 1]?.owner ?? "");
              return (
                <Fragment key={c.value}>
                  {heading && (
                    <div
                      role="presentation"
                      className="px-2.5 pt-2.5 pb-1 text-[11.5px] font-medium text-ink-3 first:pt-1"
                    >
                      {c.owner}
                    </div>
                  )}
                  {/* biome-ignore lint/a11y/useKeyWithClickEvents: the text field takes the keys (aria-activedescendant) */}
                  <div
                    ref={(el) => {
                      items.current[i] = el;
                    }}
                    id={optionId(i)}
                    role="option"
                    aria-selected={on}
                    aria-disabled={off || undefined}
                    tabIndex={-1}
                    style={{ "--i": i } as CSSProperties}
                    className={cn(
                      "relative flex cursor-default items-start gap-2.5 rounded-md py-1.5 pr-2.5 pl-2.5 pointer-coarse:py-2.5",
                      "transition-colors duration-150",
                      off ? "text-ink-4" : i === at ? "text-ink" : "text-ink-2",
                    )}
                    onPointerMove={() => {
                      if (!off && active !== c.value) setActive(c.value);
                    }}
                    onClick={() => toggle(c)}
                  >
                    <Tick on={on} draw={drawn === c.value} muted={off} />
                    <span className="flex h-5 shrink-0 items-center text-ink-4 [&_svg]:size-3.5">
                      {!c.name ? <Building2 /> : c.private ? <Lock /> : <GithubIcon />}
                    </span>
                    <span className="min-w-0 flex-1">
                      <span className="flex items-baseline gap-3">
                        <span className="min-w-0 truncate font-mono text-[12.5px] leading-5">{c.value}</span>
                        {(c.on || c.elsewhere) && (
                          <span className="ml-auto shrink-0 text-[11.5px] text-ink-4">
                            {c.on === "primary" ? "Primary" : c.on ? "Added" : "Another account"}
                          </span>
                        )}
                      </span>
                      {!c.name && !off ? (
                        <span className="block pb-0.5 text-[12px] leading-[1.45] text-ink-3">
                          Every repository of {c.owner} the GitHub App can reach, read only
                        </span>
                      ) : c.typed && loaded && !c.elsewhere ? (
                        <span className="block pb-0.5 text-[12px] leading-[1.45] text-ink-3">
                          Not on the list: fctl checks it with GitHub as it adds it
                        </span>
                      ) : null}
                    </span>
                  </div>
                </Fragment>
              );
            })}
          </div>
          {notes && (
            <div
              className={cn(
                "flex flex-col gap-1 px-3 py-2 text-[12px] leading-[1.45] text-ink-3 [overflow-wrap:anywhere]",
                shown.length > 0 && "border-t border-line",
              )}
            >
              {status && <p>{status}</p>}
              {more > 0 && (
                <p>
                  {more} more: type to find {more === 1 ? "it" : "them"}.
                </p>
              )}
            </div>
          )}
        </div>
      )}
      <p className="sr-only" aria-live="polite">
        {said}
      </p>
    </>
  );
}

/** A box that a check draws itself into when the choice is ticked. */
function Tick({ on, draw, muted }: { on: boolean; draw: boolean; muted: boolean }) {
  return (
    <span
      aria-hidden="true"
      className={cn(
        "mt-[3px] grid size-3.5 shrink-0 place-items-center rounded-[4px] border transition-colors duration-150",
        on ? "border-accent bg-accent text-accent-ink" : "border-line-strong bg-sunken/60",
        muted && "opacity-50",
      )}
    >
      <svg viewBox="0 0 16 16" aria-hidden="true" className={cn("size-3", on ? "opacity-100" : "opacity-0")}>
        <path
          d="M3.5 8.5l3 3 6-7"
          fill="none"
          stroke="currentColor"
          strokeWidth={2.25}
          strokeLinecap="round"
          strokeLinejoin="round"
          pathLength={1}
          className={cn("[stroke-dasharray:1]", draw && "animate-draw")}
        />
      </svg>
    </span>
  );
}

// The keyboard shortcuts sheet (? or ⌘/ from anywhere, on a computer): every shortcut of the web
// app (lib/shortcuts.ts), grouped by where it works, those of the page it opened on first, with a
// filter. And the hint that shows, a moment after the first key of a sequence (G), what can follow.

import { Search, X } from "lucide-react";
import { Fragment, useEffect, useId, useLayoutEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { Kbd } from "@/components/ui";
import { cn } from "@/lib/cn";
import { own } from "@/lib/dialog";
import { useSequence } from "@/lib/hotkeys";
import {
  chordKeys,
  comboKeys,
  comboText,
  GROUPS,
  type GroupId,
  type Keys,
  SHORTCUTS,
  type Shortcut,
} from "@/lib/shortcuts";

export function ShortcutsDialog({
  open,
  onClose,
  here,
}: {
  open: boolean;
  onClose: () => void;
  /** The groups of the page it opens on, or of the dialog: they come first, marked. */
  here: readonly GroupId[];
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const titleId = useId();
  // What it shows mounts once it is open, so that the filter takes the focus (as in ui/overlay.tsx).
  const [shown, setShown] = useState(false);
  // A press that starts on the backdrop, not a selection dragged out of the sheet, closes it.
  const pressedOutside = useRef(false);
  useLayoutEffect(() => {
    const d = ref.current;
    if (!d) return;
    if (open && !d.open) d.showModal();
    if (!open && d.open) d.close();
    setShown(open);
  }, [open]);
  return createPortal(
    // biome-ignore lint/a11y/useKeyWithClickEvents: the click closes on the backdrop only; Escape is native to <dialog>.
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      onClose={own(onClose)}
      onMouseDown={(e) => {
        pressedOutside.current = e.target === ref.current;
      }}
      onClick={(e) => {
        if (e.target === ref.current && pressedOutside.current) ref.current?.close();
      }}
      className={cn(
        "top-(--viewport-shift) -bottom-(--viewport-shift) m-auto w-[min(860px,calc(100vw-2rem))] max-w-none overflow-hidden rounded-xl border border-line-strong bg-surface p-0 text-ink outline-none xl:w-[min(1180px,calc(100vw-4rem))]",
        "max-h-[min(760px,calc(100dvh-3rem))] shadow-[0_30px_120px_rgb(0_0_0/0.55)] open:flex open:flex-col",
        "backdrop:bg-black/45 backdrop:backdrop-blur-[3px] open:animate-rise motion-reduce:open:animate-none",
      )}
    >
      {open && shown && <Sheet titleId={titleId} here={here} onClose={() => ref.current?.close()} />}
    </dialog>,
    document.body,
  );
}

/** What the filter looks through for `s`: its words, its group's, and its keys as each system says them. */
function searchText(s: Shortcut): string {
  const group = GROUPS.find((g) => g.id === s.group)?.title ?? "";
  const keys = s.keys.flat().flatMap((combo) => [comboText(combo), ...comboKeys(combo).flat()]);
  return [s.label, s.note ?? "", group, ...keys].join(" ").toLowerCase();
}

function Sheet({
  titleId,
  here,
  onClose,
}: {
  titleId: string;
  here: readonly GroupId[];
  onClose: () => void;
}) {
  const [q, setQ] = useState("");
  const rows = useMemo(
    () => Object.entries(SHORTCUTS).map(([id, s]: [string, Shortcut]) => ({ id, s, text: searchText(s) })),
    [],
  );
  // Those of the page first, after what works anywhere: what it opened to look for.
  const order = useMemo(() => {
    const ids: GroupId[] = GROUPS.map((g) => g.id);
    const first: GroupId[] = ids.filter((id) => id === "anywhere" || id === "go");
    const mine = ids.filter((id) => here.includes(id) && !first.includes(id));
    return [...first, ...mine, ...ids.filter((id) => !first.includes(id) && !mine.includes(id))];
  }, [here]);
  const words = q.trim().toLowerCase().split(/\s+/).filter(Boolean);
  const groups = order
    .map((id) => ({
      id,
      title: GROUPS.find((g) => g.id === id)?.title ?? "",
      rows: rows.filter((r) => r.s.group === id && words.every((w) => r.text.includes(w))),
    }))
    .filter((g) => g.rows.length > 0);
  return (
    <>
      <header className="flex shrink-0 flex-wrap items-start gap-x-6 gap-y-3 border-b border-line px-6 pt-5 pb-4">
        <div className="min-w-0 flex-1 basis-64">
          <h2 id={titleId} className="font-display text-[22px] leading-tight tracking-[-0.01em] text-ink">
            Keyboard shortcuts
          </h2>
          <p className="mt-1 text-[13px] leading-relaxed text-ink-3">
            Single keys work while you aren't typing in a box, and Esc leaves the box you are in.
          </p>
        </div>
        <div className="flex items-center gap-2">
          <label className="flex h-8 w-52 items-center gap-2 rounded-md border border-line bg-sunken/60 px-2.5 text-[13px] transition-colors focus-within:border-line-strong">
            <Search aria-hidden="true" className="size-3.5 shrink-0 text-ink-4" />
            <input
              autoFocus
              type="text"
              value={q}
              onChange={(e) => setQ(e.target.value)}
              onKeyDown={(e) => {
                // Escape clears the filter before it closes the sheet.
                if (e.key === "Escape" && q) {
                  e.preventDefault();
                  setQ("");
                }
              }}
              placeholder="Filter"
              aria-label="Filter the shortcuts"
              autoComplete="off"
              spellCheck={false}
              className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-ink-4"
            />
          </label>
          <button
            type="button"
            aria-label="Close"
            onClick={onClose}
            className="-mr-2 rounded-md p-1.5 text-ink-4 transition-colors hover:bg-hover hover:text-ink"
          >
            <X className="size-4" />
          </button>
        </div>
      </header>
      {/* flex-auto, not flex-1: the dialog is as tall as its contents, and Safari 26 takes
          flex-1's 0% basis for 0 there (WebKit bug 289876), which left the list no height. */}
      <div className="min-h-0 flex-auto overflow-y-auto overscroll-contain px-6 pt-5 pb-2">
        {groups.length === 0 ? (
          <p className="py-10 text-center text-[13.5px] text-ink-4">No shortcut matches “{q.trim()}”.</p>
        ) : (
          <div className="gap-x-10 md:columns-2 xl:columns-3">
            {groups.map((g) => (
              <section key={g.id} aria-labelledby={`${titleId}-${g.id}`} className="mb-6 break-inside-avoid">
                <h3
                  id={`${titleId}-${g.id}`}
                  className="mb-1 flex items-center gap-2 text-[12.5px] font-medium text-ink-3"
                >
                  {g.title}
                  {here.includes(g.id) && (
                    <span className="inline-flex items-center gap-1.5 text-[11.5px] font-normal text-accent">
                      <span aria-hidden="true" className="size-1.5 rounded-full bg-accent" />
                      Here
                    </span>
                  )}
                </h3>
                <ul className="divide-y divide-line">
                  {g.rows.map((r) => (
                    <li key={r.id} className="flex items-center justify-between gap-4 py-[7px]">
                      {/* What it does keeps its line where it can: the keys give way first, and wrap. */}
                      <span className="min-w-0">
                        <span className="block text-[13.5px] text-ink-2">{r.s.label}</span>
                        {r.s.note && <span className="block text-[12px] text-ink-4">{r.s.note}</span>}
                      </span>
                      <KeyList keys={r.s.keys} />
                    </li>
                  ))}
                </ul>
              </section>
            ))}
          </div>
        )}
      </div>
      <footer className="shrink-0 border-t border-line px-6 py-3 text-[12px] leading-relaxed text-ink-4">
        Shortcuts follow your keyboard's layout: for ? or /, press the keys that type them, such as Shift and
        7 for / on a Swedish or German keyboard.
      </footer>
    </>
  );
}

/** A word between keys; `hidden` from screen readers, where the keys' text says it already. */
function Between({ children, hidden }: { children: string; hidden?: boolean }) {
  return (
    <span aria-hidden={hidden} className="px-0.5 text-[11.5px] text-ink-4">
      {children}
    </span>
  );
}

/**
 * The ways to press a shortcut: any of them ("or"), each keys for what its label lists ("/").
 * Where they don't fit beside the label, the next way goes under the first.
 */
export function KeyList({ keys }: { keys: readonly Keys[] }) {
  return (
    <span className="flex shrink-[100] flex-wrap items-center justify-end gap-x-1 gap-y-1.5">
      {keys.map((k, i) => (
        <span
          key={typeof k === "string" ? k : k.join()}
          className="inline-flex items-center gap-1 whitespace-nowrap"
        >
          {i > 0 && <Between>or</Between>}
          {typeof k === "string" ? (
            <Combo combo={k} />
          ) : (
            k.map((combo, j) => (
              <Fragment key={combo}>
                {j > 0 && <Between>/</Between>}
                <Combo combo={combo} />
              </Fragment>
            ))
          )}
        </span>
      ))}
    </span>
  );
}

/** One way to press it: each step's keys together, the steps of a sequence one "then" the other. */
export function Combo({ combo, size = "md" }: { combo: string; size?: "sm" | "md" }) {
  return (
    <span className="inline-flex items-center gap-1">
      <span className="sr-only">{comboText(combo)}</span>
      {comboKeys(combo).map((caps, i) => (
        <Fragment key={i}>
          {i > 0 && <Between hidden>then</Between>}
          <span aria-hidden="true" className="inline-flex items-center gap-0.5">
            {caps.map((cap, j) => (
              <Kbd key={j} size={size}>
                {cap}
              </Kbd>
            ))}
          </span>
        </Fragment>
      ))}
    </span>
  );
}

/**
 * After the first key of a sequence (G), a moment later, what can follow it and where each goes,
 * at the foot of the window: for whoever doesn't know yet.
 */
export function SequenceHint() {
  const sequence = useSequence();
  const first = sequence?.first;
  const [late, setLate] = useState(false);
  useEffect(() => {
    setLate(false);
    if (!first) return;
    const t = setTimeout(() => setLate(true), 450);
    return () => clearTimeout(t);
  }, [first]);
  if (!sequence || !late || !sequence.next.length) return null;
  const listed = Object.values(SHORTCUTS).map((x: Shortcut) => x.label);
  const next = [...sequence.next].sort(
    (a, b) => listed.indexOf(a.label ?? "") - listed.indexOf(b.label ?? ""),
  );
  return createPortal(
    <div
      role="status"
      className="pointer-events-none fixed inset-x-0 bottom-[calc(max(1.5rem,env(safe-area-inset-bottom))-var(--viewport-shift))] z-50 flex justify-center px-4"
    >
      <div className="flex max-w-full animate-rise flex-wrap items-center gap-x-4 gap-y-2 rounded-xl border border-line-strong bg-surface/95 px-4 py-2.5 text-[12.5px] shadow-[0_18px_60px_rgb(0_0_0/0.4)] backdrop-blur-xl motion-reduce:animate-none">
        <span className="inline-flex items-center gap-1.5 text-ink-4">
          <span className="inline-flex gap-0.5">
            {chordKeys(sequence.first).map((cap) => (
              <Kbd key={cap} size="md">
                {cap}
              </Kbd>
            ))}
          </span>
          then
        </span>
        {next.map(({ step, label }) => (
          <span key={step.key} className="inline-flex items-center gap-1.5 text-ink-2">
            <span className="inline-flex gap-0.5">
              {chordKeys(step).map((cap) => (
                <Kbd key={cap} size="md">
                  {cap}
                </Kbd>
              ))}
            </span>
            {label}
          </span>
        ))}
      </div>
    </div>,
    document.body,
  );
}

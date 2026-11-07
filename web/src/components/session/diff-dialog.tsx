// The session's changes over its page: opened from a file of its Changes
// list, every changed file's diff, highlighted, one after the other in one
// scroll, at the file opened. The list of files beside them marks the one
// in view and goes to another; keys step from file to file and change to
// change; ⌘F finds in all of them. It follows the agent: when a file
// changes again, its diff does too, where the reader looks, while the line
// they read holds its place (session/diff-live.tsx).

import { useQueryClient } from "@tanstack/react-query";
import {
  ArrowUpRight,
  Binary,
  Check,
  ChevronDown,
  ChevronRight,
  ChevronUp,
  Columns2,
  Copy,
  FilePen,
  FileQuestion,
  FileX2,
  History,
  PanelLeft,
  Rows3,
  Search,
  TextWrap,
  X,
} from "lucide-react";
import {
  type KeyboardEvent,
  memo,
  type ReactNode,
  useCallback,
  useEffect,
  useId,
  useLayoutEffect,
  useMemo,
  useRef,
  useState,
} from "react";
import { createPortal } from "react-dom";
import { type DiffLayout, DiffLines, type LiveView } from "@/components/diff";
import { Button, CodeBox, IconButton, Skeleton } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure } from "@/lib/api";
import { cn } from "@/lib/cn";
import { own } from "@/lib/dialog";
import { changeStarts, firstLine, type PatchLine, parsePatch, patchLines } from "@/lib/diff";
import { compare, type DiffMatch, type DiffPlace, type FindOptions } from "@/lib/diff-find";
import { useHoldPage } from "@/lib/hold-page";
import { apple, modKey } from "@/lib/hotkeys";
import { languageOf } from "@/lib/languages";
import { fileDiffQuery, useFileDiff, useSessionDiffs } from "@/lib/queries";
import { diffCommand } from "@/lib/shell";
import {
  byDirectory,
  changeOf,
  type DiffGroup,
  type DiffTarget,
  entryKey,
  FileList,
  splitPath,
} from "./diff-files";
import { DiffFindBar, matchRanges, sectionOf, TOGGLES, useDiffFind } from "./diff-find";
import { Rolling, UnseenChanges, useChanged, useLive } from "./diff-live";

export { byDirectory, changeOf, type DiffGroup, type DiffTarget };

/** A preference this browser keeps, such as how diffs are laid out. */
function useKept<T extends string>(key: string, initial: T, allowed: readonly T[]): [T, (v: T) => void] {
  const [value, setValue] = useState<T>(() => {
    try {
      const v = localStorage.getItem(key) as T | null;
      return v && allowed.includes(v) ? v : initial;
    } catch {
      return initial;
    }
  });
  const set = useCallback(
    (v: T) => {
      setValue(v);
      try {
        localStorage.setItem(key, v);
      } catch {}
    },
    [key],
  );
  return [value, set];
}

/** Whether the window is at least this wide (px). */
function useWide(px: number) {
  const query = `(min-width: ${px}px)`;
  const [wide, setWide] = useState(() => window.matchMedia?.(query).matches ?? true);
  useEffect(() => {
    const m = window.matchMedia?.(query);
    if (!m) return;
    const on = () => setWide(m.matches);
    m.addEventListener("change", on);
    return () => m.removeEventListener("change", on);
  }, [query]);
  return wide;
}

/** A changed file as the dialog shows it: its key ([`entryKey`]), repository and change. */
type Entry = {
  key: string;
  repo?: string;
  file: G.FileChange;
  /** Where it is on GitHub, on the session's branch, while the branch is pushed. */
  blob?: string;
};

/** The same file, changed the same way, there the same: what it showed is still right. */
const sameEntry = (a: Entry, b: Entry) =>
  a.key === b.key &&
  a.blob === b.blob &&
  a.file.status === b.file.status &&
  a.file.digest === b.file.digest &&
  a.file.additions === b.file.additions &&
  a.file.deletions === b.file.deletions;

/** How tall a file's header is, which stays at the top while its lines scroll under it (px). */
const FILE_HEADER = 40;
/** A line of a diff, and a hunk's `@@` line (px): for the room a file not shown yet takes. */
const LINE = 20;
const HUNK = 28;
/** How far from the view files start showing their lines, so that they have them once there (px). */
const AHEAD = 1600;
/** Where a change the reader steps to goes: this far under its file's header, its context above it (px). */
const CHANGE_MARGIN = 3 * LINE;
/** Keys that scroll the diffs, after which the file the reader went to is no longer held at the top. */
const SCROLL_KEYS = new Set(["ArrowUp", "ArrowDown", "PageUp", "PageDown", "Home", "End", " "]);

/**
 * Calls back once an element comes within `AHEAD` px of what `root` shows,
 * one observer for all of them: each file starts showing its lines there.
 * Without IntersectionObserver, at once.
 */
function useNear(root: React.RefObject<HTMLElement | null>) {
  const [watched] = useState(() => new Map<Element, () => void>());
  const observer = useRef<IntersectionObserver | undefined>(undefined);
  useEffect(() => {
    if (typeof IntersectionObserver === "undefined") return;
    const io = new IntersectionObserver(
      (entries) => {
        for (const e of entries) {
          if (!e.isIntersecting) continue;
          watched.get(e.target)?.();
          watched.delete(e.target);
          io.unobserve(e.target);
        }
      },
      { root: root.current, rootMargin: `${AHEAD}px 0px` },
    );
    observer.current = io;
    for (const el of watched.keys()) io.observe(el);
    return () => io.disconnect();
  }, [root, watched]);
  return useCallback(
    (el: Element, near: () => void) => {
      if (typeof IntersectionObserver === "undefined") {
        near();
        return () => {};
      }
      watched.set(el, near);
      observer.current?.observe(el);
      return () => {
        watched.delete(el);
        observer.current?.unobserve(el);
      };
    },
    [watched],
  );
}

/** Where to bring a line of a file: a match in it, or the start of a change. */
type Want = DiffPlace & { start?: number; end?: number; how: "match" | "change"; seq: number };

/**
 * The session's changes over the page, on the native modal <dialog>:
 * Escape, the X or a click beside it closes it, and the focus goes back to
 * what opened it, without the ring a keyboard's focus has when a click
 * opened it. Every file's diff is in it, one after the other, from the file
 * `initial`. `j` and `k` (or the arrows in its header, or those of the list
 * at its side) go from file to file, `n` and `p` from change to change; ⌘F
 * finds in all of them; `s` lays the diffs out side by side, `w` wraps long
 * lines.
 */
export function DiffDialog({
  session,
  cliRef,
  groups,
  initial,
  onClose,
}: {
  session: string;
  /** How `fctl` names the session (`project#12`), for the command that prints a diff too large to show. */
  cliRef: string;
  groups: DiffGroup[];
  /** The file it opens at. */
  initial: DiffTarget;
  onClose: () => void;
}) {
  const ref = useRef<HTMLDialogElement>(null);
  const body = useRef<HTMLDivElement>(null);
  const column = useRef<HTMLDivElement>(null);
  const titleId = useId();
  const [layout, setLayout] = useKept<DiffLayout>("fctl.diff.layout", "unified", ["unified", "split"]);
  // On a phone's narrow screen long lines wrap, unless someone chose otherwise.
  const narrow = window.matchMedia?.("(max-width: 639px)").matches ?? false;
  const [wrap, setWrap] = useKept<"wrap" | "scroll">("fctl.diff.wrap", narrow ? "wrap" : "scroll", [
    "wrap",
    "scroll",
  ]);
  const [listShown, setListShown] = useKept<"shown" | "hidden">("fctl.diff.list", "shown", [
    "shown",
    "hidden",
  ]);
  const onOff = ["on", "off"] as const;
  const [matchCase, setMatchCase] = useKept("fctl.diff.find.case", "off", onOff);
  const [wholeWord, setWholeWord] = useKept("fctl.diff.find.word", "off", onOff);
  const [regex, setRegex] = useKept("fctl.diff.find.regex", "off", onOff);
  const options: FindOptions = useMemo(
    () => ({ matchCase: matchCase === "on", wholeWord: wholeWord === "on", regex: regex === "on" }),
    [matchCase, wholeWord, regex],
  );
  const toggle = (o: keyof FindOptions) => {
    const set = { matchCase: setMatchCase, wholeWord: setWholeWord, regex: setRegex }[o];
    set(options[o] ? "off" : "on");
  };
  // On a phone the list covers the diffs while it is open.
  const [listOpen, setListOpen] = useState(false);
  const wide = useWide(1024);
  // Side by side takes room: below it, lines go one below the other.
  const roomy = useWide(900);
  const shownLayout: DiffLayout = roomy ? layout : "unified";
  const pressedOutside = useRef(false);

  // In the order the list shows them. The page lists the files anew at each step the agent
  // takes: a file that didn't change keeps its entry, and its diff isn't drawn again.
  const kept = useRef(new Map<string, Entry>());
  const all: Entry[] = useMemo(() => {
    const next = groups.flatMap((g) =>
      byDirectory(g.files).flatMap((d) =>
        d.files.map((file): Entry => {
          const key = entryKey(g.repo, file.path);
          const blob = file.status !== "deleted" ? g.blob?.(file.path) : undefined;
          const e: Entry = { key, repo: g.repo, file, blob };
          const before = kept.current.get(key);
          return before && sameEntry(before, e) ? before : e;
        }),
      ),
    );
    kept.current = new Map(next.map((e) => [e.key, e]));
    return next;
  }, [groups]);
  const indexOf = useMemo(() => new Map(all.map((e, i) => [e.key, i])), [all]);
  const several = groups.length > 1;

  // Every diff at once, in one request, unless the cache has them all.
  const qc = useQueryClient();
  const [needAll] = useState(() =>
    all.some((e) => !!e.file.digest && qc.getQueryData(fileDiffQuery(session, e).queryKey) === undefined),
  );
  const bulk = useSessionDiffs(session, needAll);
  const waiting = needAll && bulk.isPending;

  // The diffs the files show, for finding in them and stepping from change to change.
  const shown = useRef(new Map<string, G.FileDiff>());
  const [version, setVersion] = useState(0);
  const bumping = useRef(false);
  const onDiff = useCallback((key: string, d: G.FileDiff | undefined) => {
    if (shown.current.get(key) === d) return;
    if (d) shown.current.set(key, d);
    else shown.current.delete(key);
    if (bumping.current) return;
    bumping.current = true;
    queueMicrotask(() => {
      bumping.current = false;
      setVersion((v) => v + 1);
    });
  }, []);
  const parsed = useRef(new Map<string, PatchLine[]>());
  const lastLines = useRef<(PatchLine[] | undefined)[]>([]);
  // biome-ignore lint/correctness/useExhaustiveDependencies: `version` says the diffs changed
  const lines = useMemo(() => {
    const next = new Map<string, PatchLine[]>();
    const out = all.map((e) => {
      const d = shown.current.get(e.key);
      if (!d?.patch) return undefined;
      const k = `${e.key}\u0001${d.digest}`;
      const l = parsed.current.get(k) ?? patchLines(d.patch);
      next.set(k, l);
      return l;
    });
    parsed.current = next;
    // The same lines: finding doesn't look through them again.
    const before = lastLines.current;
    if (out.length === before.length && out.every((l, i) => l === before[i])) return before;
    lastLines.current = out;
    return out;
  }, [all, version]);
  const changes = useMemo(
    () => lines.flatMap((l, file) => (l ? changeStarts(l).map((c) => ({ file, h: c.h, j: c.j })) : [])),
    [lines],
  );

  // The file in view, and the one the reader went to, which stays at the top while files
  // above it take their room, until they scroll.
  const [active, setActive] = useState(() => entryKey(initial.repo, initial.path));
  const activeIndex = indexOf.get(active) ?? (all.length ? 0 : -1);
  const pinned = useRef<string | undefined>(indexOf.has(active) ? active : undefined);
  // Files whose lines show wherever they are, for what the reader goes to; folded files.
  const [forced, setForced] = useState<ReadonlySet<string>>(() => new Set([active]));
  const [folded, setFolded] = useState<ReadonlySet<string>>(() => new Set());
  const watch = useNear(body);
  // The reader's place, which holds as the agent changes the files, and what it changed out of sight;
  // the file the reader went to stays at the top as files above it show their lines.
  const pinnedSection = useCallback(() => {
    const i = pinned.current === undefined ? undefined : indexOf.get(pinned.current);
    return i === undefined || !body.current ? undefined : sectionOf(body.current, i);
  }, [indexOf]);
  const {
    view: live,
    pick,
    counts: unseen,
    nearest,
    look,
  } = useLive(body, column, FILE_HEADER, pinnedSection);
  // Files the agent changed for the first time while the dialog shows: their lines come in.
  const [atOpen] = useState(() => new Set(all.map((e) => e.key)));
  // The change the reader stepped to, and where the diffs were then.
  const changeAt = useRef<{ index: number; top: number } | undefined>(undefined);

  // The focus goes back, on closing, to what opened it: with its ring only when the keyboard
  // did. Read as the dialog first renders, before it takes the focus (and before development's
  // second run of the effect below, which finds the focus in it).
  const [quiet] = useState(() => {
    const opener = document.activeElement;
    if (!(opener instanceof HTMLElement) || opener === document.body) return undefined;
    try {
      return opener.matches(":focus-visible") ? undefined : opener;
    } catch {
      return undefined;
    }
  });
  useEffect(() => {
    if (quiet) quiet.dataset.quietFocus = "";
    ref.current?.setAttribute("autofocus", "");
    if (!ref.current?.open) ref.current?.showModal();
    body.current?.focus({ preventScroll: true });
    return () => {
      if (!quiet) return;
      // Until it loses the focus it got back.
      if (document.activeElement === quiet)
        quiet.addEventListener("blur", () => delete quiet.dataset.quietFocus, { once: true });
      else delete quiet.dataset.quietFocus;
    };
  }, [quiet]);
  useHoldPage();

  const letGo = useCallback(() => {
    pinned.current = undefined;
    changeAt.current = undefined;
  }, []);

  /** Goes to a file: its header at the top. */
  const jumpTo = useCallback(
    (key: string) => {
      const b = body.current;
      const i = indexOf.get(key);
      if (!b || i === undefined) return;
      letGo();
      pinned.current = key;
      setActive(key);
      const s = sectionOf(b, i);
      if (s) b.scrollTo({ top: s.offsetTop });
      pick();
    },
    [indexOf, letGo, pick],
  );

  // It opens at the file opened.
  // biome-ignore lint/correctness/useExhaustiveDependencies: once
  useLayoutEffect(() => {
    if (pinned.current) jumpTo(pinned.current);
  }, []);

  // The file in view: the one under the top of the view, or the one the reader went to.
  useEffect(() => {
    const b = body.current;
    if (!b) return;
    let raf = 0;
    const spy = () => {
      raf = 0;
      if (pinned.current !== undefined) return;
      const sections = b.querySelectorAll<HTMLElement>("section[data-file]");
      const top = b.scrollTop + 1;
      let lo = 0;
      let hi = sections.length - 1;
      while (lo < hi) {
        const mid = (lo + hi + 1) >> 1;
        if ((sections[mid]?.offsetTop ?? 0) <= top) lo = mid;
        else hi = mid - 1;
      }
      const e = all[Number(sections[lo]?.dataset.file)];
      if (e) setActive(e.key);
    };
    const onScroll = () => {
      if (!raf) raf = requestAnimationFrame(spy);
    };
    const onKey = (e: globalThis.KeyboardEvent) => {
      if (SCROLL_KEYS.has(e.key) && e.target === b) letGo();
    };
    b.addEventListener("scroll", onScroll, { passive: true });
    b.addEventListener("wheel", letGo, { passive: true });
    b.addEventListener("touchstart", letGo, { passive: true });
    b.addEventListener("pointerdown", letGo);
    b.addEventListener("keydown", onKey);
    return () => {
      b.removeEventListener("scroll", onScroll);
      b.removeEventListener("wheel", letGo);
      b.removeEventListener("touchstart", letGo);
      b.removeEventListener("pointerdown", letGo);
      b.removeEventListener("keydown", onKey);
      cancelAnimationFrame(raf);
    };
  }, [all, letGo]);

  /** To the next file, or back: back goes to the top of the file in view first, when the reader is down in it. */
  const go = useCallback(
    (by: -1 | 1) => {
      const b = body.current;
      const s = b && sectionOf(b, activeIndex);
      const e = all[activeIndex];
      if (by === -1 && b && s && e && pinned.current === undefined && b.scrollTop > s.offsetTop + 4) {
        jumpTo(e.key);
        return;
      }
      const next = all[activeIndex + by];
      if (next) jumpTo(next.key);
    },
    [all, activeIndex, jumpTo],
  );

  /** The line at this far under the top of the view (under the header of the file there). */
  const placeAt = useCallback((below = 0): DiffPlace | undefined => {
    const b = body.current;
    if (!b) return undefined;
    const y = b.getBoundingClientRect().top + FILE_HEADER + below + 1;
    const first = <T extends Element>(list: ArrayLike<T>) => {
      let lo = 0;
      let hi = list.length;
      while (lo < hi) {
        const mid = (lo + hi) >> 1;
        if ((list[mid]?.getBoundingClientRect().bottom ?? 0) <= y) lo = mid + 1;
        else hi = mid;
      }
      return lo;
    };
    const sections = b.querySelectorAll<HTMLElement>("section[data-file]");
    const s = sections[first(sections)];
    if (!s) return undefined;
    const file = Number(s.dataset.file);
    const rows = s.querySelectorAll<HTMLElement>("[data-l]");
    if (!rows.length) return { file, h: 0, j: 0 };
    const row = rows[first(rows)];
    if (!row) return { file: file + 1, h: 0, j: 0 };
    const [h = 0, j = 0] = (row.dataset.l ?? "").split(":").map(Number);
    return { file, h, j };
  }, []);
  const placeAtTop = useCallback(() => placeAt(0), [placeAt]);

  /** Whether a line shows, under its file's header. */
  const inView = useCallback((p: DiffPlace) => {
    const b = body.current;
    const row = b && sectionOf(b, p.file)?.querySelector(`[data-l="${p.h}:${p.j}"]`);
    if (!b || !row) return false;
    const r = row.getBoundingClientRect();
    const v = b.getBoundingClientRect();
    return r.bottom > v.top + FILE_HEADER && r.top < v.bottom;
  }, []);

  // Where to bring a line, once its file shows it.
  const [want, setWant] = useState<Want>();
  const reveal = useCallback(
    (w: Omit<Want, "seq">) => {
      const e = all[w.file];
      if (!e) return;
      pinned.current = undefined;
      setFolded((f) => {
        if (!f.has(e.key)) return f;
        const next = new Set(f);
        next.delete(e.key);
        return next;
      });
      setForced((f) => (f.has(e.key) ? f : new Set(f).add(e.key)));
      setWant({ ...w, seq: performance.now() });
    },
    [all],
  );
  const goTo = useCallback((m: DiffMatch) => reveal({ ...m, how: "match" }), [reveal]);
  useLayoutEffect(() => {
    if (!want) return;
    let frames = 0;
    let raf = 0;
    const attempt = () => {
      const b = body.current;
      const row = b && sectionOf(b, want.file)?.querySelector<HTMLElement>(`[data-l="${want.h}:${want.j}"]`);
      if (!b || !row) {
        // Its file is still loading, or showing its lines.
        if (frames++ < 120) raf = requestAnimationFrame(attempt);
        return;
      }
      if (want.how === "match") {
        const range = matchRanges(b, { ...want, start: want.start ?? 0, end: want.end ?? 0 })[0];
        bringIntoView(b, range ?? row);
      } else {
        const v = b.getBoundingClientRect();
        b.scrollTop += row.getBoundingClientRect().top - v.top - FILE_HEADER - CHANGE_MARGIN;
        if (changeAt.current) changeAt.current.top = b.scrollTop;
      }
      pick();
    };
    attempt();
    return () => cancelAnimationFrame(raf);
  }, [want, pick]);

  /** To the next change, or back: from the one the reader stepped to while they stay there, else from the view. */
  const stepChange = useCallback(
    (forward: boolean) => {
      const b = body.current;
      if (!b || !changes.length) return;
      const was = changeAt.current;
      let i: number;
      if (was && Math.abs(b.scrollTop - was.top) < 2) i = was.index + (forward ? 1 : -1);
      else {
        const at = placeAt(CHANGE_MARGIN) ?? { file: Math.max(activeIndex, 0), h: 0, j: 0 };
        i = forward
          ? changes.findIndex((c) => compare(c, at) > 0)
          : changes.findLastIndex((c) => compare(c, at) < 0);
      }
      const c = changes[i];
      if (!c) return;
      reveal({ ...c, how: "change" });
      changeAt.current = { index: i, top: Number.NaN };
    },
    [changes, placeAt, activeIndex, reveal],
  );

  const find = useDiffFind({ lines, body, options, placeAtTop, inView, goTo });

  const onFold = useCallback((key: string) => {
    setFolded((f) => {
      const next = new Set(f);
      if (!next.delete(key)) next.add(key);
      return next;
    });
  }, []);

  /** To the nearest change the agent made out of sight, above or below. */
  const goUnseen = (dir: "above" | "below") => {
    const b = body.current;
    const el = nearest(dir);
    if (!b || !el) return;
    letGo();
    if (el.tagName === "SECTION") {
      const e = all[Number(el.dataset.file)];
      if (e) jumpTo(e.key);
    } else bringIntoView(b, el);
    pick();
    look();
  };

  const showList = () => (wide ? setListShown("shown") : setListOpen(true));
  const onKeyDown = (e: KeyboardEvent) => {
    if (e.nativeEvent.isComposing) return;
    const t = e.target as HTMLElement;
    const typing = t.tagName === "INPUT" || t.tagName === "TEXTAREA" || t.isContentEditable;
    // Elsewhere than on Apple's systems, ⌘ too, for a Mac whose browser says it is another.
    const mod = e.metaKey || (!apple() && e.ctrlKey);
    const key = e.key.toLowerCase();
    if (mod && !e.altKey && !e.shiftKey && key === "f") {
      // Once more in the bar, the browser's own find opens.
      if (find.open && t === find.input.current) return;
      e.preventDefault();
      find.show();
      return;
    }
    if (find.open && ((mod && !e.altKey && key === "g") || e.key === "F3")) {
      e.preventDefault();
      find.step(!e.shiftKey);
      return;
    }
    if (find.open && e.altKey && !e.ctrlKey) {
      const option = TOGGLES.find((x) => x.code === e.code)?.option;
      if (option) {
        e.preventDefault();
        toggle(option);
        return;
      }
    }
    if (e.key === "Escape" && find.open) {
      e.preventDefault();
      find.close();
      return;
    }
    if (e.metaKey || e.ctrlKey || e.altKey || typing) return;
    if (key === "j" || key === "k") go(key === "j" ? 1 : -1);
    else if (key === "n" || key === "p") stepChange(key === "n");
    else if (key === "s") setLayout(layout === "split" ? "unified" : "split");
    else if (key === "w") setWrap(wrap === "wrap" ? "scroll" : "wrap");
    else if (key === "/") {
      showList();
      requestAnimationFrame(() => document.getElementById(`${titleId}-filter`)?.focus());
    } else return;
    e.preventDefault();
  };

  // What finding found in each file, by key, for the list.
  const found = useMemo(() => {
    if (!find.byFile) return undefined;
    return new Map(all.map((e, i) => [e.key, find.byFile?.get(i) ?? 0]));
  }, [find.byFile, all]);

  const list = (
    <FileList
      groups={groups}
      active={all[activeIndex]?.key}
      filterId={`${titleId}-filter`}
      found={found}
      sideBySide={roomy}
      onSelect={(key) => {
        jumpTo(key);
        if (!wide) setListOpen(false);
      }}
    />
  );
  const listVisible = wide ? listShown === "shown" : listOpen;
  const totals = all.reduce(
    (t, e) => ({ a: t.a + (e.file.additions ?? 0), d: t.d + (e.file.deletions ?? 0) }),
    { a: 0, d: 0 },
  );
  // Buttons of the header leave the focus where it is: in the diffs, for the keys that scroll them.
  const keepFocus = (e: React.MouseEvent) => e.preventDefault();

  return createPortal(
    <dialog
      ref={ref}
      aria-labelledby={titleId}
      // Its keys are first in the keyboard shortcuts sheet (? or ⌘/) opened over it.
      data-shortcuts="changes"
      onClose={own(onClose)}
      onKeyDown={onKeyDown}
      onMouseDown={(e) => {
        pressedOutside.current = e.target === ref.current;
      }}
      onClick={(e) => {
        if (e.target === ref.current && pressedOutside.current) ref.current?.close();
      }}
      className={cn(
        "top-(--viewport-shift) -bottom-(--viewport-shift) m-auto overflow-hidden bg-surface p-0 text-ink outline-none",
        "h-dvh max-h-none w-screen max-w-none sm:h-[calc(100dvh-2.5rem)] sm:w-[min(1440px,calc(100vw-2.5rem))] sm:rounded-xl sm:border sm:border-line-strong",
        "shadow-[0_30px_120px_rgb(0_0_0/0.55)] backdrop:bg-black/55 backdrop:backdrop-blur-[3px] open:animate-rise",
      )}
    >
      <div className="flex h-full flex-col">
        <header className="flex shrink-0 items-center gap-2 border-b border-line py-2.5 pr-2.5 pl-2.5 sm:gap-3 sm:pl-3">
          <IconButton
            label={listVisible ? "Hide the changed files" : "Show the changed files"}
            aria-expanded={listVisible}
            onMouseDown={keepFocus}
            onClick={() =>
              wide ? setListShown(listShown === "shown" ? "hidden" : "shown") : setListOpen(!listOpen)
            }
            className={cn(listVisible && "text-ink")}
          >
            <PanelLeft className="size-4" />
          </IconButton>
          <div className="flex min-w-0 flex-1 items-baseline gap-2.5">
            <h2 id={titleId} className="shrink-0 text-[14px] font-medium text-ink">
              Changes
            </h2>
            <span className="min-w-0 truncate text-[12px] text-ink-3 max-sm:hidden">
              {all.length} {all.length === 1 ? "file" : "files"}
              {(totals.a > 0 || totals.d > 0) && (
                <span className="ml-2 font-mono text-[11.5px] tabular-nums">
                  {totals.a > 0 && (
                    <span className="text-added">
                      +<Rolling value={totals.a} />
                    </span>
                  )}
                  {totals.a > 0 && totals.d > 0 && " "}
                  {totals.d > 0 && (
                    <span className="text-danger">
                      −<Rolling value={totals.d} />
                    </span>
                  )}
                </span>
              )}
            </span>
          </div>
          <IconButton
            label={`Find in the diffs (${modKey("F")})`}
            aria-pressed={find.open}
            onClick={() => (find.open ? find.close() : find.show())}
            className={cn(find.open && "bg-hover text-ink")}
          >
            <Search className="size-4" />
          </IconButton>
          <nav aria-label="Changed files" className="flex shrink-0 items-center">
            <IconButton
              label="Previous file (k)"
              onMouseDown={keepFocus}
              onClick={() => go(-1)}
              disabled={activeIndex <= 0}
            >
              <ChevronUp className="size-4" />
            </IconButton>
            <span
              className="min-w-[4.5ch] text-center font-mono text-[11.5px] text-ink-3 tabular-nums"
              aria-live="polite"
            >
              {activeIndex >= 0 ? activeIndex + 1 : "–"}
              <span className="text-ink-4">/{all.length}</span>
            </span>
            <IconButton
              label="Next file (j)"
              onMouseDown={keepFocus}
              onClick={() => go(1)}
              disabled={activeIndex >= all.length - 1}
            >
              <ChevronDown className="size-4" />
            </IconButton>
          </nav>
          <div className="hidden h-5 w-px bg-line sm:block" />
          {roomy && (
            <div className="flex shrink-0 rounded-md border border-line p-0.5">
              {(
                [
                  ["unified", "One below the other (s)", <Rows3 key="u" className="size-3.5" />],
                  ["split", "Side by side (s)", <Columns2 key="s" className="size-3.5" />],
                ] as const
              ).map(([value, label, icon]) => (
                <button
                  key={value}
                  type="button"
                  aria-pressed={layout === value}
                  aria-label={label}
                  title={label}
                  onMouseDown={keepFocus}
                  onClick={() => setLayout(value)}
                  className={cn(
                    "inline-flex h-6.5 w-7.5 items-center justify-center rounded-[5px] transition-colors",
                    layout === value
                      ? "bg-raised text-ink shadow-[0_0_0_1px_var(--c-line-strong)]"
                      : "text-ink-4 hover:text-ink-2",
                  )}
                >
                  {icon}
                </button>
              ))}
            </div>
          )}
          <IconButton
            label={wrap === "wrap" ? "Scroll long lines (w)" : "Wrap long lines (w)"}
            aria-pressed={wrap === "wrap"}
            onMouseDown={keepFocus}
            onClick={() => setWrap(wrap === "wrap" ? "scroll" : "wrap")}
            className={cn(wrap === "wrap" && "bg-hover text-ink")}
          >
            <TextWrap className="size-4" />
          </IconButton>
          <IconButton label="Close (Esc)" onClick={() => ref.current?.close()}>
            <X className="size-4" />
          </IconButton>
        </header>
        <div className="relative flex min-h-0 flex-1">
          {listVisible && (
            <aside
              aria-label="Changed files"
              className={cn(
                "flex min-h-0 flex-col border-line bg-surface",
                wide
                  ? "w-[288px] shrink-0 border-r"
                  : "absolute inset-0 z-10 animate-fade border-b sm:right-auto sm:w-[340px] sm:border-r",
              )}
            >
              {list}
            </aside>
          )}
          <div className="flex min-w-0 flex-1 flex-col">
            {find.open && <DiffFindBar find={find} options={options} onToggle={toggle} loading={waiting} />}
            <div className="relative flex min-h-0 flex-1 flex-col">
              <div
                ref={body}
                // The diffs scroll; the keys move them once they have the focus. They keep the
                // reader's place themselves as the agent changes them (diff-live.tsx), not the browser.
                // biome-ignore lint/a11y/noNoninteractiveTabindex: a scrolling region takes focus to scroll
                tabIndex={0}
                className="min-h-0 flex-1 overflow-auto overscroll-contain bg-canvas/40 outline-none [overflow-anchor:none]"
              >
                <div ref={column} className="relative">
                  {all.map((e, i) => (
                    <FileSection
                      key={e.key}
                      entry={e}
                      index={i}
                      session={session}
                      cliRef={cliRef}
                      layout={shownLayout}
                      wrap={wrap === "wrap"}
                      lines={lines[i]}
                      folded={folded.has(e.key)}
                      forced={forced.has(e.key)}
                      waiting={waiting}
                      several={several}
                      entering={!atOpen.has(e.key)}
                      live={live}
                      watch={watch}
                      onFold={onFold}
                      onDiff={onDiff}
                    />
                  ))}
                </div>
                {all.length === 0 ? (
                  <Note icon={<FileQuestion />} title="No changes any more">
                    The agent's work no longer changes any file.
                  </Note>
                ) : (
                  <p className="px-4 pt-8 pb-12 text-center text-[12px] text-ink-4">End of the changes</p>
                )}
              </div>
              <UnseenChanges dir="above" count={unseen.above} onGo={() => goUnseen("above")} />
              <UnseenChanges dir="below" count={unseen.below} onGo={() => goUnseen("below")} />
            </div>
          </div>
        </div>
      </div>
    </dialog>,
    document.body,
  );
}

/**
 * Brings a match, or a line, into view: across its diff's long lines, then
 * up or down the diffs to the middle of the view, unless it shows already
 * (not under its file's header).
 * Far, the diffs jump there, as a browser's find does; near, they glide.
 */
function bringIntoView(b: HTMLElement, to: Range | Element) {
  const start = to instanceof Range ? to.startContainer : to;
  const el = start.nodeType === 1 ? (start as Element) : start.parentElement;
  // Across, in a diff whose long lines scroll: clear of the line numbers that stay put.
  const across = el?.closest<HTMLElement>(".diff-scroll");
  if (across && across.scrollWidth > across.clientWidth) {
    const r = to.getBoundingClientRect();
    const box = across.getBoundingClientRect();
    const gutter =
      (el?.closest(".diff-line")?.querySelector(".diff-gutter")?.getBoundingClientRect().width ?? 0) + 20;
    const left = box.left + gutter;
    if (r.left < left || r.right > box.right)
      across.scrollLeft += r.left - left - Math.max(0, (box.right - left - r.width) / 3);
  }
  const r = to.getBoundingClientRect();
  const v = b.getBoundingClientRect();
  const top = v.top + FILE_HEADER;
  // Clear of its file's header and the bottom edge, it stays where it is.
  const margin = LINE / 2;
  if (r.top >= top + margin && r.bottom <= v.bottom - margin) return;
  const by = r.top + r.height / 2 - (top + (v.bottom - top) / 2);
  const reduce = window.matchMedia?.("(prefers-reduced-motion: reduce)").matches;
  b.scrollTo({
    top: b.scrollTop + by,
    behavior: reduce || Math.abs(by) > (v.bottom - top) * 1.5 ? "instant" : "smooth",
  });
}

/** The room a file takes before it shows its lines (px): as many lines as it has, or as it changed. */
function roomFor(file: G.FileChange, diff: G.FileDiff | undefined, lines: PatchLine[] | undefined): number {
  if (lines) {
    const hunks = new Set(lines.map((l) => l.h)).size;
    return lines.length * LINE + (hunks > 1 ? hunks * HUNK : 0) + 8;
  }
  if (diff || !file.digest) return 220;
  return Math.min(4000, ((file.additions ?? 0) + (file.deletions ?? 0) + 6) * LINE);
}

/**
 * A changed file: its header, which stays at the top while its lines scroll
 * under it, and its diff, which shows once it comes near the view.
 */
const FileSection = memo(function FileSection({
  entry,
  index,
  session,
  cliRef,
  layout,
  wrap,
  lines,
  folded,
  forced,
  waiting,
  several,
  entering,
  live,
  watch,
  onFold,
  onDiff,
}: {
  entry: Entry;
  index: number;
  session: string;
  cliRef: string;
  layout: DiffLayout;
  wrap: boolean;
  /** Its diff's lines, once the dialog has them, for the room it takes before it shows them. */
  lines?: PatchLine[];
  folded: boolean;
  /** Its lines show, wherever it is: the reader goes there. */
  forced: boolean;
  /** All the diffs are coming at once: not one by one yet. */
  waiting: boolean;
  several: boolean;
  /** The agent changed it first while the dialog shows: its lines come in as it shows them. */
  entering: boolean;
  live: LiveView;
  watch: (el: Element, near: () => void) => () => void;
  onFold: (key: string) => void;
  onDiff: (key: string, d: G.FileDiff | undefined) => void;
}) {
  const q = useFileDiff(session, entry, waiting);
  const diff = q.data;
  const { key } = entry;
  useEffect(() => onDiff(key, diff), [key, diff, onDiff]);
  useEffect(() => () => onDiff(key, undefined), [key, onDiff]);
  const el = useRef<HTMLElement>(null);
  // A file the agent changed first while the dialog shows draws its lines at once, as they come.
  const [near, setNear] = useState(forced || entering);
  if (forced && !near) setNear(true);
  useEffect(() => {
    if (near || !el.current) return;
    return watch(el.current, () => setNear(true));
  }, [near, watch]);
  // Its lines come in once, as it first shows them; drawn again (folded and opened), they are there.
  const [comes, setComes] = useState(entering);
  useEffect(() => {
    if (diff && near && !folded) setComes(false);
  }, [diff, near, folded]);
  // Changed where its lines don't show yet: the reader can go there. Where they show, they say
  // themselves what changed.
  const digest = entry.file.digest;
  const seen = useRef(entering ? undefined : digest);
  useEffect(() => {
    if (digest === seen.current) return;
    seen.current = digest;
    const b = live.band();
    const r = el.current?.getBoundingClientRect();
    if (near || folded || !r || (b && r.bottom > b.top && r.top < b.bottom)) return;
    setComes(false);
    if (el.current) live.unseen([[el.current]]);
  }, [digest, near, folded, live]);
  const room = roomFor(entry.file, diff, lines);
  const headingId = useId();
  return (
    <section
      ref={el}
      data-file={index}
      aria-labelledby={headingId}
      className={cn("border-b border-line", entering && "animate-fade")}
    >
      <FileHeader
        entry={entry}
        diff={diff}
        headingId={headingId}
        folded={folded}
        several={several}
        isNew={entering}
        onFold={() => onFold(key)}
      />
      {!folded &&
        (near ? (
          <FileBody
            entry={entry}
            q={q}
            cliRef={cliRef}
            layout={layout}
            wrap={wrap}
            live={live}
            entering={comes}
          />
        ) : (
          <div style={{ height: room }} aria-hidden="true" />
        ))}
    </section>
  );
});

/** A file's name, how it changed and by how much, what folds it, and where it is on GitHub. */
function FileHeader({
  entry,
  diff,
  headingId,
  folded,
  several,
  isNew,
  onFold,
}: {
  entry: Entry;
  diff?: G.FileDiff;
  headingId: string;
  folded: boolean;
  several: boolean;
  /** The agent changed it first while the dialog shows. */
  isNew: boolean;
  onFold: () => void;
}) {
  const { file } = entry;
  const { dir, name } = splitPath(file.path);
  const change = changeOf(file.status);
  const a = file.additions ?? 0;
  const d = file.deletions ?? 0;
  const { blob } = entry;
  const changed = useChanged(diff?.digest);
  return (
    <div
      className="sticky top-0 z-[3] flex items-center gap-2 border-b border-line bg-surface pr-2 pl-1.5"
      style={{ height: FILE_HEADER }}
    >
      {/* A light runs along its edge as the agent changes the file. */}
      {changed.times > 0 && (
        <span
          className="pointer-events-none absolute inset-x-0 -bottom-px h-px overflow-hidden"
          aria-hidden="true"
        >
          <span key={changed.times} className="diff-header-scan" />
        </span>
      )}
      <button
        type="button"
        onClick={onFold}
        aria-expanded={!folded}
        aria-label={folded ? `Show the lines of ${file.path}` : `Hide the lines of ${file.path}`}
        title={folded ? "Show its lines" : "Hide its lines"}
        className="inline-flex size-6 shrink-0 items-center justify-center rounded-md text-ink-4 transition-colors hover:bg-hover hover:text-ink"
      >
        <ChevronRight className={cn("size-3.5 transition-transform duration-150", !folded && "rotate-90")} />
      </button>
      <span className={cn("w-3 shrink-0 text-center font-mono text-[11px]", change.tone)} aria-hidden="true">
        {change.letter}
      </span>
      <h3 id={headingId} className="flex min-w-0 items-baseline font-mono text-[12.5px]" title={file.path}>
        <span className="sr-only">{change.label}: </span>
        {several && entry.repo && <span className="mr-2 shrink-0 text-ink-4">{entry.repo}</span>}
        {dir && <span className="min-w-0 shrink-[1000] truncate text-ink-4">{dir}</span>}
        <span className="min-w-0 truncate font-medium text-ink">{name}</span>
      </h3>
      <CopyPath path={file.path} />
      {diff?.old_path && (
        <span
          className="hidden min-w-0 truncate font-mono text-[11.5px] text-ink-4 md:inline"
          title={diff.old_path}
        >
          from {diff.old_path}
        </span>
      )}
      <span className="flex-1" />
      <Updated fresh={changed.fresh} isNew={isNew} />
      {(a > 0 || d > 0) && (
        <span className="shrink-0 font-mono text-[11.5px] tabular-nums">
          {a > 0 && (
            <span className="text-added">
              +<Rolling value={a} />
            </span>
          )}
          {a > 0 && d > 0 && " "}
          {d > 0 && (
            <span className="text-danger">
              −<Rolling value={d} />
            </span>
          )}
        </span>
      )}
      {blob && (
        <a
          href={blob}
          target="_blank"
          rel="noreferrer"
          title="The file on GitHub, on the session's branch"
          className="hidden h-7 shrink-0 items-center gap-1 rounded-md px-1.5 text-[12px] text-ink-3 transition-colors hover:bg-hover hover:text-ink md:inline-flex"
        >
          GitHub <ArrowUpRight className="size-3.5" />
        </a>
      )}
    </div>
  );
}

/** Copies a file's path. */
function CopyPath({ path }: { path: string }) {
  const [done, setDone] = useState(false);
  const t = useRef<ReturnType<typeof setTimeout>>(undefined);
  useEffect(() => () => clearTimeout(t.current), []);
  const label = done ? "Copied" : "Copy the path";
  return (
    <button
      type="button"
      aria-label={label}
      title={label}
      onClick={async () => {
        await navigator.clipboard.writeText(path);
        setDone(true);
        clearTimeout(t.current);
        t.current = setTimeout(() => setDone(false), 1600);
      }}
      className="inline-flex size-6 shrink-0 items-center justify-center rounded-md text-ink-4 transition-colors hover:bg-hover hover:text-ink"
    >
      {done ? <Check className="size-3.5 text-ink" /> : <Copy className="size-3.5" />}
    </button>
  );
}

/**
 * Says, for a moment, that a file's diff changed while it shows (the agent
 * is at work on it), or that the agent changed it first while the dialog
 * shows.
 */
function Updated({ fresh, isNew }: { fresh: boolean; isNew: boolean }) {
  const [arrived, setArrived] = useState(isNew);
  useEffect(() => {
    if (!arrived) return;
    const t = setTimeout(() => setArrived(false), 2600);
    return () => clearTimeout(t);
  }, [arrived]);
  if (!fresh && !arrived) return null;
  return (
    <span className="inline-flex shrink-0 animate-fade items-center gap-1 rounded-full bg-accent-soft px-1.5 py-px text-[11px] text-accent">
      <span className="size-1.5 animate-pulse-soft rounded-full bg-accent" />
      {fresh ? "Updated" : "New"}
    </span>
  );
}

/** A file's diff, or why there is none to show. */
function FileBody({
  entry,
  q,
  cliRef,
  layout,
  wrap,
  live,
  entering,
}: {
  entry: Entry;
  q: ReturnType<typeof useFileDiff>;
  cliRef: string;
  layout: DiffLayout;
  wrap: boolean;
  live: LiveView;
  entering: boolean;
}) {
  const diff = q.data;
  const path = entry.file.path;
  const hunks = useMemo(() => (diff?.patch ? parsePatch(diff.patch) : []), [diff?.patch]);
  const lang = useMemo(() => languageOf(path, firstLine(hunks)).id, [path, hunks]);
  if (!entry.file.digest) {
    return (
      <Note icon={<History />} title="No diff of this file">
        Diffs come from the session's agent, and this one sent none: its runner runs an older version of fctl.
      </Note>
    );
  }
  if (!diff) {
    if (q.isError) {
      const missing = q.error instanceof ApiFailure && q.error.status === 404;
      return (
        <Note
          icon={<FileX2 />}
          title={missing ? "The session doesn't have this diff yet" : "The diff didn't load"}
          action={
            <Button size="sm" onClick={() => q.refetch()} loading={q.isFetching}>
              Try again
            </Button>
          }
        >
          {missing ? "The agent sends it with its next look at the repository." : q.error.message}
        </Note>
      );
    }
    // A file whose lines come in shows them as they come, from nothing.
    if (entering) return null;
    return <Loading lines={(entry.file.additions ?? 0) + (entry.file.deletions ?? 0)} />;
  }
  const command = diffCommand(cliRef, path, entry.repo);
  // What changed beside its lines: where a renamed file was, and its mode.
  const mode = diff.old_mode && diff.new_mode ? modeChange(diff.old_mode, diff.new_mode) : undefined;
  const from = diff.old_path && <code className="font-mono text-ink-2">{diff.old_path}</code>;
  let content: ReactNode;
  // What the lines don't say goes above them; with no lines, the note says it.
  let above = true;
  // A file that changed type from or to a binary one has the other half's lines.
  if (diff.binary && !hunks.length) {
    content = (
      <Note icon={<Binary />} title="A binary file">
        git shows no lines of it.
      </Note>
    );
  } else if (diff.too_large) {
    content = (
      <Note icon={<FileX2 />} title="This diff is too large to show here">
        <p>
          {entry.file.additions ?? "?"} lines added and {entry.file.deletions ?? "?"} taken out: more than the
          session keeps. The CLI prints it:
        </p>
        <CodeBox value={command} className="mt-3 text-left" />
      </Note>
    );
  } else if (!hunks.length) {
    above = false;
    content = from ? (
      <Note icon={<FilePen />} title="Renamed, its lines as they were">
        From {from}
        {mode && `, its mode ${mode}`}.
      </Note>
    ) : mode ? (
      <Note icon={<FilePen />} title="Only its mode changed">
        {mode}.
      </Note>
    ) : (
      <Note icon={<FileQuestion />} title="An empty file">
        It has no lines.
      </Note>
    );
  } else {
    content = (
      <DiffLines
        hunks={hunks}
        lang={lang}
        layout={layout}
        wrap={wrap}
        live={live}
        entering={entering}
        className="diff-on-surface pb-2"
      />
    );
  }
  return (
    <div className="flex flex-col">
      {above && (from || mode) && (
        <div className="flex flex-wrap gap-x-4 gap-y-1 border-b border-line bg-surface px-4 py-2 text-[12px] text-ink-3">
          {from && <span>Renamed from {from}</span>}
          {mode && <span>Mode {mode}</span>}
        </div>
      )}
      <div className="flex animate-fade flex-col">{content}</div>
    </div>
  );
}

/** What a change of a file's mode means: `100644 → 100755: executable now`. */
export function modeChange(before: string, after: string): string {
  const link = (m: string) => m === "120000";
  const exec = (m: string) => m.endsWith("755");
  const meaning =
    link(after) && !link(before)
      ? "a symbolic link now"
      : link(before) && !link(after)
        ? "no longer a symbolic link"
        : exec(after) && !exec(before)
          ? "executable now"
          : exec(before) && !exec(after)
            ? "no longer executable"
            : "";
  return `${before} → ${after}${meaning && `: ${meaning}`}`;
}

/** Lines that shimmer while a diff loads, as many as the file's change, up to a screenful. */
function Loading({ lines }: { lines: number }) {
  const widths = ["w-2/5", "w-3/5", "w-1/3", "w-4/5", "w-1/2", "w-2/3", "w-1/4", "w-3/4", "w-2/5", "w-1/2"];
  const n = Math.max(3, Math.min(widths.length, lines + 2));
  return (
    <div className="flex flex-col gap-2.5 px-5 py-4" aria-busy="true">
      <span className="sr-only" role="status">
        Loading the diff
      </span>
      {widths.slice(0, n).map((w, i) => (
        <div key={i} className="flex items-center gap-4" style={{ opacity: 1 - i * 0.07 }}>
          <Skeleton className="h-3 w-8 shrink-0" />
          <Skeleton className={cn("h-3", w)} />
        </div>
      ))}
    </div>
  );
}

/** Why a file shows no lines: a binary file, one too large, one an older agent sent no diff of. */
function Note({
  icon,
  title,
  children,
  action,
}: {
  icon: ReactNode;
  title: string;
  children?: ReactNode;
  action?: ReactNode;
}) {
  return (
    <div className="flex animate-fade flex-col items-center justify-center px-6 py-8 text-center">
      <div className="mb-3 flex size-9 items-center justify-center rounded-full border border-line bg-sunken text-ink-3 [&_svg]:size-4">
        {icon}
      </div>
      <h4 className="text-[14px] font-medium text-ink">{title}</h4>
      {children && <div className="mt-1 max-w-md text-[13px] leading-relaxed text-ink-3">{children}</div>}
      {action && <div className="mt-4">{action}</div>}
    </div>
  );
}

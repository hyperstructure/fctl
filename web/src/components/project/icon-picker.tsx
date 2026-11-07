// Choosing a project's icon: an IconMCP glyph, its color and its style, shown as the sidebar and
// the project's page will show it while you choose. Nothing changes until you save. The keyboard
// does it all: type to search, the arrows move through the icons, Enter picks one, ⌘Enter saves.

import { Search, X } from "lucide-react";
import { type KeyboardEvent, useEffect, useMemo, useRef, useState } from "react";
import { Button, Dialog, Kbd } from "@/components/ui";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { apple } from "@/lib/hotkeys";
import {
  COLOR_ORDER,
  type Fits,
  ICON_COLORS,
  ICON_GROUPS,
  ICON_STYLES,
  ICONS,
  type IconChoice,
  iconDef,
  iconOf,
  sameIcon,
  searchIcons,
  suggestIcons,
} from "@/lib/project-icons";
import { Glyph, ProjectIcon } from "./project-icon";

/** The project whose icon is chosen: what suggests one, and what the preview shows. */
export type PickerProject = Fits & {
  icon?: G.ProjectIcon;
  active_sessions?: number;
  workspace?: Pick<G.WorkspaceRef, "name">;
};

export function IconPicker({
  open,
  onClose,
  project,
  onSave,
  saving,
}: {
  open: boolean;
  onClose: () => void;
  project: PickerProject;
  /** Keeps the choice (none: the icon that fits again). */
  onSave: (icon: G.ProjectIcon | null) => void;
  saving?: boolean;
}) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title={project.name ? `Icon for ${project.name}` : "Project icon"}
      description={
        project.workspace
          ? `It shows in the sidebar, on the project's page and in search, for everyone in ${project.workspace.name}.`
          : "It shows in the sidebar, on the project's page and in search, for everyone in its workspace."
      }
    >
      {open && <Picker project={project} onSave={onSave} onCancel={onClose} saving={saving} />}
    </Dialog>
  );
}

interface Section {
  title: string;
  names: string[];
}

function Picker({
  project,
  onSave,
  onCancel,
  saving,
}: {
  project: PickerProject;
  onSave: (icon: G.ProjectIcon | null) => void;
  onCancel: () => void;
  saving?: boolean;
}) {
  const current = iconOf(project);
  const [draft, setDraft] = useState<IconChoice>({
    name: current.name,
    color: current.color,
    style: current.style,
  });
  // Back to the icon that fits: saving then takes the project's own away.
  const [fitting, setFitting] = useState(!current.chosen);
  const [query, setQuery] = useState("");
  const [focus, setFocus] = useState(0);
  const [pointed, setPointed] = useState<string>();
  const search = useRef<HTMLInputElement>(null);
  const grid = useRef<HTMLDivElement>(null);
  const suggested = useMemo(() => suggestIcons(project, 12), [project]);
  const sections: Section[] = useMemo(() => {
    if (query.trim()) {
      const found = searchIcons(query).map((i) => i.name);
      return [{ title: found.length === 1 ? "1 icon" : `${found.length} icons`, names: found }];
    }
    return [
      { title: project.name ? `Fits ${project.name}` : "Suggested", names: suggested },
      ...ICON_GROUPS.map((g) => ({ title: g, names: ICONS.filter((i) => i.group === g).map((i) => i.name) })),
    ];
  }, [query, suggested, project.name]);
  const flat = useMemo(() => sections.flatMap((s) => s.names), [sections]);
  // The focus goes back to the first icon as the list changes.
  const [focusIn, setFocusIn] = useState(sections);
  if (focusIn !== sections) {
    setFocusIn(sections);
    setFocus(0);
  }
  const changed = fitting ? current.chosen : !current.chosen || !sameIcon(draft, current);
  // Once until the save settles: two keys in one tick would otherwise both save.
  const sent = useRef(false);
  useEffect(() => {
    if (!saving) sent.current = false;
  }, [saving]);
  const save = () => {
    if (saving || sent.current) return;
    if (!changed) return onCancel();
    sent.current = true;
    onSave(fitting ? null : draft);
  };
  const pick = (name: string) => {
    setDraft((d) => ({ ...d, name }));
    setFitting(false);
  };
  const shown = fitting ? iconOf({ ...project, icon: undefined }) : draft;
  const label = (name?: string) => (name ? iconDef(name)?.label : undefined);
  const about = pointed ?? flat[focus];

  // The grid's columns, for the arrows that go up and down.
  const columns = () => {
    const first = grid.current?.querySelector<HTMLElement>("[data-i]");
    const box = first?.parentElement;
    if (!first || !box) return 1;
    const gap = Number.parseFloat(getComputedStyle(box).columnGap) || 0;
    return Math.max(1, Math.round((box.clientWidth + gap) / (first.offsetWidth + gap)));
  };
  const moveTo = (i: number) => {
    const next = Math.max(0, Math.min(flat.length - 1, i));
    setFocus(next);
    grid.current?.querySelector<HTMLElement>(`[data-i="${next}"]`)?.focus();
  };
  /** Where up and down go: a row up or down in the section, or into the next one at the same column. */
  const vertical = (i: number, down: boolean) => {
    const cols = columns();
    let start = 0;
    for (const [k, s] of sections.entries()) {
      const end = start + s.names.length;
      if (i < end) {
        const col = (i - start) % cols;
        const target = i + (down ? cols : -cols);
        if (target >= start && target < end) return target;
        if (down && target >= end) {
          // The last row of a section: the next one's first row, or this one's last icon.
          const nextStart = end;
          const next = sections[k + 1];
          if (!next) return Math.min(target, end - 1);
          if (Math.floor((i - start) / cols) < Math.floor((end - 1 - start) / cols)) return end - 1;
          return nextStart + Math.min(col, next.names.length - 1);
        }
        if (!down && target < start) {
          const prev = sections[k - 1];
          if (!prev) return -1;
          const prevStart = start - prev.names.length;
          const lastRow = Math.floor((prev.names.length - 1) / cols) * cols;
          return prevStart + Math.min(lastRow + col, prev.names.length - 1);
        }
        return i;
      }
      start = end;
    }
    return i;
  };
  const onGridKey = (e: KeyboardEvent) => {
    const i = focus;
    if (e.key === "ArrowRight") moveTo(i + 1);
    else if (e.key === "ArrowLeft") moveTo(i - 1);
    else if (e.key === "ArrowDown") moveTo(vertical(i, true));
    else if (e.key === "ArrowUp") {
      const up = vertical(i, false);
      if (up < 0) search.current?.focus();
      else moveTo(up);
    } else if (e.key === "Home") moveTo(0);
    else if (e.key === "End") moveTo(flat.length - 1);
    // ⌘Enter goes on to the picker, which saves.
    else if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) return;
    else if (e.key === "Enter" || e.key === " ") {
      const name = flat[i];
      if (name) pick(name);
    } else if (e.key.length === 1 && !e.metaKey && !e.ctrlKey && !e.altKey) {
      // Typing searches, wherever the focus is.
      search.current?.focus();
      setQuery((q) => q + e.key);
    } else return;
    e.preventDefault();
  };

  return (
    // biome-ignore lint/a11y/noStaticElementInteractions: ⌘Enter saves from anywhere in the picker.
    <div
      className="flex flex-col gap-5"
      onKeyDown={(e) => {
        if (e.key === "Enter" && (e.metaKey || e.ctrlKey)) {
          e.preventDefault();
          save();
        }
      }}
    >
      <div className="grid gap-4 sm:grid-cols-[minmax(0,1fr)_auto]">
        {/* As the sidebar and the project's page will show it. */}
        <div className="flex min-w-0 items-center gap-4 rounded-xl border border-line bg-sunken/70 px-4 py-3.5">
          <ProjectIcon icon={shown} tile={52} className="transition-[background-color,color] duration-200" />
          <div className="min-w-0 flex-1">
            <div className="flex h-8 items-center gap-2.5 rounded-md bg-raised px-2.5 text-[13.5px] text-ink shadow-[inset_0_0_0_1px_var(--c-line)]">
              <ProjectIcon icon={shown} size={16} />
              <span className="min-w-0 flex-1 truncate">{project.name || "New project"}</span>
              {!!project.active_sessions && (
                <span className="text-[11.5px] text-accent tabular-nums">{project.active_sessions}</span>
              )}
            </div>
            <p className="mt-1.5 truncate text-[12px] text-ink-4">
              {fitting ? "The icon that fits its name" : (label(shown.name) ?? shown.name)}
            </p>
          </div>
        </div>
        <div className="flex flex-col justify-center gap-3">
          <div className="flex items-center gap-3">
            <span className="w-10 text-[12.5px] text-ink-3">Color</span>
            <div className="flex gap-1" role="radiogroup" aria-label="Color">
              {COLOR_ORDER.map((c) => (
                // biome-ignore lint/a11y/useSemanticElements: a swatch, as a radio of the group.
                <button
                  key={c}
                  type="button"
                  role="radio"
                  aria-checked={!fitting && draft.color === c}
                  aria-label={ICON_COLORS[c].label}
                  title={ICON_COLORS[c].label}
                  onClick={() => {
                    setDraft((d) => ({ ...d, name: fitting ? shown.name : d.name, color: c }));
                    setFitting(false);
                  }}
                  className={cn(
                    "flex size-8 items-center justify-center rounded-lg transition-[box-shadow,background-color] duration-150",
                    !fitting && draft.color === c
                      ? "bg-raised shadow-[inset_0_0_0_1.5px_var(--c-ink-3)]"
                      : "hover:bg-hover",
                  )}
                >
                  <span
                    className="size-4 rounded-full shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                    style={{ background: ICON_COLORS[c].ink }}
                  />
                </button>
              ))}
            </div>
          </div>
          <div className="flex items-center gap-3">
            <span className="w-10 text-[12.5px] text-ink-3">Style</span>
            <div
              className="flex rounded-lg border border-line bg-sunken/60 p-0.5"
              role="radiogroup"
              aria-label="Style"
            >
              {ICON_STYLES.map(({ style, label: name }) => (
                // biome-ignore lint/a11y/useSemanticElements: a segment, as a radio of the group.
                <button
                  key={style}
                  type="button"
                  role="radio"
                  aria-checked={!fitting && draft.style === style}
                  title={name}
                  onClick={() => {
                    setDraft((d) => ({ ...d, name: fitting ? shown.name : d.name, style }));
                    setFitting(false);
                  }}
                  className={cn(
                    "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12.5px] transition-colors duration-150",
                    !fitting && draft.style === style
                      ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                      : "text-ink-3 hover:text-ink",
                  )}
                >
                  <Glyph name={shown.name} style={style} color={shown.color} size={14} />
                  {name}
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>

      <div>
        <label className="flex h-9.5 items-center gap-2.5 rounded-lg border border-line-strong bg-sunken/60 px-3 text-[13.5px] transition-colors focus-within:border-ink-4">
          <Search className="size-4 shrink-0 text-ink-4" />
          <input
            ref={search}
            autoFocus
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            onKeyDown={(e) => {
              if (e.key === "ArrowDown") {
                e.preventDefault();
                moveTo(0);
              } else if (e.key === "Enter" && !e.metaKey && !e.ctrlKey) {
                e.preventDefault();
                const [first] = flat;
                if (query.trim() && first) pick(first);
              } else if (e.key === "Escape" && query) {
                // A search clears before the picker closes.
                e.preventDefault();
                setQuery("");
              }
            }}
            placeholder="Search icons: server, rocket, leaf…"
            aria-label="Search icons"
            className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-ink-4"
          />
          {query && (
            <button
              type="button"
              aria-label="Clear the search"
              onClick={() => {
                setQuery("");
                search.current?.focus();
              }}
              className="-mr-1 rounded p-1 text-ink-4 hover:text-ink"
            >
              <X className="size-3.5" />
            </button>
          )}
        </label>
      </div>

      <div
        ref={grid}
        role="listbox"
        aria-label="Icons"
        tabIndex={-1}
        onKeyDown={onGridKey}
        onPointerLeave={() => setPointed(undefined)}
        className="-mx-2 -mt-2 h-[min(44vh,340px)] overflow-y-auto overscroll-contain px-2 [scrollbar-gutter:stable]"
      >
        {flat.length === 0 ? (
          <div className="flex h-full flex-col items-center justify-center gap-1.5 text-center">
            <p className="text-[13.5px] text-ink-2">No icon here is called “{query.trim()}”.</p>
            <p className="max-w-xs text-[12.5px] text-ink-4">
              Try what it shows rather than what it means: a leaf rather than nature.
            </p>
          </div>
        ) : (
          (() => {
            let at = 0;
            return sections.map((sec) => {
              const start = at;
              at += sec.names.length;
              return (
                <section key={sec.title} className="pb-3">
                  <h3 className="sticky top-0 z-[1] -mx-0.5 bg-surface/95 px-0.5 pt-2 pb-1.5 text-[12px] font-medium text-ink-4 backdrop-blur-sm">
                    {sec.title}
                  </h3>
                  <div className="grid grid-cols-[repeat(auto-fill,minmax(40px,1fr))] gap-1 pointer-coarse:grid-cols-[repeat(auto-fill,minmax(46px,1fr))]">
                    {sec.names.map((name, k) => {
                      const i = start + k;
                      const on = !fitting && name === draft.name;
                      const def = iconDef(name);
                      return (
                        <button
                          key={`${sec.title}-${name}`}
                          type="button"
                          role="option"
                          aria-selected={on}
                          aria-label={def?.label ?? name}
                          data-i={i}
                          tabIndex={i === focus ? 0 : -1}
                          onFocus={() => setFocus(i)}
                          onPointerEnter={() => setPointed(name)}
                          onClick={() => pick(name)}
                          onDoubleClick={() => {
                            pick(name);
                            if (saving || sent.current) return;
                            sent.current = true;
                            onSave({ ...draft, name });
                          }}
                          className={cn(
                            "group/i relative flex aspect-square items-center justify-center rounded-lg outline-offset-0 transition-[background-color,box-shadow,color,transform] duration-150 active:scale-90",
                            on
                              ? "bg-raised shadow-[inset_0_0_0_1.5px_var(--c-accent)]"
                              : "text-ink-3 hover:bg-hover hover:text-ink focus-visible:text-ink",
                          )}
                          style={on ? { color: ICON_COLORS[draft.color].ink } : undefined}
                        >
                          <Glyph name={name} style={draft.style} size={20} />
                        </button>
                      );
                    })}
                  </div>
                </section>
              );
            });
          })()
        )}
      </div>

      {/* What the icon under the pointer, or in focus, is. */}
      <p className="-mt-3 flex h-4 items-center gap-2 truncate text-[12px] text-ink-4" aria-live="polite">
        {about && (
          <>
            <span className="text-ink-2">{label(about)}</span>
            <span className="truncate">{iconDef(about)?.tags.slice(0, 4).join(", ")}</span>
          </>
        )}
      </p>

      <div className="-mx-6 -mb-6 flex flex-wrap items-center gap-2 border-t border-line px-6 py-4">
        <p className="min-w-0 flex-1 text-[12px] text-ink-4">
          Icons from{" "}
          <a
            href="https://iconmcp.com"
            target="_blank"
            rel="noreferrer"
            className="underline decoration-line-strong underline-offset-2 hover:text-ink-2"
          >
            IconMCP
          </a>
          , MIT licence.
        </p>
        {!fitting && (
          <Button variant="ghost" onClick={() => setFitting(true)}>
            Use the one that fits
          </Button>
        )}
        <Button variant="ghost" onClick={onCancel}>
          Cancel
        </Button>
        <Button
          variant="primary"
          onClick={save}
          loading={saving}
          disabled={!changed}
          trailing={<Kbd>{apple() ? "⌘" : "Ctrl"}↵</Kbd>}
        >
          Save
        </Button>
      </div>
    </div>
  );
}

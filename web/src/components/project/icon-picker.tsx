// Choosing a project's icon: an IconMCP glyph, its color and its style, shown as the sidebar and
// the project's page will show it while you choose. Nothing changes until you save. The keyboard
// does it all: type to search, the arrows move through the icons, Enter picks one, ⌘Enter saves.
// It fits the screen: on a phone the icons' list gives up its height, so that Save stays in view.
// Its colors are the workspace's four and one of its own, which ui/color-popover.tsx chooses.

import { Search, X } from "lucide-react";
import { type KeyboardEvent, useCallback, useEffect, useId, useMemo, useRef, useState } from "react";
import { Button, Dialog, Kbd, toast } from "@/components/ui";
import { ColorPopover } from "@/components/ui/color-popover";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { apple } from "@/lib/hotkeys";
import {
  BRAND_HEX,
  COLOR_ORDER,
  FIRST_OWN,
  type Fits,
  ICON_COLORS,
  ICON_GROUPS,
  ICON_STYLES,
  ICONS,
  type IconChoice,
  iconDef,
  iconOf,
  iconPaint,
  ownColors,
  sameIcon,
  searchIcons,
  suggestIcons,
  toSave,
  useSaveIcon,
} from "@/lib/project-icons";
import { keys, queryClient } from "@/lib/queries";
import { touchDevice } from "@/lib/two-factor";
import { IconThemes, OwnSwatch } from "./icon-color";
import { Glyph, ProjectIcon } from "./project-icon";

/** The project whose icon is chosen: what suggests one, and what the preview shows. */
export type PickerProject = Fits & {
  icon?: G.ProjectIcon;
  active_sessions?: number;
  /** Its `id` finds the colors of their own its other projects have. */
  workspace?: Pick<G.WorkspaceRef, "name"> & { id?: string };
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
      fit
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
    ...(current.hex && { hex: current.hex }),
  });
  // The color of its own, kept while a workspace color is tried, to go back to.
  const [own, setOwn] = useState(current.chosen ? current.hex : undefined);
  const [editing, setEditing] = useState<false | "pointer" | "keyboard">(false);
  const ownSwatch = useRef<HTMLButtonElement>(null);
  const popoverId = useId();
  // The colors of their own the workspace's other projects have, as the sidebar's list knows them.
  const used = useMemo(() => {
    const ws = project.workspace?.id;
    const all = queryClient.getQueryData<G.ProjectList>(keys.projects)?.projects ?? [];
    return ws ? ownColors(all.filter((p) => p.workspace?.id === ws && p.id !== project.id)).slice(0, 12) : [];
  }, [project.workspace?.id, project.id]);
  // Back to the icon that fits: saving then takes the project's own away.
  const [fitting, setFitting] = useState(!current.chosen);
  const [query, setQuery] = useState("");
  const [focus, setFocus] = useState(0);
  const [pointed, setPointed] = useState<string>();
  const search = useRef<HTMLInputElement>(null);
  // On a touch screen, a focused search would bring up the keyboard over the icons.
  const [touch] = useState(touchDevice);
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
    onSave(fitting ? null : toSave(draft));
  };
  const pick = (name: string) => {
    setDraft((d) => ({ ...d, name }));
    setFitting(false);
  };
  const shown = fitting ? iconOf({ ...project, icon: undefined }) : draft;
  const paint = iconPaint(draft);
  const ownPaint = own ? iconPaint({ color: "ink", hex: own }) : undefined;
  const closeEditor = useCallback((refocus: boolean) => {
    setEditing(false);
    if (refocus) ownSwatch.current?.focus();
  }, []);
  const chooseOwn = (hex: string) => {
    setOwn(hex);
    setDraft((d) => ({ ...d, name: fitting ? shown.name : d.name, hex }));
    setFitting(false);
  };
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
      className="flex min-h-0 flex-auto flex-col gap-5 max-sm:gap-4"
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
        {/* On a phone, color and style share a row, without their names. */}
        <div className="flex flex-col justify-center gap-3 max-sm:flex-row max-sm:items-center max-sm:justify-between">
          <div className="flex items-center gap-3">
            <span className="w-10 text-[12.5px] text-ink-3 max-sm:hidden">Color</span>
            <div className="flex gap-1 max-sm:gap-0.5" role="radiogroup" aria-label="Color">
              {COLOR_ORDER.map((c) => (
                // biome-ignore lint/a11y/useSemanticElements: a swatch, as a radio of the group.
                <button
                  key={c}
                  type="button"
                  role="radio"
                  aria-checked={!fitting && !draft.hex && draft.color === c}
                  aria-label={ICON_COLORS[c].label}
                  title={ICON_COLORS[c].label}
                  onClick={() => {
                    setDraft((d) => ({
                      ...d,
                      name: fitting ? shown.name : d.name,
                      color: c,
                      hex: undefined,
                    }));
                    setFitting(false);
                  }}
                  className={cn(
                    "flex size-8 items-center justify-center rounded-lg transition-[box-shadow,background-color] duration-150 max-sm:size-[30px]",
                    !fitting && !draft.hex && draft.color === c
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
              <OwnSwatch
                ref={ownSwatch}
                hex={own}
                ink={
                  ownPaint && { color: ownPaint.ink, className: ownPaint.className, style: ownPaint.style }
                }
                checked={!fitting && !!draft.hex}
                open={!!editing}
                popover={popoverId}
                onClick={(byKeyboard) => {
                  if (editing) return closeEditor(byKeyboard);
                  // It starts from the color it has, or the workspace color it's in.
                  chooseOwn(own ?? (shown.color === "ink" ? FIRST_OWN : BRAND_HEX[shown.color]));
                  setEditing(byKeyboard ? "keyboard" : "pointer");
                }}
                onEscape={() => closeEditor(true)}
              />
            </div>
            <ColorPopover
              id={popoverId}
              open={!!editing}
              anchor={ownSwatch}
              hex={draft.hex ?? own ?? FIRST_OWN}
              preview={
                <IconThemes
                  hex={draft.hex ?? own ?? FIRST_OWN}
                  glyph={{ name: shown.name, style: shown.style }}
                />
              }
              used={used}
              workspace={project.workspace?.name}
              focusHex={editing === "keyboard"}
              onChange={chooseOwn}
              onClose={closeEditor}
            />
          </div>
          <div className="flex items-center gap-3">
            <span className="w-10 text-[12.5px] text-ink-3 max-sm:hidden">Style</span>
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
                    "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12.5px] transition-colors duration-150 max-sm:h-8 max-sm:px-3",
                    !fitting && draft.style === style
                      ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                      : "text-ink-3 hover:text-ink",
                  )}
                >
                  <Glyph name={shown.name} style={style} color={shown.color} hex={shown.hex} size={14} />
                  <span className="max-sm:sr-only">{name}</span>
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
            autoFocus={!touch}
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
        // As tall as it can be up to its height, but no shorter than about two rows of icons.
        className="-mx-2 -mt-2 h-[min(44vh,340px)] min-h-28 overflow-y-auto overscroll-contain px-2 [scrollbar-gutter:stable]"
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
                            onSave(toSave({ ...draft, name }));
                          }}
                          className={cn(
                            "group/i relative flex aspect-square items-center justify-center rounded-lg outline-offset-0 transition-[background-color,box-shadow,color,transform] duration-150 active:scale-90",
                            on
                              ? cn("bg-raised shadow-[inset_0_0_0_1.5px_var(--c-accent)]", paint.className)
                              : "text-ink-3 hover:bg-hover hover:text-ink focus-visible:text-ink",
                          )}
                          style={on ? { ...paint.style, color: paint.ink } : undefined}
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

      {/* What the icon under the pointer, or in focus, is. A touch screen has neither: the preview names it. */}
      <p
        className="-mt-3 flex h-4 items-center gap-2 truncate text-[12px] text-ink-4 pointer-coarse:hidden"
        aria-live="polite"
      >
        {about && (
          <>
            <span className="text-ink-2">{label(about)}</span>
            <span className="truncate">{iconDef(about)?.tags.slice(0, 4).join(", ")}</span>
          </>
        )}
      </p>

      {/* On a phone, the credit takes a line of its own and the buttons the one under it. */}
      <div className="-mx-6 -mb-6 flex flex-wrap items-center gap-x-2 gap-y-3 border-t border-line px-6 py-4">
        <p className="min-w-0 flex-1 text-[12px] text-ink-4 max-sm:basis-full">
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
        <div className="ml-auto flex items-center gap-2">
          {!fitting && (
            <Button variant="ghost" onClick={() => setFitting(true)}>
              Use the one that fits
            </Button>
          )}
          {/* On a phone, the dialog's × closes it: three buttons don't fit a row. */}
          <Button variant="ghost" onClick={onCancel} className="max-sm:hidden">
            Cancel
          </Button>
          <Button
            variant="primary"
            onClick={save}
            loading={saving}
            disabled={!changed}
            // Phones have no keys to press.
            trailing={
              <span className="max-sm:hidden">
                <Kbd>{apple() ? "⌘" : "Ctrl"}↵</Kbd>
              </span>
            }
          >
            Save
          </Button>
        </div>
      </div>
    </div>
  );
}

/** The picker for a project's icon, which saves it as it's chosen: its header's tile, its settings. */
export function ProjectIconPicker({
  project,
  open,
  onClose,
}: {
  project: G.Project;
  open: boolean;
  onClose: () => void;
}) {
  const save = useSaveIcon(project);
  return (
    <IconPicker
      open={open}
      onClose={onClose}
      project={project}
      saving={save.isPending}
      onSave={(icon) =>
        save.mutate(icon, {
          onSuccess: () => {
            onClose();
            toast(icon ? `${project.name} has its icon` : `${project.name} shows the icon that fits it`);
          },
          onError: (e) => toast(`The icon wasn't saved: ${e.message}`, "error"),
        })
      }
    />
  );
}

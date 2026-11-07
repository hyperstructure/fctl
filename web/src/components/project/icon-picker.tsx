// Choosing a project's icon (a design prototype): an IconMCP glyph, its style and its color,
// shown as the sidebar and the project's page will show it while you choose.

import { Search } from "lucide-react";
import { useMemo, useState } from "react";
import { Button, Dialog } from "@/components/ui";
import { cn } from "@/lib/cn";
import {
  chooseIcon,
  ICON_COLORS,
  ICON_GROUPS,
  ICON_STYLES,
  ICONS,
  type IconColor,
  type IconStyle,
  iconOf,
  type ProjectIconChoice,
  searchIcons,
  suggestIcons,
  useIconChoices,
} from "@/lib/project-icons";
import { Glyph, ProjectIcon } from "./project-icon";

export function IconPicker({
  project,
  open,
  onClose,
}: {
  project: { id: string; name: string; description?: string; active_sessions?: number };
  open: boolean;
  onClose: () => void;
}) {
  return (
    <Dialog
      open={open}
      onClose={onClose}
      wide
      title={`${project.name}'s icon`}
      description="How it shows in the sidebar, on its page and in search. Everyone in the workspace sees the same one."
    >
      {open && <Picker project={project} onClose={onClose} />}
    </Dialog>
  );
}

function Picker({
  project,
  onClose,
}: {
  project: { id: string; name: string; description?: string; active_sessions?: number };
  onClose: () => void;
}) {
  const choices = useIconChoices();
  const icon = iconOf(project, choices);
  const [query, setQuery] = useState("");
  const suggested = useMemo(() => suggestIcons(project, 10), [project]);
  const found = useMemo(() => searchIcons(query), [query]);
  const set = (change: Partial<ProjectIconChoice>) =>
    chooseIcon(project.id, { name: icon.name, color: icon.color, style: icon.style, ...change });
  return (
    <div className="flex flex-col gap-5">
      <div className="flex flex-wrap items-stretch gap-4">
        {/* As the sidebar and the project's page show it. */}
        <div className="flex min-w-[220px] flex-1 items-center gap-4 rounded-lg border border-line bg-sunken/70 px-4 py-3">
          <ProjectIcon icon={icon} tile={44} />
          <div className="min-w-0 flex-1">
            <div className="flex h-8 items-center gap-2.5 rounded-md bg-raised px-2.5 text-[13.5px] text-ink shadow-[inset_0_0_0_1px_var(--c-line)]">
              <ProjectIcon icon={icon} size={16} />
              <span className="min-w-0 flex-1 truncate">{project.name}</span>
              {!!project.active_sessions && (
                <span className="text-[11.5px] text-accent tabular-nums">{project.active_sessions}</span>
              )}
            </div>
            <p className="mt-1.5 truncate text-[12px] text-ink-4">
              {icon.chosen ? "Chosen for it" : "Picked from its name and description"}
            </p>
          </div>
        </div>
        <div className="flex flex-col justify-center gap-3">
          <div className="flex items-center gap-3">
            <span className="w-11 text-[12.5px] text-ink-3">Style</span>
            <div className="flex rounded-lg border border-line bg-sunken/60 p-0.5">
              {ICON_STYLES.map(({ style, label }) => (
                <button
                  key={style}
                  type="button"
                  aria-pressed={icon.style === style}
                  onClick={() => set({ style: style as IconStyle })}
                  className={cn(
                    "flex h-7 items-center gap-1.5 rounded-md px-2.5 text-[12.5px] transition-colors",
                    icon.style === style
                      ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                      : "text-ink-3 hover:text-ink",
                  )}
                >
                  <Glyph name="sparkles" style={style} size={14} />
                  {label}
                </button>
              ))}
            </div>
          </div>
          <div className="flex items-center gap-3">
            <span className="w-11 text-[12.5px] text-ink-3">Color</span>
            <div className="flex gap-1.5">
              {(Object.keys(ICON_COLORS) as IconColor[]).map((c) => (
                <button
                  key={c}
                  type="button"
                  aria-label={ICON_COLORS[c].label}
                  aria-pressed={icon.color === c}
                  title={ICON_COLORS[c].label}
                  onClick={() => set({ color: c })}
                  className={cn(
                    "flex size-8 items-center justify-center rounded-lg transition-[box-shadow,background-color]",
                    icon.color === c
                      ? "bg-raised shadow-[inset_0_0_0_1.5px_var(--c-ink-3)]"
                      : "hover:bg-hover",
                  )}
                >
                  <span
                    className="size-4 rounded-full shadow-[inset_0_0_0_1px_var(--c-line-strong)]"
                    style={{ background: c === "ink" ? "var(--c-ink-2)" : ICON_COLORS[c].ink }}
                  />
                </button>
              ))}
            </div>
          </div>
        </div>
      </div>
      <label className="flex h-9.5 items-center gap-2.5 rounded-lg border border-line-strong bg-sunken/60 px-3 text-[13.5px] focus-within:border-ink-4">
        <Search className="size-4 text-ink-4" />
        <input
          value={query}
          onChange={(e) => setQuery(e.target.value)}
          placeholder={`Search ${ICONS.length} icons: carrot, server, rocket…`}
          className="min-w-0 flex-1 bg-transparent text-ink outline-none placeholder:text-ink-4"
        />
      </label>
      <div className="-mx-2 max-h-[300px] overflow-y-auto px-2">
        {query ? (
          <Grid
            title={found.length ? `${found.length} found` : "Nothing found"}
            names={found.map((i) => i.name)}
            icon={icon}
            onPick={(name) => set({ name })}
          />
        ) : (
          <>
            <Grid
              title={`Fits ${project.name}`}
              names={suggested}
              icon={icon}
              onPick={(name) => set({ name })}
            />
            {ICON_GROUPS.map((g) => (
              <Grid
                key={g}
                title={g}
                names={ICONS.filter((i) => i.group === g).map((i) => i.name)}
                icon={icon}
                onPick={(name) => set({ name })}
              />
            ))}
          </>
        )}
      </div>
      <div className="-mx-6 -mb-6 flex items-center gap-3 border-t border-line px-6 py-4">
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
        {icon.chosen && (
          <Button variant="ghost" onClick={() => chooseIcon(project.id, undefined)}>
            Use the one that fits
          </Button>
        )}
        <Button variant="primary" onClick={onClose}>
          Done
        </Button>
      </div>
    </div>
  );
}

function Grid({
  title,
  names,
  icon,
  onPick,
}: {
  title: string;
  names: string[];
  icon: Pick<ProjectIconChoice, "name" | "color" | "style">;
  onPick: (name: string) => void;
}) {
  return (
    <section className="mb-4 last:mb-0">
      <h3 className="mb-1.5 text-[12px] font-medium text-ink-4">{title}</h3>
      <div className="grid grid-cols-[repeat(auto-fill,minmax(38px,1fr))] gap-1">
        {names.map((name) => {
          const on = name === icon.name;
          return (
            <button
              key={name}
              type="button"
              title={ICONS.find((i) => i.name === name)?.label ?? name}
              aria-label={ICONS.find((i) => i.name === name)?.label ?? name}
              aria-pressed={on}
              onClick={() => onPick(name)}
              className={cn(
                "flex aspect-square items-center justify-center rounded-lg transition-[background-color,box-shadow,color] duration-150",
                on
                  ? "bg-raised shadow-[inset_0_0_0_1.5px_var(--c-accent)]"
                  : "text-ink-3 hover:bg-hover hover:text-ink",
              )}
            >
              <Glyph name={name} style={icon.style} color={on ? icon.color : undefined} size={20} />
            </button>
          );
        })}
      </div>
    </section>
  );
}

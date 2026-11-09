// The pages of a settings area, two ways: a list beside the page open, where both fit, whose
// highlight slides to the page chosen; and on a phone the list alone, each page with what it says
// now, as a phone's own settings app has them. Settings (routes/settings.tsx) and a project's
// Settings tab (components/project/settings.tsx) use them.

import { ChevronRight } from "lucide-react";
import { type ReactNode, useLayoutEffect, useRef, useState } from "react";
import { cn } from "@/lib/cn";
import type { Gist } from "@/lib/settings";

export interface NavEntry<K extends string> {
  key: K;
  label: string;
  /** Its icon, drawn with the size and color the list gives it. */
  icon: (className: string) => ReactNode;
}

/** What a list hands its caller to draw a link to a page with: the router's `Link`, to its address. */
export interface NavLinkProps {
  className: string;
  "aria-current"?: "page";
  children: ReactNode;
}

type Groups<K extends string> = readonly (readonly NavEntry<K>[])[];

/** Groups with something in them: a group whose pages someone can't see goes, with its hairline. */
const filled = <K extends string>(groups: Groups<K>) => groups.filter((g) => g.length > 0);

/** The pages beside the one open: each with its icon, and a mark on what needs the person. */
export function SettingsNav<K extends string>({
  label,
  groups,
  current,
  attention = {},
  link,
  className,
}: {
  /** What the navigation is called, for screen readers. */
  label: string;
  groups: Groups<K>;
  current: K;
  attention?: Partial<Record<K, Gist>>;
  link: (key: K, props: NavLinkProps) => ReactNode;
  className?: string;
}) {
  const nav = useRef<HTMLElement>(null);
  // The highlight under the page open, which slides to the page chosen. Where it was first
  // measured it shows at once: it moves only from one page to another.
  const [mark, setMark] = useState<{ top: number; height: number; moved: boolean }>();
  const shown = groups
    .flat()
    .map((p) => p.key)
    .join();
  // biome-ignore lint/correctness/useExhaustiveDependencies: again as another page opens, or the list changes
  useLayoutEffect(() => {
    const list = nav.current;
    if (!list) return;
    const measure = () => {
      const here = list.querySelector<HTMLElement>('[aria-current="page"]');
      // Hidden, where the window is narrow: measured once it shows.
      if (!here || !list.offsetHeight) return;
      // From the top of the list, past the groups above.
      const top = here.getBoundingClientRect().top - list.getBoundingClientRect().top;
      const height = here.offsetHeight;
      setMark((m) => (m?.top === top && m.height === height ? m : { top, height, moved: !!m }));
    };
    measure();
    const resized = typeof ResizeObserver === "undefined" ? undefined : new ResizeObserver(measure);
    resized?.observe(list);
    return () => resized?.disconnect();
  }, [current, shown]);
  return (
    // Its icons line up with the page's title, their highlight in the margin beside it. The
    // router's view transition leaves it out of its cross-fade (styles/app.css), so that the
    // highlight slides rather than fading out in one place and in at another.
    <nav
      ref={nav}
      aria-label={label}
      className={cn("sticky top-8 -ml-2.5 [view-transition-name:settings-pages] max-md:top-20", className)}
    >
      {mark && (
        <span
          aria-hidden="true"
          className={cn(
            "absolute inset-x-0 top-0 rounded-md bg-raised shadow-[inset_0_0_0_1px_var(--c-line)]",
            mark.moved && "transition-[translate] duration-[260ms] ease-[var(--ease-out)]",
          )}
          style={{ translate: `0 ${mark.top}px`, height: mark.height }}
        />
      )}
      {filled(groups).map((group, i) => (
        <ul
          key={group[0]?.key}
          // Over the highlight, which is positioned (absolute): so are the groups (relative).
          className={cn("relative flex flex-col gap-0.5", i > 0 && "mt-3 border-t border-line pt-3")}
        >
          {group.map((p) => {
            const here = p.key === current;
            const needs = attention[p.key];
            return (
              <li key={p.key}>
                {link(p.key, {
                  "aria-current": here ? "page" : undefined,
                  className: cn(
                    "group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
                    here ? "text-ink" : "text-ink-3 hover:bg-hover hover:text-ink",
                  ),
                  children: (
                    <>
                      {p.icon(
                        cn(
                          "size-4 shrink-0 transition-colors duration-150",
                          here ? "text-accent" : "text-ink-4 group-hover:text-ink-3",
                        ),
                      )}
                      <span className="min-w-0 flex-1 truncate">{p.label}</span>
                      {needs && (
                        <span
                          title={needs.text}
                          className="size-1.5 shrink-0 rounded-full bg-amber shadow-[0_0_6px_var(--c-amber)]"
                        >
                          <span className="sr-only">: {needs.text}</span>
                        </span>
                      )}
                    </>
                  ),
                })}
              </li>
            );
          })}
        </ul>
      ))}
    </nav>
  );
}

/**
 * The pages alone, on a phone: what comes first (`head`), then the pages in their groups as inset
 * cards, each with what it says now, in amber where it needs the person. Each opens on its own.
 */
export function SettingsList<K extends string>({
  label,
  head,
  groups,
  gists,
  link,
  className,
}: {
  label: string;
  head?: ReactNode;
  groups: Groups<K>;
  gists: Partial<Record<K, Gist>>;
  link: (key: K, props: NavLinkProps) => ReactNode;
  className?: string;
}) {
  return (
    <nav aria-label={label} className={cn("flex flex-col gap-5", className)}>
      {head}
      {filled(groups).map((group) => (
        <ul
          key={group[0]?.key}
          className="divide-y divide-line overflow-hidden rounded-xl border border-line bg-surface"
        >
          {group.map((p) => {
            const gist = gists[p.key];
            return (
              <li key={p.key}>
                {link(p.key, {
                  className:
                    "flex min-h-[58px] items-center gap-3 py-2.5 pr-3.5 pl-3 transition-colors hover:bg-hover active:bg-hover",
                  children: (
                    <>
                      <span className="flex size-8 shrink-0 items-center justify-center rounded-lg bg-raised text-ink-3 shadow-[inset_0_0_0_1px_var(--c-line)]">
                        {p.icon("size-4")}
                      </span>
                      <span className="min-w-0 flex-1">
                        <span className="block truncate text-[14.5px] text-ink">{p.label}</span>
                        {gist && (
                          <span
                            className={cn(
                              "block truncate text-[12.5px]",
                              gist.attention ? "text-amber" : "text-ink-4",
                            )}
                          >
                            {gist.text}
                          </span>
                        )}
                      </span>
                      <ChevronRight className="size-4 shrink-0 text-ink-4" />
                    </>
                  ),
                })}
              </li>
            );
          })}
        </ul>
      ))}
    </nav>
  );
}

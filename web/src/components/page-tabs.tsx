// The tabs of a project's, a workspace's and the admin page: links, each with its icon, the open one
// underlined in the accent. Where they don't fit (phones) they scroll sideways, and the open one
// scrolls into view.

import { Link, type LinkProps } from "@tanstack/react-router";
import { type ReactNode, useLayoutEffect, useRef } from "react";
import { cn } from "@/lib/cn";

export interface PageTab<T extends string> {
  id: T;
  label: string;
  /** Its icon, drawn with this class: 16 px, in the tab's color. */
  icon: (className: string) => ReactNode;
  /** A color of its own for the icon, open or not, where it says how something is. */
  tint?: string;
  /** A count after its label. */
  badge?: ReactNode;
}

/** The size and colors of a tab's icon: in the accent on the open tab, lighter as the pointer comes. */
const iconClass = (here: boolean, tint?: string) =>
  cn(
    "size-4 shrink-0 transition-colors duration-150",
    tint ?? (here ? "text-accent" : "text-ink-4 group-hover:text-ink-3"),
  );

/** A page's tabs, each an address: `link` says where each goes; the default one should have none. */
export function PageTabs<T extends string>({
  label,
  tabs,
  open,
  link,
}: {
  /** What the tabs are of, for screen readers: "Project". */
  label: string;
  tabs: PageTab<T>[];
  open: T;
  link: (id: T) => LinkProps;
}) {
  const strip = useRef<HTMLElement>(null);
  const shown = tabs.map((t) => t.id).join();
  // biome-ignore lint/correctness/useExhaustiveDependencies: again as another tab opens, or the tabs change
  useLayoutEffect(() => {
    const nav = strip.current;
    const here = nav?.querySelector<HTMLElement>('[aria-current="page"]');
    if (!nav || !here || nav.scrollWidth <= nav.clientWidth) return;
    const a = here.getBoundingClientRect();
    const v = nav.getBoundingClientRect();
    if (a.left >= v.left && a.right <= v.right) return;
    // Sideways only: the page stays where it is.
    nav.scrollLeft += a.left + a.width / 2 - (v.left + v.width / 2);
  }, [open, shown]);
  return (
    <nav ref={strip} aria-label={label} className="-mx-6 mt-8 overflow-x-auto px-6 [scrollbar-width:none]">
      <div className="flex w-max min-w-full gap-1 border-b border-line">
        {tabs.map((t) => {
          const here = open === t.id;
          return (
            <Link
              key={t.id}
              {...link(t.id)}
              // The page says which tab is open: a page under a tab keeps it open, and TanStack's
              // Link, matching search partly, would call the default tab open on every tab.
              aria-current={here ? "page" : undefined}
              activeOptions={{ exact: true }}
              className={cn(
                "group relative inline-flex shrink-0 items-center gap-1.5 px-2.5 py-2.5 text-[13.5px] transition-colors duration-150",
                here ? "text-ink" : "text-ink-3 hover:text-ink",
              )}
            >
              {t.icon(iconClass(here, t.tint))}
              {t.label}
              {t.badge}
              {here && (
                <span className="absolute inset-x-1.5 -bottom-px h-[2px] rounded-full bg-accent forced-colors:bg-[CanvasText] forced-color-adjust-none" />
              )}
            </Link>
          );
        })}
      </div>
    </nav>
  );
}

/** A count on a tab: how many wait for you there. */
export function TabCount({
  n,
  label,
  tone = "highlight",
}: {
  n: number;
  label: string;
  tone?: "highlight" | "blue";
}) {
  return (
    <span
      role="img"
      aria-label={label}
      title={label}
      className={cn(
        "inline-flex h-4 min-w-4 items-center justify-center rounded-full px-1 text-[10px] font-semibold tabular-nums",
        tone === "blue" ? "bg-blue text-white" : "bg-highlight text-highlight-ink",
      )}
    >
      {n}
    </span>
  );
}

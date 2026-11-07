// A row of a list that opens something as a whole: a session on the Agent Floor, a project on its
// workspace's page. No arrow says so: the row fills under the pointer and around the keyboard's
// focus, as a rounded tile that reaches a little past the list's edges, so that what it holds lines
// up with the heading above, and the lines on either side of it give way.

import { cn } from "@/lib/cn";

/**
 * The row: one of the list's children, each under a line, which goes while it or the row above it
 * fills: under the pointer, as a finger presses it (touch screens have no hover), or as its link
 * (`ROW_COVER`) has the keyboard's focus.
 */
export const ROW = cn(
  "relative isolate -mx-3 rounded-lg px-3 transition-colors duration-150",
  "hover:bg-hover active:bg-hover has-[[data-cover]:focus-visible]:bg-hover",
  // The line above it, as wide as the list: a border, which forced colors keep, as they don't a fill.
  "before:pointer-events-none before:absolute before:inset-x-3 before:top-0 before:border-t before:border-line before:transition-opacity before:duration-150",
  "hover:before:opacity-0 active:before:opacity-0 has-[[data-cover]:focus-visible]:before:opacity-0",
  // Only where hover does, as for the row itself: a touch screen leaves what it tapped hovered.
  "[@media(hover:hover)]:[:hover+&]:before:opacity-0",
  "[:active+&]:before:opacity-0 [:has([data-cover]:focus-visible)+&]:before:opacity-0",
);

/**
 * The link that opens the row, marked `data-cover`: its ::after covers the row, so that a click
 * anywhere on it opens it, and draws its focus ring. A row that holds other links (its pull
 * requests, say) has it on its title, since a link can't hold links: they lie over it
 * (`ROW_ABOVE`). The ring is an outline, inside the tile's edge: forced colors (Windows' contrast
 * themes) keep outlines, where they drop shadows and the fill.
 */
export const ROW_COVER =
  "outline-none after:absolute after:inset-0 after:z-[1] after:rounded-lg focus-visible:after:outline-2 focus-visible:after:-outline-offset-2 focus-visible:after:outline-accent";

/** What lies over a row's cover link: the links in it, and what says more where the pointer rests. */
export const ROW_ABOVE = "relative z-[2]";

// A file's path on one line, as the transcript and the diff dialog show it: its directory dimmer,
// giving way first when room runs out, and its name.

import type { HTMLAttributes } from "react";
import { cn } from "@/lib/cn";

/** A path's directory, with its slash, and its name. */
export const splitPath = (path: string) => {
  const cut = path.lastIndexOf("/") + 1;
  return { dir: path.slice(0, cut), name: path.slice(cut) };
};

/**
 * A path on one line: its directory (`ink-4`) truncates first, and its name keeps its whole width
 * until the directory is gone, then truncates too. The name never shrinks, and is capped at the
 * line's width instead: given any share of the shrinking, however small, Safari took a fraction of a
 * pixel from it and drew its ellipsis over the last letters ("tool.t…"). `nameClassName` styles the
 * name; the rest goes on the line.
 */
export function PathText({
  path,
  className,
  nameClassName,
  ...rest
}: { path: string; nameClassName?: string } & HTMLAttributes<HTMLSpanElement>) {
  const { dir, name } = splitPath(path);
  return (
    <span {...rest} className={cn("flex min-w-0", className)}>
      {dir && <span className="min-w-0 truncate text-ink-4">{dir}</span>}
      <span className={cn("max-w-full shrink-0 truncate", nameClassName)}>{name}</span>
    </span>
  );
}

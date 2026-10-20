// On phones the sidebar hides, and the menu button opens it. The shell's top
// bar holds it, except on a page whose own header sticks to the top of the
// screen (the session's): that header would cover the bar, so it holds the
// button instead, and the shell leaves the bar out.

import { Menu } from "lucide-react";
import { createContext, useContext, useLayoutEffect, useMemo, useState } from "react";
import { cn } from "@/lib/cn";

/** What the shell gives pages: the sidebar, and the top bar's place. */
interface Shell {
  openSidebar: () => void;
  /** A header shows the menu button, until the function this returns is called. */
  addHeaderButton: () => () => void;
}

/** Pages get it from the shell, which makes it with `useHeaderButtons`. */
export const ShellContext = createContext<Shell | null>(null);

/**
 * For the shell: whether a page's header holds the menu button, which then
 * takes the top bar's place, and what to give pages through `ShellContext`.
 */
export function useHeaderButtons(openSidebar: () => void): [boolean, Shell] {
  const [buttons, setButtons] = useState(0);
  const shell = useMemo(
    () => ({
      openSidebar,
      addHeaderButton: () => {
        setButtons((n) => n + 1);
        return () => setButtons((n) => n - 1);
      },
    }),
    [openSidebar],
  );
  return [buttons > 0, shell];
}

/** The button that opens the sidebar: 44 px, which a thumb hits easily. */
export function MenuButton({ onClick, className }: { onClick: () => void; className?: string }) {
  return (
    <button
      type="button"
      aria-label="Open menu"
      className={cn(
        "flex size-11 shrink-0 items-center justify-center rounded-lg text-ink-3 hover:text-ink",
        className,
      )}
      onClick={onClick}
    >
      <Menu className="size-6" />
    </button>
  );
}

/** The menu button in a page's header, on phones, in place of the top bar. */
export function HeaderMenuButton({ className }: { className?: string }) {
  const shell = useContext(ShellContext);
  // Before the page first paints, so that the bar never shows with the header.
  useLayoutEffect(() => shell?.addHeaderButton(), [shell]);
  return <MenuButton onClick={() => shell?.openSidebar()} className={cn("md:hidden", className)} />;
}

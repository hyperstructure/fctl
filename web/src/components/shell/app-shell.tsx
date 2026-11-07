// The signed-in layout: the sidebar (./sidebar.tsx), the page, and the command palette.

import { Outlet, useRouter, useRouterState } from "@tanstack/react-router";
import { type CSSProperties, useCallback, useEffect, useRef, useState, useSyncExternalStore } from "react";
import { Logo, toast } from "@/components/ui";
import { cn } from "@/lib/cn";
import { useFeed } from "@/lib/feed";
import { gravatarNews, gravatarPicture } from "@/lib/gravatar";
import { useHotkey } from "@/lib/hotkeys";
import { onPushOpen, syncPush } from "@/lib/push";
import { useMe } from "@/lib/queries";
import { RAIL_WIDTH, SIDEBAR_WIDTH, useSidebarFolded, useSidebarWidth } from "@/lib/sidebar-layout";
import { useFactorsWatch } from "@/lib/workspaces";
import { CommandPalette, usePaletteKeys } from "./command-palette";
import { useDrawerDrag } from "./drawer-drag";
import { MenuButton, ShellContext, useHeaderButtons } from "./menu-button";
import { Sidebar } from "./sidebar";
import { ResizeHandle } from "./sidebar-parts";

/**
 * The sidebar on phones: open, closed with its button or the shade (it slides away), or
 * gone, closed by one of its links or by an entry of the palette opened from it (at once); or
 * dragged, in or out, by a finger (it follows the finger, then slides open or closed).
 */
type Drawer = "open" | "closed" | "gone" | "dragged";

export function AppShell() {
  const [palette, setPalette] = useState(false);
  const [drawer, setDrawer] = useState<Drawer>("closed");
  const openPalette = useCallback(() => setPalette(true), []);
  const openDrawer = useCallback(() => setDrawer("open"), []);
  const closeDrawer = () => setDrawer("closed");
  const dropDrawer = () => setDrawer("gone");
  const root = useRef<HTMLDivElement>(null);
  const aside = useRef<HTMLElement>(null);
  const shade = useRef<HTMLButtonElement>(null);
  useDrawerDrag({
    page: root,
    drawer: aside,
    shade,
    open: drawer === "open",
    onDrag: () => setDrawer("dragged"),
    onLetGo: (open) => setDrawer(open ? "open" : "closed"),
  });
  const shown = drawer === "open" || drawer === "dragged";
  // A page whose header holds the menu button (the session's) takes the top bar's place.
  const [headerButton, shell] = useHeaderButtons(openDrawer);
  usePaletteKeys(openPalette);
  useFeed(); // live updates for every page
  usePush();
  useGravatarNews();
  useFactorsWatch();
  const path = useRouterState({ select: (s) => s.location.pathname });
  const tab = useRouterState({ select: (s) => (s.location.search as { tab?: string } | undefined)?.tab });
  // How wide the sidebar is, and whether it is folded into its rail: as each person leaves it.
  const [width, setWidth] = useSidebarWidth();
  const [folded, setFolded] = useSidebarFolded();
  const [resizing, setResizing] = useState(false);
  const wide = useWide();
  useHotkey(
    "mod+\\",
    useCallback(() => setFolded(!folded), [folded, setFolded]),
  );
  return (
    <div
      ref={root}
      className="flex min-h-dvh bg-canvas"
      style={{ "--sidebar-w": `${folded && wide ? RAIL_WIDTH : width}px` } as CSSProperties}
    >
      {/* On phones the sidebar is a drawer over the page: opaque, and out of the page once
          it has slid away (display: none), so that it is drawn anew each time it opens.
          Kept off screen with a blur, it sometimes stopped showing properly in Safari on iOS
          until a reload. A link in it, or an entry of the palette opened from it, takes it
          out at once, without the slide: the router's view transition snapshots the page as
          the link is followed, and a drawer that had only started to slide away showed there
          open, fading out over the page the link opened. A finger drags it in from the left
          edge of the screen, and out by it or the shade (./drawer-drag.ts). */}
      <aside
        ref={aside}
        className={cn(
          "fixed top-(--viewport-shift) -bottom-(--viewport-shift) left-0 z-30 flex w-[min(calc(100vw-3rem),calc(300px+env(safe-area-inset-left)))] flex-col border-r border-line bg-surface pl-[env(safe-area-inset-left)] max-md:starting:-translate-x-full md:w-[calc(var(--sidebar-w)+env(safe-area-inset-left))] md:translate-x-0 md:bg-surface/80 md:backdrop-blur-xl",
          drawer !== "gone" &&
            "transition-[translate,display] transition-discrete duration-300 ease-out motion-reduce:transition-none",
          // Folding animates its width; dragging its edge follows the pointer at once.
          !resizing &&
            "md:transition-[width] md:duration-200 md:ease-[var(--ease-out)] md:motion-reduce:transition-none",
          shown ? "translate-x-0" : "-translate-x-full max-md:hidden",
        )}
      >
        <Sidebar
          onPalette={openPalette}
          onNavigate={dropDrawer}
          onClose={closeDrawer}
          path={path}
          tab={tab}
          folded={folded && wide}
          onFold={setFolded}
        />
        {!(folded && wide) && (
          <ResizeHandle
            width={width}
            min={SIDEBAR_WIDTH.min}
            max={SIDEBAR_WIDTH.max}
            onResize={setWidth}
            onDragging={setResizing}
          />
        )}
      </aside>
      {/* The shade fades in and out with the drawer's slide (and goes with it at once). */}
      <button
        ref={shade}
        type="button"
        aria-label="Close menu"
        className={cn(
          "fixed inset-x-0 top-(--viewport-shift) -bottom-(--viewport-shift) z-20 bg-black/40 md:hidden",
          drawer !== "gone" &&
            "transition-[opacity,visibility] transition-discrete duration-300 ease-out motion-reduce:transition-none",
          !shown && "pointer-events-none invisible opacity-0",
        )}
        onClick={closeDrawer}
      />
      {/* The page is never wider than the window: what is wider scrolls in its own row. The
          clip, unlike hidden, keeps the sticky bars working. */}
      <div
        className={cn(
          "flex min-w-0 flex-1 flex-col overflow-x-clip pr-[env(safe-area-inset-right)] pl-[env(safe-area-inset-left)] md:pl-[calc(var(--sidebar-w)+env(safe-area-inset-left))]",
          !resizing &&
            "md:transition-[padding] md:duration-200 md:ease-[var(--ease-out)] md:motion-reduce:transition-none",
        )}
      >
        {/* On phones: the menu button, its icon in line with the page's text. */}
        {!headerButton && (
          <div className="sticky top-(--viewport-shift) z-10 flex h-14 items-center gap-1.5 border-b border-line bg-canvas/80 px-2.5 backdrop-blur-xl md:hidden">
            <MenuButton onClick={openDrawer} />
            <Logo large />
          </div>
        )}
        <main className="min-w-0 flex-1">
          <ShellContext value={shell}>
            <Outlet />
          </ShellContext>
        </main>
      </div>
      <CommandPalette open={palette} onClose={() => setPalette(false)} onNavigate={dropDrawer} />
    </div>
  );
}

/** Says once that their picture is their Gravatar now: where it came from, and where to change it. */
function useGravatarNews() {
  const { data: me } = useMe();
  const picture = gravatarPicture(me);
  useEffect(() => {
    if (gravatarNews(picture)) toast("Your picture is your Gravatar now. Change it in Settings → Profile.");
  }, [picture]);
}

/** Notifications in this browser: keep its subscription current, and open what a click is about. */
function usePush() {
  const { data: me } = useMe();
  const router = useRouter();
  const userId = me?.user.id;
  useEffect(() => {
    if (userId) syncPush(userId).catch((e) => console.warn("fctl: notifications", e));
  }, [userId]);
  useEffect(() => onPushOpen((url) => router.history.push(url)), [router]);
}

/** Whether the window is wide enough for the sidebar beside the page (md, 768 px). */
function useWide(): boolean {
  return useSyncExternalStore(
    (fn) => {
      const q = window.matchMedia?.("(min-width: 48rem)");
      q?.addEventListener?.("change", fn);
      return () => q?.removeEventListener?.("change", fn);
    },
    () => window.matchMedia?.("(min-width: 48rem)").matches ?? true,
    () => true,
  );
}

export { Sidebar };

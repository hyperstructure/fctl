// The signed-in layout: the sidebar (./sidebar.tsx), the page, the command palette, and the
// shortcuts that work anywhere (lib/shortcuts.ts): the new session dialog and the keyboard
// shortcuts sheet among them.

import { Outlet, useNavigate, useRouter, useRouterState } from "@tanstack/react-router";
import { type CSSProperties, useCallback, useEffect, useRef, useState } from "react";
import { Logo, toast } from "@/components/ui";
import { cn } from "@/lib/cn";
import { useFeed } from "@/lib/feed";
import { gravatarNews, gravatarPicture } from "@/lib/gravatar";
import { useMedia } from "@/lib/media";
import { onPushOpen, syncPush } from "@/lib/push";
import { useMe } from "@/lib/queries";
import { type GroupId, useShortcut } from "@/lib/shortcuts";
import { RAIL_WIDTH, useSidebarFolded, useSidebarWidth } from "@/lib/sidebar-layout";
import { focusPageTaskBox, focusTaskBox } from "@/lib/task-box";
import { useFactorsWatch } from "@/lib/workspaces";
import { CommandPalette, usePaletteKeys } from "./command-palette";
import { useDrawerDrag } from "./drawer-drag";
import { MenuButton, ShellContext, useHeaderButtons } from "./menu-button";
import { NewSessionDialog } from "./new-session";
import { SequenceHint, ShortcutsDialog } from "./shortcuts";
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
  const scope = useRouterState({
    select: (s) => (s.location.search as { scope?: string } | undefined)?.scope,
  });
  // How wide the sidebar is, and whether it is folded into its rail: as each person leaves it.
  const [width, setWidth] = useSidebarWidth();
  const [folded, setFolded] = useSidebarFolded();
  const [resizing, setResizing] = useState(false);
  // Wide enough for the sidebar beside the page (md, 768 px).
  const wide = useMedia("(min-width: 48rem)");
  // Only where it shows: on a narrow screen the sidebar is a drawer, which doesn't fold. ⌘B, as
  // in code editors, takes no Shift or Option on any layout; ⌘\ too, where \ is a key of its own.
  useShortcut("fold", () => {
    if (wide) setFolded(!folded);
  });
  const navigate = useNavigate();
  useShortcut("floor", () => {
    void navigate({ to: "/" });
    focusTaskBox();
  });
  useShortcut("pulls", () => void navigate({ to: "/pulls", search: {} }));
  useShortcut("settings", () => void navigate({ to: "/settings", search: {} }));
  // N goes to the task box the page shows, or brings the Agent Floor's over it: on a phone, from ⌘K,
  // the Agent Floor opens instead, whose box has the room.
  const [starting, setStarting] = useState(false);
  const newSession = () => {
    setPalette(false);
    if (focusPageTaskBox()) return;
    if (wide) setStarting(true);
    else {
      void navigate({ to: "/" });
      focusTaskBox();
    }
  };
  useShortcut("newSession", newSession);
  // ? or ⌘/, from a dialog too (the changes'), whose shortcuts it then lists first.
  const [help, setHelp] = useState<GroupId[]>();
  const showHelp = () => {
    setPalette(false);
    setHelp(here(path));
  };
  useShortcut("help", () => (help ? setHelp(undefined) : showHelp()), { inDialogs: true });
  // They belong to the page they opened on: another page closes them, as a session starting does.
  const [overlaysOf, setOverlaysOf] = useState(path);
  if (overlaysOf !== path) {
    setOverlaysOf(path);
    setStarting(false);
    setHelp(undefined);
  }
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
          "fixed top-(--viewport-shift) -bottom-(--viewport-shift) left-0 z-30 flex w-[min(calc(100vw-3rem),calc(300px+env(safe-area-inset-left)))] flex-col bg-surface pl-[env(safe-area-inset-left)] max-md:starting:-translate-x-full md:w-[calc(var(--sidebar-w)+env(safe-area-inset-left))] md:translate-x-0 md:bg-surface/80 md:backdrop-blur-xl",
          drawer !== "gone" &&
            "transition-[translate,display] transition-discrete duration-300 ease-out motion-reduce:transition-none",
          // Folding animates its width, as does dragging its edge into the rail or out of it;
          // otherwise the edge follows the pointer at once.
          !resizing &&
            "md:transition-[width] md:duration-200 md:ease-[var(--ease-out)] md:motion-reduce:transition-none",
          shown ? "translate-x-0" : "-translate-x-full max-md:hidden",
        )}
      >
        <Sidebar
          onPalette={openPalette}
          onShortcuts={showHelp}
          onNavigate={dropDrawer}
          onClose={closeDrawer}
          path={path}
          tab={tab}
          scope={path === "/pulls" ? scope : undefined}
          folded={folded && wide}
          onFold={setFolded}
        />
        {/* On the rail too, whose edge drags the sidebar out of it: the same element, so that it
            keeps the pointer as the sidebar unfolds under it. */}
        <ResizeHandle
          width={width}
          folded={folded && wide}
          onResize={setWidth}
          onFold={setFolded}
          onFollow={setResizing}
        />
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
          <div className="sticky top-(--viewport-shift) z-10 flex h-14 items-center gap-1.5 bg-canvas/80 px-2.5 backdrop-blur-xl md:hidden">
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
      <CommandPalette
        open={palette}
        onClose={() => setPalette(false)}
        onNavigate={dropDrawer}
        onNewSession={() => {
          dropDrawer();
          newSession();
        }}
        onShortcuts={showHelp}
      />
      <NewSessionDialog open={starting} onClose={() => setStarting(false)} />
      <ShortcutsDialog open={!!help} onClose={() => setHelp(undefined)} here={help ?? []} />
      <SequenceHint />
    </div>
  );
}

/**
 * The groups of shortcuts for what is in front of the reader on the page at `path`, or in the dialog
 * over it: the keyboard shortcuts sheet lists them first.
 */
function here(path: string): GroupId[] {
  if (document.querySelector("dialog[open][data-shortcuts='changes']")) return ["changes", "find"];
  if (path.startsWith("/s/")) return ["session", "find"];
  if (path === "/pulls") return ["pulls"];
  if (path === "/" || path.startsWith("/p/")) return ["writing"];
  return [];
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

export { Sidebar };

// The signed-in layout: a quiet sidebar, the page, and the command palette.

import { Link, Outlet, useNavigate, useRouterState } from "@tanstack/react-router";
import {
  Cpu,
  FolderPlus,
  Home as HomeIcon,
  LogOut,
  Menu as MenuIcon,
  Moon,
  Plus,
  Search,
  Settings,
  Shield,
  Sun,
  X,
} from "lucide-react";
import { useCallback, useState } from "react";
import { Avatar, Kbd, Logo, Menu, MenuItem, StatusDot } from "@/components/ui";
import { api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { useFeed } from "@/lib/feed";
import { useHotkey } from "@/lib/hotkeys";
import { queryClient, useInstance, useMe, useProjects } from "@/lib/queries";
import { useTheme } from "@/lib/theme";
import { CommandPalette } from "./command-palette";

export function AppShell() {
  const [palette, setPalette] = useState(false);
  const [drawer, setDrawer] = useState(false);
  const openPalette = useCallback(() => setPalette(true), []);
  useHotkey("mod+k", openPalette, { inInputs: true });
  useFeed(); // live updates for every page
  const path = useRouterState({ select: (s) => s.location.pathname });
  return (
    <div className="flex min-h-dvh bg-canvas">
      <aside
        className={cn(
          "fixed inset-y-0 left-0 z-30 flex w-[calc(244px+env(safe-area-inset-left))] flex-col border-r border-line bg-surface/80 pl-[env(safe-area-inset-left)] backdrop-blur-xl transition-transform duration-300 ease-out md:translate-x-0",
          drawer ? "translate-x-0" : "-translate-x-full",
        )}
      >
        <Sidebar onPalette={openPalette} onNavigate={() => setDrawer(false)} path={path} />
      </aside>
      {drawer && (
        <button
          type="button"
          aria-label="Close menu"
          className="fixed inset-0 z-20 bg-black/40 md:hidden"
          onClick={() => setDrawer(false)}
        />
      )}
      <div className="flex min-w-0 flex-1 flex-col pr-[env(safe-area-inset-right)] pl-[env(safe-area-inset-left)] md:pl-[calc(244px+env(safe-area-inset-left))]">
        <div className="sticky top-0 z-10 flex h-12 items-center gap-2 border-b border-line bg-canvas/80 px-3 backdrop-blur-xl md:hidden">
          <button
            type="button"
            aria-label="Open menu"
            className="rounded-md p-2 text-ink-3 hover:text-ink"
            onClick={() => setDrawer(true)}
          >
            <MenuIcon className="size-4.5" />
          </button>
          <Logo />
        </div>
        <main className="min-w-0 flex-1">
          <Outlet />
        </main>
      </div>
      <CommandPalette open={palette} onClose={() => setPalette(false)} />
    </div>
  );
}

function NavLink({
  to,
  icon,
  children,
  active,
  onClick,
}: {
  to: string;
  icon: React.ReactNode;
  children: React.ReactNode;
  active: boolean;
  onClick: () => void;
}) {
  return (
    <Link
      to={to}
      onClick={onClick}
      className={cn(
        "group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
        active
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "text-ink-3 hover:bg-raised/60 hover:text-ink",
      )}
    >
      <span
        className={cn(
          "transition-colors [&_svg]:size-4",
          active ? "text-accent" : "text-ink-4 group-hover:text-ink-3",
        )}
      >
        {icon}
      </span>
      {children}
    </Link>
  );
}

function Sidebar({
  onPalette,
  onNavigate,
  path,
}: {
  onPalette: () => void;
  onNavigate: () => void;
  path: string;
}) {
  const { data: me } = useMe();
  const { data: instance } = useInstance();
  const { data: projects } = useProjects();
  const [theme, setTheme] = useTheme();
  const navigate = useNavigate();
  const isAdmin = me?.user.role === "owner" || me?.user.role === "admin";
  return (
    <div className="flex h-full flex-col">
      <div className="flex h-14 items-center justify-between px-4">
        <Link to="/" onClick={onNavigate} className="rounded-md">
          <Logo />
        </Link>
        <button
          type="button"
          aria-label="Close menu"
          className="rounded-md p-1.5 text-ink-4 hover:text-ink md:hidden"
          onClick={onNavigate}
        >
          <X className="size-4" />
        </button>
      </div>
      <div className="px-3">
        <button
          type="button"
          onClick={onPalette}
          className="flex h-8.5 w-full items-center gap-2.5 rounded-md border border-line bg-sunken/60 px-2.5 text-[13px] text-ink-4 transition-colors hover:border-line-strong hover:text-ink-3"
        >
          <Search className="size-3.5" />
          <span className="flex-1 text-left">Search or jump to…</span>
          <Kbd>⌘K</Kbd>
        </button>
      </div>
      <nav className="mt-4 flex flex-col gap-0.5 px-3">
        <NavLink to="/" icon={<HomeIcon />} active={path === "/"} onClick={onNavigate}>
          Floor
        </NavLink>
      </nav>
      <div className="mt-6 flex items-center justify-between px-5 pb-1.5">
        <span className="text-[12.5px] font-medium text-ink-3">Projects</span>
        <Link
          to="/new"
          onClick={onNavigate}
          aria-label="New project"
          className="rounded p-0.5 text-ink-4 transition-colors hover:text-ink"
        >
          <Plus className="size-3.5" />
        </Link>
      </div>
      <nav className="flex min-h-0 flex-1 flex-col gap-0.5 overflow-y-auto px-3 pb-3">
        {projects?.projects.map((p) => (
          <Link
            key={p.id}
            to="/p/$slug"
            params={{ slug: p.slug }}
            search={{ tab: undefined }}
            onClick={onNavigate}
            className={cn(
              "flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
              path === `/p/${p.slug}`
                ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
                : "text-ink-3 hover:bg-raised/60 hover:text-ink",
            )}
          >
            <span className="flex size-4 items-center justify-center">
              {p.active_sessions > 0 ? (
                <StatusDot status="running" />
              ) : (
                <span className="size-1.5 rounded-full bg-ink-4/60" />
              )}
            </span>
            <span className="min-w-0 flex-1 truncate">{p.name}</span>
            {p.active_sessions > 0 && (
              <span className="text-[11.5px] text-ink-4 tabular-nums">{p.active_sessions}</span>
            )}
          </Link>
        ))}
        {projects && projects.projects.length === 0 && (
          <Link
            to="/new"
            onClick={onNavigate}
            className="flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13px] text-ink-4 hover:text-ink"
          >
            <FolderPlus className="size-4" /> Create a project
          </Link>
        )}
      </nav>
      <div className="flex flex-col gap-0.5 border-t border-line px-3 pt-3 pb-[max(0.75rem,env(safe-area-inset-bottom))]">
        {isAdmin && (
          <NavLink to="/admin" icon={<Shield />} active={path === "/admin"} onClick={onNavigate}>
            Admin
          </NavLink>
        )}
        <NavLink to="/settings" icon={<Settings />} active={path === "/settings"} onClick={onNavigate}>
          Settings
        </NavLink>
        {me && (
          <Menu
            align="start"
            trigger={(toggle) => (
              <button
                type="button"
                onClick={toggle}
                className="mt-1 flex w-full items-center gap-2.5 rounded-md px-2 py-1.5 text-left transition-colors hover:bg-raised/60"
              >
                <Avatar user={me.user} size={24} />
                <span className="min-w-0 flex-1">
                  <span className="block truncate text-[13px] text-ink">{me.user.name}</span>
                  <span className="block truncate text-[11.5px] text-ink-4">{instance?.name ?? "fctl"}</span>
                </span>
              </button>
            )}
          >
            {(close) => (
              <>
                <MenuItem
                  icon={theme === "dark" ? <Sun /> : <Moon />}
                  onClick={() => {
                    setTheme(theme === "dark" ? "light" : "dark");
                    close();
                  }}
                >
                  {theme === "dark" ? "Light theme" : "Dark theme"}
                </MenuItem>
                <MenuItem
                  icon={<Cpu />}
                  onClick={() => {
                    close();
                    void navigate({ to: "/settings", search: { linked: undefined, error: undefined } });
                  }}
                >
                  Claude and tokens
                </MenuItem>
                <MenuItem
                  icon={<LogOut />}
                  onClick={async () => {
                    close();
                    await api.logout().catch(() => {});
                    queryClient.clear();
                    window.location.assign("/login");
                  }}
                >
                  Sign out
                </MenuItem>
              </>
            )}
          </Menu>
        )}
      </div>
    </div>
  );
}

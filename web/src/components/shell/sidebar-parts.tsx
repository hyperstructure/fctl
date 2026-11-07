// The sidebar's parts: the search that opens ⌘K, a session's line, a count, the rows at its foot,
// the person's menu, and the handle on its edge that resizes, folds and unfolds it.

import { Link, useNavigate } from "@tanstack/react-router";
import {
  ChevronsUpDown,
  Cpu,
  Keyboard,
  LogOut,
  MessageCircleQuestion,
  Moon,
  Search,
  Settings,
  Shield,
  Sun,
} from "lucide-react";
import { type ReactNode, useEffect, useRef, useState } from "react";
import { PullRequestChecksIcon, PullRequestStateIcon } from "@/components/session/pull-request";
import { Avatar, Kbd, Menu, MenuItem, MenuSeparator, StatusDot, statusLabel } from "@/components/ui";
import type * as G from "@/gen";
import { api } from "@/lib/api";
import { clearAttention } from "@/lib/attention";
import { cn } from "@/lib/cn";
import { forgetDraft } from "@/lib/draft";
import { pullRequests } from "@/lib/pulls";
import { forgetPush } from "@/lib/push";
import { queryClient } from "@/lib/queries";
import { shortcutText } from "@/lib/shortcuts";
import { forgetSidebar } from "@/lib/sidebar";
import { draggedTo, RAIL_WIDTH, SIDEBAR_WIDTH } from "@/lib/sidebar-layout";
import { useTheme } from "@/lib/theme";

/** The time, again every minute: a session leaves the sidebar an hour after it ended. */
export function useMinute(): number {
  const [now, setNow] = useState(Date.now);
  useEffect(() => {
    const t = setInterval(() => setNow(Date.now()), 60_000);
    return () => clearInterval(t);
  }, []);
  return now;
}

/** The search field that opens the command palette. */
export function SearchButton({ onPalette }: { onPalette: () => void }) {
  return (
    <button
      type="button"
      onClick={onPalette}
      className="flex h-8.5 w-full items-center gap-2.5 rounded-lg border border-line bg-sunken/60 px-2.5 text-[13px] text-ink-4 transition-colors hover:border-line-strong hover:text-ink-3"
    >
      <Search className="size-3.5" />
      <span className="flex-1 text-left">Search or jump to…</span>
      <Kbd>{shortcutText("palette")}</Kbd>
    </button>
  );
}

/** How many of something, beside a row; what it counts on hover. */
export function Count({ n, label }: { n: number; label: string }) {
  if (n <= 0) return null;
  return (
    <span title={label} className="text-[11.5px] text-accent tabular-nums">
      {n}
    </span>
  );
}

/**
 * A session under its project: its status, its title, and what it waits on: its question, or its
 * pull request's state and checks. Those that ended are dimmer, the one whose page shows marked.
 */
export function SessionLine({
  s,
  here,
  lifted,
  onNavigate,
}: {
  s: G.Session;
  here: boolean;
  /** Its project is being dragged, with it. */
  lifted?: boolean;
  onNavigate: () => void;
}) {
  const ended = s.status === "stopped" || s.status === "failed";
  const [pr] = pullRequests(s);
  return (
    <Link
      to="/s/$id"
      params={{ id: s.id }}
      onClick={onNavigate}
      title={s.question ? `${s.title}\nAsks: ${s.question.text}` : `${s.title} · ${statusLabel(s.status)}`}
      className={cn(
        "flex h-7 items-center gap-2 rounded-md px-2 text-[13px] transition-colors duration-150",
        here
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : s.question
            ? "text-ink-2"
            : ended
              ? "text-ink-4"
              : "text-ink-3",
        !here && !lifted && "hover:bg-hover hover:text-ink",
      )}
    >
      <StatusDot status={s.status} className="scale-75" />
      <span className="min-w-0 flex-1 truncate">{s.title}</span>
      <span className="sr-only">: {s.question ? "asks you something" : statusLabel(s.status)}</span>
      {s.question ? (
        <MessageCircleQuestion className="size-3.5 shrink-0 text-blue" aria-hidden="true" />
      ) : (
        pr && (
          <span className="flex shrink-0 items-center gap-0.5" aria-hidden="true">
            <PullRequestStateIcon pr={pr} className="size-3" />
            <PullRequestChecksIcon pr={pr} className="size-3" />
          </span>
        )
      )}
    </Link>
  );
}

/** Admin (for instance admins, with who waits for early access) and Settings. */
/** What the Admin link's count says. */
export const waitingForAccess = (n: number) =>
  n === 1 ? "1 person waits for early access" : `${n} people wait for early access`;

export function SettingsRows({
  isAdmin,
  asking,
  path,
  onNavigate,
}: {
  isAdmin: boolean;
  asking: number;
  path: string;
  onNavigate: () => void;
}) {
  return (
    <>
      {isAdmin && (
        <FootRow to="/admin" icon={<Shield />} active={path === "/admin"} onClick={onNavigate}>
          Admin
          {asking > 0 && (
            <span
              title={waitingForAccess(asking)}
              className="ml-auto inline-flex h-4 min-w-4 items-center justify-center rounded-full bg-highlight px-1 text-[10.5px] font-semibold text-highlight-ink tabular-nums"
            >
              {asking}
              <span className="sr-only">: {waitingForAccess(asking)}</span>
            </span>
          )}
        </FootRow>
      )}
      <FootRow to="/settings" icon={<Settings />} active={path === "/settings"} onClick={onNavigate}>
        Settings
      </FootRow>
    </>
  );
}

function FootRow({
  to,
  icon,
  active,
  onClick,
  children,
}: {
  to: string;
  icon: ReactNode;
  active: boolean;
  onClick: () => void;
  children: ReactNode;
}) {
  return (
    <Link
      to={to}
      onClick={onClick}
      className={cn(
        "group flex h-8 items-center gap-2.5 rounded-md px-2.5 text-[13.5px] transition-colors duration-150",
        active
          ? "bg-raised text-ink shadow-[inset_0_0_0_1px_var(--c-line)]"
          : "text-ink-3 hover:bg-hover hover:text-ink",
      )}
    >
      <span className={cn("[&_svg]:size-4", active ? "text-accent" : "text-ink-4 group-hover:text-ink-3")}>
        {icon}
      </span>
      {children}
    </Link>
  );
}

/**
 * The person signed in, and their menu: the theme, Claude connections, the keyboard shortcuts (on a
 * computer), signing out.
 */
export function UserMenu({
  me,
  instance,
  onNavigate,
  onShortcuts,
  compact,
}: {
  me?: G.Me;
  instance?: G.InstanceInfo;
  onNavigate: () => void;
  /** Keyboard shortcuts: the sheet that lists them. */
  onShortcuts?: () => void;
  /** Their picture alone, as the rail shows it. */
  compact?: boolean;
}) {
  const [theme, setTheme] = useTheme();
  const navigate = useNavigate();
  if (!me) return null;
  return (
    // At the bottom of the window: the menu opens upward.
    <Menu
      side="top"
      align={compact ? "start" : "stretch"}
      trigger={(toggle, open) => (
        <button
          type="button"
          onClick={toggle}
          aria-haspopup="menu"
          aria-expanded={open}
          aria-label={compact ? me.user.name : undefined}
          title={compact ? me.user.name : undefined}
          className={cn(
            "group flex items-center gap-2.5 text-left transition-colors hover:bg-hover aria-expanded:bg-hover",
            compact ? "size-10 justify-center rounded-xl" : "w-full rounded-md px-2 py-1.5",
          )}
        >
          <Avatar user={me.user} size={compact ? 28 : 24} />
          {!compact && (
            <>
              <span className="min-w-0 flex-1">
                <span className="block truncate text-[13px] text-ink">{me.user.name}</span>
                <span className="block truncate text-[11.5px] text-ink-4">{instance?.name ?? "fctl"}</span>
              </span>
              <ChevronsUpDown className="size-3.5 shrink-0 text-ink-4 transition-colors group-hover:text-ink-3 group-aria-expanded:text-ink-3" />
            </>
          )}
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
              onNavigate();
              void navigate({ to: "/settings", search: { tab: "claude" } });
            }}
          >
            Claude connections
          </MenuItem>
          {onShortcuts && (
            // A computer's: phones and tablets have no keys for them.
            <MenuItem
              icon={<Keyboard />}
              onClick={() => {
                close();
                onShortcuts();
              }}
              className="[@media(hover:none)]:hidden"
            >
              Keyboard shortcuts
              <span className="ml-auto pl-3">
                <Kbd>{shortcutText("help")}</Kbd>
              </span>
            </MenuItem>
          )}
          <MenuSeparator />
          <MenuItem
            icon={<LogOut />}
            onClick={async () => {
              close();
              await forgetPush();
              await forgetDraft();
              await api.logout().catch(() => {});
              queryClient.clear();
              clearAttention();
              forgetSidebar(); // last, so that no list that comes is kept again
              window.location.assign("/login");
            }}
          >
            Sign out
          </MenuItem>
        </>
      )}
    </Menu>
  );
}

/**
 * The handle on the sidebar's edge: drag it, or use the arrow keys, to make the sidebar wider or
 * narrower, or to fold it into its rail and out again (Enter does either); a double click puts it
 * back as it was at first.
 */
export function ResizeHandle({
  width,
  folded,
  onResize,
  onFold,
  onFollow,
}: {
  width: number;
  /** It is folded into its rail. */
  folded: boolean;
  /** A new width, or none for the initial one. */
  onResize: (w: number | null) => void;
  onFold: (folded: boolean) => void;
  /** Whether it follows the pointer at once, without its animation: not as it folds or unfolds. */
  onFollow: (on: boolean) => void;
}) {
  // Where the drag started, and how wide the sidebar was then: folding puts that width back.
  const start = useRef<{ x: number; from: number; width: number } | null>(null);
  const end = () => {
    start.current = null;
    onFollow(false);
  };
  return (
    // biome-ignore lint/a11y/useSemanticElements: a separator that moves is a focusable widget, not an <hr>.
    <div
      role="separator"
      aria-orientation="vertical"
      aria-label="Sidebar width"
      aria-valuenow={folded ? RAIL_WIDTH : width}
      aria-valuetext={folded ? "Folded" : undefined}
      aria-valuemin={RAIL_WIDTH}
      aria-valuemax={SIDEBAR_WIDTH.max}
      tabIndex={0}
      onPointerDown={(e) => {
        e.currentTarget.setPointerCapture(e.pointerId);
        start.current = { x: e.clientX, from: folded ? RAIL_WIDTH : width, width };
      }}
      onPointerMove={(e) => {
        const s = start.current;
        if (!s) return;
        const to = draggedTo(s.from + e.clientX - s.x);
        if (to === "folded") {
          if (folded) return;
          onFollow(false);
          if (width !== s.width) onResize(s.width);
          onFold(true);
        } else {
          // Out of the rail it slides to where the pointer is, then follows it.
          onFollow(!folded);
          onResize(to);
          if (folded) onFold(false);
        }
      }}
      onPointerUp={end}
      onLostPointerCapture={end}
      onDoubleClick={() => {
        onResize(null);
        if (folded) onFold(false);
      }}
      onKeyDown={(e) => {
        if (e.key === "Enter") onFold(!folded);
        else if (e.key === "ArrowRight") {
          if (folded) onFold(false);
          else onResize(width + 16);
        } else if (e.key === "ArrowLeft" && !folded) {
          if (width <= SIDEBAR_WIDTH.min) onFold(true);
          else onResize(width - 16);
        } else return;
        e.preventDefault();
      }}
      className="group/handle absolute inset-y-0 -right-1.5 z-10 hidden w-3 cursor-col-resize touch-none outline-none md:block"
    >
      <span className="absolute inset-y-0 left-1/2 w-px -translate-x-1/2 transition-colors duration-150 group-hover/handle:bg-line-strong group-focus-visible/handle:bg-accent group-active/handle:bg-accent" />
    </div>
  );
}

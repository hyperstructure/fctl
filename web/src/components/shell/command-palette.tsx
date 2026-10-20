// ⌘K: jump to workspaces, projects and sessions, or run an action. Home comes
// first, so that ⌘K and Enter open the task box for a new session from anywhere.

import { useNavigate } from "@tanstack/react-router";
import {
  FolderGit2,
  FolderPlus,
  MessageSquareText,
  Moon,
  Plus,
  Search,
  Settings,
  Shield,
} from "lucide-react";
import { type ReactNode, useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { HomeIcon, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { useMe, useProjects, useSessions, useWorkspaces } from "@/lib/queries";
import { repoLabel } from "@/lib/repos";
import { focusTaskBox } from "@/lib/task-box";
import { getTheme, setTheme } from "@/lib/theme";

interface Entry {
  id: string;
  label: string;
  hint?: string;
  icon: ReactNode;
  run: () => void;
  group: string;
}

export function CommandPalette({ open, onClose }: { open: boolean; onClose: () => void }) {
  const [q, setQ] = useState("");
  const [i, setI] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  const { data: me } = useMe();
  const { data: projects } = useProjects();
  const { data: sessions } = useSessions({ limit: 50 });
  const { data: workspaces } = useWorkspaces();

  useEffect(() => {
    if (open) {
      setQ("");
      setI(0);
      requestAnimationFrame(() => input.current?.focus());
    }
  }, [open]);

  const entries = useMemo<Entry[]>(() => {
    const go = (fn: () => void) => () => {
      onClose();
      fn();
    };
    const list: Entry[] = [
      {
        id: "home",
        group: "Go to",
        label: "Home",
        hint: "new session",
        icon: <HomeIcon className="size-4" />,
        run: go(() => {
          void navigate({ to: "/" });
          focusTaskBox();
        }),
      },
    ];
    for (const s of sessions?.sessions ?? []) {
      list.push({
        id: `s-${s.id}`,
        group: "Sessions",
        label: s.title,
        hint: `${s.project_slug}#${s.number}`,
        icon: <StatusDot status={s.status as G.SessionStatus} />,
        run: go(() => navigate({ to: "/s/$id", params: { id: s.id } })),
      });
    }
    const several = (workspaces?.workspaces.length ?? 0) > 1;
    for (const p of projects?.projects ?? []) {
      list.push({
        id: `p-${p.id}`,
        group: "Projects",
        label: several && p.workspace ? `${p.workspace.name} / ${p.name}` : p.name,
        hint: repoLabel(p),
        icon: <FolderGit2 className="size-4" />,
        run: go(() => navigate({ to: "/p/$slug", params: { slug: p.slug }, search: { tab: undefined } })),
      });
    }
    for (const w of workspaces?.workspaces ?? []) {
      list.push({
        id: `w-${w.id}`,
        group: "Workspaces",
        label: w.name,
        hint: `${w.projects} ${w.projects === 1 ? "project" : "projects"}`,
        icon: <WorkspaceAvatar workspace={w} size={16} title={false} />,
        run: go(() => navigate({ to: "/w/$slug", params: { slug: w.slug } })),
      });
    }
    list.push(
      {
        id: "a-new",
        group: "Actions",
        label: "New project",
        icon: <FolderPlus className="size-4" />,
        run: go(() => navigate({ to: "/new" })),
      },
      {
        id: "a-new-workspace",
        group: "Actions",
        label: "New workspace",
        icon: <Plus className="size-4" />,
        run: go(() => navigate({ to: "/w/new" })),
      },
      {
        id: "a-settings",
        group: "Actions",
        label: "Settings",
        icon: <Settings className="size-4" />,
        run: go(() => navigate({ to: "/settings", search: { linked: undefined, error: undefined } })),
      },
      {
        id: "a-theme",
        group: "Actions",
        label: "Toggle theme",
        icon: <Moon className="size-4" />,
        run: go(() => setTheme(getTheme() === "dark" ? "light" : "dark")),
      },
    );
    if (me?.user.role === "owner" || me?.user.role === "admin") {
      list.push({
        id: "a-admin",
        group: "Actions",
        label: "Admin",
        icon: <Shield className="size-4" />,
        run: go(() => navigate({ to: "/admin", search: { tab: undefined } })),
      });
    }
    const needle = q.trim().toLowerCase();
    if (!needle) return list.slice(0, 40);
    return list
      .map((e) => ({ e, score: score(`${e.label} ${e.hint ?? ""}`.toLowerCase(), needle) }))
      .filter((x) => x.score > 0)
      .sort((a, b) => b.score - a.score)
      .slice(0, 40)
      .map((x) => x.e);
  }, [q, sessions, projects, workspaces, me, navigate, onClose]);

  if (!open) return null;
  let lastGroup = "";
  // On touch screens Safari scrolls the list in a native scroll view, which loses the
  // swipe when the layers above it are rebuilt (WebKit bug 322849: a filter changing on
  // an ancestor). Above it were the card's rise, which animates a filter and then keeps
  // one, and the layers of two blurs and of the card's clip. So on touch screens the
  // palette fades in whole, without blurs (a darker dim stands in for the backdrop's), in
  // an opaque card that doesn't clip: the only layer above the list is the fixed backdrop,
  // which stays as it is while it's open.
  return createPortal(
    // biome-ignore lint/a11y/noStaticElementInteractions: clicking outside closes; Escape closes from the keyboard
    <div
      role="presentation"
      className="fixed inset-0 z-50 flex items-start justify-center bg-black/40 px-4 pt-[14vh] backdrop-blur-[3px] pointer-coarse:animate-fade pointer-coarse:bg-black/55 pointer-coarse:backdrop-filter-none"
      onMouseDown={(e) => e.target === e.currentTarget && onClose()}
    >
      <div className="w-full max-w-xl animate-rise overflow-hidden rounded-xl border border-line-strong bg-surface/95 shadow-[0_30px_120px_rgb(0_0_0/0.55)] backdrop-blur-2xl pointer-coarse:animate-none pointer-coarse:overflow-visible pointer-coarse:bg-surface pointer-coarse:backdrop-filter-none">
        <div className="flex items-center gap-3 border-b border-line px-4">
          <Search className="size-4 text-ink-4" />
          {/* On touch screens the field is drawn in a layer of its own. Safari on iPhones and
              iPads puts the views of the field's caret in the layer that draws the field, and
              while that layer also holds the list, as the palette's does, a swipe over the list
              scrolls the page instead (as in WebKit bug 283804). */}
          <input
            ref={input}
            value={q}
            onChange={(e) => {
              setQ(e.target.value);
              setI(0);
            }}
            onKeyDown={(e) => {
              if (e.key === "Escape") onClose();
              if (e.key === "ArrowDown") {
                e.preventDefault();
                setI((x) => Math.min(entries.length - 1, x + 1));
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setI((x) => Math.max(0, x - 1));
              }
              if (e.key === "Enter") {
                // Or the key would go on to the field that has the focus next, like the task box.
                e.preventDefault();
                entries[i]?.run();
              }
            }}
            placeholder="Search sessions, projects, workspaces and actions"
            className="h-13 flex-1 bg-transparent text-[15px] text-ink outline-none placeholder:text-ink-4 pointer-coarse:will-change-transform"
          />
        </div>
        {/* A swipe past either end stays in the list rather than scrolling the page. Its
            own stacking context (isolate) keeps the layers of the pulsing status dots
            inside the list, where they scroll with it, rather than beside it. */}
        <div className="isolate max-h-[52vh] overflow-y-auto overscroll-contain p-1.5">
          {entries.length === 0 && (
            <div className="px-3 py-8 text-center text-[13px] text-ink-4">Nothing matches “{q}”.</div>
          )}
          {entries.map((e, n) => {
            const header = e.group !== lastGroup ? e.group : null;
            lastGroup = e.group;
            return (
              <div key={e.id}>
                {header && (
                  <div className="px-3 pt-2.5 pb-1 text-[12px] font-medium text-ink-4">{header}</div>
                )}
                <button
                  type="button"
                  onMouseMove={() => setI(n)}
                  onClick={e.run}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-md px-3 py-2 text-left text-[13.5px] transition-colors",
                    n === i ? "bg-hover text-ink" : "text-ink-2",
                  )}
                >
                  <span className="flex size-4 items-center justify-center text-ink-4">{e.icon}</span>
                  <span className="min-w-0 flex-1 truncate">{e.label}</span>
                  {e.hint && <span className="shrink-0 font-mono text-[11.5px] text-ink-4">{e.hint}</span>}
                </button>
              </div>
            );
          })}
        </div>
        <div className="flex items-center gap-3 border-t border-line px-4 py-2 text-[11.5px] text-ink-4">
          <MessageSquareText className="size-3.5" /> Sessions, projects and actions · ↑↓ to move · ↵ to open
        </div>
      </div>
    </div>,
    document.body,
  );
}

/** Subsequence match, rewarding prefixes and word starts. */
function score(hay: string, needle: string): number {
  if (hay.includes(needle)) return 100 - hay.indexOf(needle);
  let s = 0;
  let pos = 0;
  for (const ch of needle) {
    const at = hay.indexOf(ch, pos);
    if (at === -1) return 0;
    s += at === pos ? 3 : 1;
    pos = at + 1;
  }
  return s;
}

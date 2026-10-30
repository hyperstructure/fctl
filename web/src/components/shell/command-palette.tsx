// ⌘K: jump to workspaces, projects and sessions, or run an action. The Agent Floor
// comes first, so that ⌘K and Enter open the task box for a new session from anywhere.
// Past a couple of letters it also searches what all sessions said (lib/search.ts):
// those open at what it found, with the session's find bar on it.

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
import { AgentFloorIcon, Spinner, StatusDot } from "@/components/ui";
import { WorkspaceAvatar } from "@/components/workspace/avatar";
import type * as G from "@/gen";
import { cn } from "@/lib/cn";
import { useHotkey } from "@/lib/hotkeys";
import { useMe, useProjects, useSessions, useWorkspaces } from "@/lib/queries";
import { repoLabel } from "@/lib/repos";
import { around, landing, marked, said, useSessionSearch } from "@/lib/search";
import { focusTaskBox } from "@/lib/task-box";
import { getTheme, setTheme } from "@/lib/theme";
import { createsProjects } from "@/lib/workspaces";

interface Entry {
  id: string;
  label: string;
  /** The label as it shows, where it differs: with what a search found in it marked. */
  title?: ReactNode;
  /** A second line: where a search found what was typed. */
  detail?: ReactNode;
  hint?: string;
  /** More words it is found by. */
  keywords?: string;
  icon: ReactNode;
  run: () => void;
  group: string;
}

/** What a search found, marked in its text. */
function Marked({ m }: { m: G.SearchMatch }) {
  return marked(m.text, m.ranges).map((p, i) =>
    p.hit ? (
      <mark
        key={i}
        className="rounded-[3px] bg-highlight/25 text-ink [:root[data-theme=light]_&]:bg-highlight"
      >
        {p.text}
      </mark>
    ) : (
      p.text
    ),
  );
}

/**
 * Opens the palette on ⌘K, and on ⌘P and ⌘⇧P as quick open and the command palette do in
 * code editors (⌘P in place of printing), even while typing. The search button shows ⌘K alone.
 */
export function usePaletteKeys(open: () => void) {
  useHotkey("mod+k", open, { inInputs: true });
  useHotkey("mod+p", open, { inInputs: true });
  useHotkey("mod+shift+p", open, { inInputs: true });
}

export function CommandPalette({
  open,
  onClose,
  onNavigate,
}: {
  open: boolean;
  onClose: () => void;
  /** An entry opens a page: on phones, the sidebar the palette was opened from goes too. */
  onNavigate: () => void;
}) {
  const [q, setQ] = useState("");
  const [i, setI] = useState(0);
  const input = useRef<HTMLInputElement>(null);
  const navigate = useNavigate();
  const { data: me } = useMe();
  const { data: projects } = useProjects();
  const { data: sessions } = useSessions({ limit: 50 });
  const { data: workspaces } = useWorkspaces();
  const found = useSessionSearch(q, { enabled: open });

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
      onNavigate();
      fn();
    };
    const list: Entry[] = [
      {
        id: "home",
        group: "Go to",
        label: "Agent Floor",
        hint: "new session",
        // What it was called before.
        keywords: "home",
        icon: <AgentFloorIcon className="size-4" />,
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
    // Only where you create projects: in workspaces you are an admin of.
    if (workspaces?.workspaces.some(createsProjects)) {
      list.push({
        id: "a-new",
        group: "Actions",
        label: "New project",
        icon: <FolderPlus className="size-4" />,
        run: go(() => navigate({ to: "/new" })),
      });
    }
    list.push(
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
        // It opens no page: the sidebar the palette was opened from stays, as the sidebar's
        // own theme item leaves it.
        run: () => {
          onClose();
          setTheme(getTheme() === "dark" ? "light" : "dark");
        },
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
      .map((e) => ({
        e,
        score: score(`${e.label} ${e.hint ?? ""} ${e.keywords ?? ""}`.toLowerCase(), needle),
      }))
      .filter((x) => x.score > 0)
      .sort((a, b) => b.score - a.score)
      .slice(0, 40)
      .map((x) => x.e);
  }, [q, sessions, projects, workspaces, me, navigate, onClose, onNavigate]);

  // The sessions that say what was typed, past those listed already.
  const deep = useMemo<Entry[]>(() => {
    const listed = new Set(entries.map((e) => e.id));
    return (found.results?.sessions ?? [])
      .filter((hit) => !listed.has(`s-${hit.session.id}`))
      .map((hit) => {
        const s = hit.session;
        const title = hit.matches.find((m) => m.kind === "title");
        const where = hit.matches.find((m) => m.kind !== "title");
        return {
          id: `f-${s.id}`,
          group: "In sessions",
          label: s.title,
          title: title && <Marked m={title} />,
          detail: where && (
            <>
              <span className="text-ink-3">{said(where, me?.user.id)}:</span> <Marked m={around(where, 24)} />
            </>
          ),
          hint: `${s.project_slug}#${s.number}${s.archived ? " · archived" : ""}`,
          icon: <StatusDot status={s.status} />,
          run: () => {
            onClose();
            onNavigate();
            // What found it, which may be what was typed before the last keys.
            void navigate({
              to: "/s/$id",
              params: { id: s.id },
              search: landing(hit, found.results?.for ?? q),
            });
          },
        };
      });
  }, [found.results, entries, me, q, navigate, onClose, onNavigate]);
  const all = deep.length ? entries.concat(deep) : entries;

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
      className="fixed inset-x-0 top-(--viewport-shift) -bottom-(--viewport-shift) z-50 flex items-start justify-center bg-black/40 px-4 pt-[14vh] backdrop-blur-[3px] pointer-coarse:animate-fade pointer-coarse:bg-black/55 pointer-coarse:backdrop-filter-none"
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
                setI((x) => Math.min(all.length - 1, x + 1));
              }
              if (e.key === "ArrowUp") {
                e.preventDefault();
                setI((x) => Math.max(0, x - 1));
              }
              if (e.key === "Enter") {
                // Or the key would go on to the field that has the focus next, like the task box.
                e.preventDefault();
                all[i]?.run();
              }
            }}
            placeholder="Search sessions, projects, workspaces and actions"
            aria-label="Search sessions, what they say, projects, workspaces and actions"
            className="h-13 flex-1 bg-transparent text-[15px] text-ink outline-none placeholder:text-ink-4 pointer-coarse:will-change-transform"
          />
        </div>
        {/* A swipe past either end stays in the list rather than scrolling the page. Its
            own stacking context (isolate) keeps the layers of the pulsing status dots
            inside the list, where they scroll with it, rather than beside it. */}
        <div className="isolate max-h-[52vh] overflow-y-auto overscroll-contain p-1.5">
          {all.length === 0 && !found.searching && (
            <div className="px-3 py-8 text-center text-[13px] text-ink-4">Nothing matches “{q}”.</div>
          )}
          {all.map((e, n) => {
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
                    e.detail && "items-start",
                    n === i ? "bg-hover text-ink" : "text-ink-2",
                  )}
                >
                  <span
                    className={cn("flex size-4 items-center justify-center text-ink-4", e.detail && "h-5")}
                  >
                    {e.icon}
                  </span>
                  <span className="min-w-0 flex-1">
                    <span className="block truncate">{e.title ?? e.label}</span>
                    {e.detail && (
                      <span className="mt-0.5 block truncate text-[12.5px] text-ink-3">{e.detail}</span>
                    )}
                  </span>
                  {e.hint && (
                    <span
                      className={cn("shrink-0 font-mono text-[11.5px] text-ink-4", e.detail && "leading-5")}
                    >
                      {e.hint}
                    </span>
                  )}
                </button>
              </div>
            );
          })}
          {found.searching && !deep.length && (
            <div className="flex items-center gap-3 px-3 py-2 text-[12.5px] text-ink-4" aria-live="polite">
              <Spinner className="size-3.5" /> Looking in what sessions said…
            </div>
          )}
        </div>
        <div className="flex items-center gap-3 border-t border-line px-4 py-2 text-[11.5px] text-ink-4">
          <MessageSquareText className="size-3.5" /> Finds what sessions said too · ↑↓ to move · ↵ to open
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

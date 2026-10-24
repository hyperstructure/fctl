// Which GitHub Apps' pull requests start reviews by themselves in a project, in its review agents'
// settings: the apps on the list, each with its logo and what it opened in the project's
// repositories lately, and a field that finds more among the apps that opened pull requests there,
// the apps many repositories use, and any app GitHub shows by its name.

import { useQuery } from "@tanstack/react-query";
import { Bot, Plus, Search, X } from "lucide-react";
import { type KeyboardEvent, type ReactNode, useEffect, useId, useRef, useState } from "react";
import { Button, IconButton } from "@/components/ui";
import type * as G from "@/gen";
import { ApiFailure, api } from "@/lib/api";
import { cn } from "@/lib/cn";
import { ago } from "@/lib/format";
import { keys } from "@/lib/queries";
import {
  type BotView,
  botLogin,
  botSlug,
  botView,
  KNOWN_BOTS,
  knownBot,
  matchBots,
  meant,
  sized,
} from "@/lib/review-bots";

/** How many of the apps that opened pull requests lately show before anyone types. */
const SEEN_SHOWN = 4;
/** How many apps that match what someone typed show. */
const MATCHES_SHOWN = 6;
/** How long typing pauses before GitHub is asked about the app typed. */
const SETTLE_MS = 300;

/** `value`, once it has stayed the same for `ms`. */
function useSettled<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

/** An app as GitHub shows it, for one the Reviews tab doesn't know; kept for the page's life. */
function useFound(project: string, login: string | null, enabled: boolean) {
  return useQuery({
    queryKey: keys.reviewBot(project, login ?? ""),
    queryFn: () => api.reviewBot(project, login ?? ""),
    enabled: enabled && !!login,
    staleTime: Number.POSITIVE_INFINITY,
    retry: false,
  });
}

/** "3 pull requests here lately, the newest 2 days ago". */
function activity(b: G.ReviewBot): string {
  const n = b.pull_requests === 1 ? "1 pull request" : `${b.pull_requests} pull requests`;
  return b.latest_at ? `${n} here lately, the newest ${ago(b.latest_at)}` : `${n} here lately`;
}

/** What else a row says of an app: what it's for, who makes it, what it did here. */
function facts(v: BotView): string {
  return [v.about ?? (v.owner && `By ${v.owner}`), v.seen && activity(v.seen)].filter(Boolean).join(" · ");
}

/** The newest pull request it opened here, for a tooltip: "chore(deps): update vite (web#318)". */
function newest(v: BotView): string | undefined {
  const pr = v.seen?.latest;
  if (!pr) return undefined;
  const at = `${pr.repo?.name ?? ""}#${pr.number}`;
  return pr.title ? `The newest: ${pr.title} (${at})` : `The newest: ${at}`;
}

/**
 * The apps whose pull requests start reviews by themselves (`value`, as the form has it; `saved`, as
 * the project does), which the project's owners change; the form saves them.
 */
export function ReviewBots({
  project,
  value,
  saved,
  onChange,
  error,
}: {
  project: G.Project;
  value: readonly string[];
  saved: readonly string[];
  onChange: (bots: string[]) => void;
  error?: string;
}) {
  const id = useId();
  const [query, setQuery] = useState("");
  const [active, setActive] = useState(0);
  // The active option shows while the field has the keys, or the pointer is on the list.
  const [focused, setFocused] = useState(false);
  const [pointing, setPointing] = useState(false);
  const [said, setSaid] = useState("");
  const input = useRef<HTMLInputElement>(null);
  const seen = useQuery({
    queryKey: keys.reviewBots(project.id),
    queryFn: () => api.reviewBots(project.id),
    staleTime: 5 * 60_000,
    retry: false,
  });
  const seenBots = seen.data?.bots ?? [];
  const seenOf = (login: string) => seenBots.find((b) => b.login === login);
  const on = new Set(value);

  // The list: the saved apps where they were, those taken off it included until the form saves,
  // then the ones added.
  const rows = [
    ...saved.map((login) => ({ login, state: on.has(login) ? ("kept" as const) : ("removed" as const) })),
    ...value.filter((l) => !saved.includes(l)).map((login) => ({ login, state: "added" as const })),
  ];

  // Apps to add: those that opened pull requests here lately, then those many repositories use.
  const candidates: BotView[] = [
    ...seenBots.map((b) => botView(b.login, b)),
    ...KNOWN_BOTS.filter((k) => !seenOf(k.login)).map((k) => botView(k.login)),
  ];
  const q = query.trim();
  const typed = q ? botLogin(q) : null;
  const settled = useSettled(typed, SETTLE_MS);
  const isCandidate = !!typed && candidates.some((c) => c.login === typed);
  const lookUp = !!typed && !isCandidate && !on.has(typed);
  const found = useFound(project.id, settled, lookUp && settled === typed);
  const missing = found.error instanceof ApiFailure && found.error.code === "not_found";

  type Option = { view: BotView; note?: ReactNode; already?: boolean };
  const options: Option[] = q
    ? matchBots(q, candidates)
        .slice(0, MATCHES_SHOWN)
        .map((view) => ({ view, already: on.has(view.login) }))
    : candidates
        .filter((c) => c.seen && !on.has(c.login) && !saved.includes(c.login))
        .slice(0, SEEN_SHOWN)
        .map((view) => ({ view }));
  // What GitHub says of the app typed, where nothing above is it: its page's name and logo, or
  // that it shows none by that name, as for other accounts' private apps, which may still be added.
  let pending = false;
  if (typed && lookUp) {
    if (settled === typed && found.data) {
      options.push({ view: botView(typed, undefined, found.data) });
    } else if (settled !== typed || found.isFetching) {
      pending = options.length === 0;
    } else if (options.length === 0 || !missing) {
      const guess = missing
        ? meant(
            typed,
            candidates.map((c) => c.login),
          )
        : undefined;
      if (guess) {
        const view = candidates.find((c) => c.login === guess);
        if (view) options.push({ view, note: "Did you mean this one?", already: on.has(view.login) });
      }
      options.push({
        view: botView(typed),
        note: missing
          ? "GitHub shows no public app by this name. Add it if it's a private app of yours."
          : `fctl couldn't ask GitHub about it: ${found.error?.message ?? "it didn't answer"}`,
      });
    }
  } else if (typed && on.has(typed) && !options.some((o) => o.view.login === typed)) {
    options.unshift({ view: botView(typed, seenOf(typed)), already: true });
  }
  const pickable = options.map((o, i) => (o.already ? -1 : i)).filter((i) => i >= 0);
  const current = pickable.includes(active) ? active : (pickable[0] ?? -1);
  const listId = `${id}-apps`;
  const optionId = (i: number) => `${id}-app-${i}`;

  const add = (view: BotView) => {
    if (on.has(view.login)) return;
    onChange([...value, view.login]);
    setQuery("");
    setActive(0);
    setSaid(`${view.name} added. Save to let its pull requests start reviews.`);
    input.current?.focus();
  };
  const remove = (view: BotView) => {
    onChange(value.filter((l) => l !== view.login));
    setSaid(`${view.name} taken off the list. Save to keep it off.`);
  };
  const undo = (view: BotView) => {
    onChange([...value, view.login]);
    setSaid(`${view.name} is back on the list.`);
  };
  const onKeyDown = (e: KeyboardEvent<HTMLInputElement>) => {
    if (e.key === "ArrowDown" || e.key === "ArrowUp") {
      e.preventDefault();
      if (pickable.length === 0) return;
      const at = pickable.indexOf(current);
      const step = e.key === "ArrowDown" ? 1 : -1;
      setActive(pickable[(at + step + pickable.length) % pickable.length] ?? 0);
    } else if (e.key === "Enter") {
      // It would save the form.
      e.preventDefault();
      const o = options[current];
      if (o && !o.already) add(o.view);
    } else if (e.key === "Escape" && query) {
      e.preventDefault();
      setQuery("");
    }
  };

  return (
    <fieldset aria-describedby={`${id}-hint`} className="flex min-w-0 flex-col gap-1.5">
      <legend className="mb-1.5 text-[12.5px] font-medium text-ink-2">Apps that start reviews</legend>
      <p id={`${id}-hint`} className="text-[12.5px] leading-relaxed text-ink-3">
        Pull requests these GitHub Apps open from the repository's branches get a review without anyone
        asking. What they say reaches the agent before anyone here reads it, and it runs none of their code.
      </p>
      <ul
        aria-label="Apps that start reviews"
        className="mt-1.5 divide-y divide-line overflow-hidden rounded-lg border border-line"
      >
        {rows.length === 0 && (
          <li className="px-3.5 py-3 text-[13px] text-ink-4">
            None: every app's pull requests wait until someone asks for their review.
          </li>
        )}
        {rows.map((r) => (
          <ListedBot
            key={r.login}
            project={project.id}
            login={r.login}
            state={r.state}
            seen={seenOf(r.login)}
            lookUp={!seen.isPending}
            onRemove={remove}
            onUndo={undo}
          />
        ))}
      </ul>
      <div className="relative mt-1.5">
        <Search
          aria-hidden="true"
          className="pointer-events-none absolute top-1/2 left-3 size-3.5 -translate-y-1/2 text-ink-4"
        />
        <input
          ref={input}
          id={`${id}-input`}
          type="text"
          role="combobox"
          aria-label="Add an app"
          aria-expanded={options.length > 0}
          aria-controls={listId}
          aria-autocomplete="list"
          aria-activedescendant={current >= 0 && options.length > 0 ? optionId(current) : undefined}
          aria-invalid={q && !typed && options.length === 0 ? true : undefined}
          value={query}
          onChange={(e) => {
            setQuery(e.target.value);
            setActive(0);
          }}
          onKeyDown={onKeyDown}
          onFocus={() => setFocused(true)}
          onBlur={() => setFocused(false)}
          placeholder="Add an app by its name or its page on GitHub"
          autoComplete="off"
          autoCapitalize="off"
          autoCorrect="off"
          spellCheck={false}
          className={cn(
            "h-9.5 w-full rounded-md border border-line-strong bg-sunken/60 pr-3 pl-8.5 text-[14px] text-ink placeholder:text-ink-4",
            "transition-[border-color,box-shadow,background-color] duration-150 outline-none",
            "hover:border-ink-4/60 focus:border-accent/70 focus:bg-sunken focus:shadow-[0_0_0_3px_var(--c-accent-soft)]",
            "aria-invalid:border-danger/60",
          )}
        />
      </div>
      {q && !typed && options.length === 0 ? (
        <p className="text-[12.5px] text-ink-3">
          No app here goes by that. Name one as it's named on GitHub, such as renovate or my-app[bot], or
          paste its page, github.com/apps/….
        </p>
      ) : pending ? (
        <p className="flex items-center gap-2 px-1 text-[12.5px] text-ink-3">Asking GitHub about {typed}…</p>
      ) : !q && seen.isError ? (
        <p className="text-[12.5px] text-ink-4 [overflow-wrap:anywhere]">
          fctl couldn't ask GitHub which apps opened pull requests here lately: {seen.error.message}
        </p>
      ) : null}
      {options.length > 0 && (
        <div className="animate-rise rounded-lg border border-line bg-sunken/40 p-1 [animation-duration:240ms]">
          {!q && (
            <p className="px-2.5 pt-1.5 pb-1 text-[11.5px] font-medium text-ink-3">
              Opened pull requests here lately
            </p>
          )}
          <div id={listId} role="listbox" aria-label="Apps to add" onPointerLeave={() => setPointing(false)}>
            {options.map((o, i) => {
              const lit = i === current && !o.already && (focused || pointing);
              return (
                // biome-ignore lint/a11y/useKeyWithClickEvents: the field takes the keys (aria-activedescendant)
                <div
                  key={`${o.view.login}-${o.note ? "note" : ""}`}
                  id={optionId(i)}
                  role="option"
                  tabIndex={-1}
                  aria-selected={i === current}
                  aria-disabled={o.already || undefined}
                  className={cn(
                    "flex items-center gap-3 rounded-md px-2.5 py-2 transition-colors duration-150",
                    o.already ? "text-ink-4" : "cursor-pointer text-ink-2",
                    lit && "bg-hover text-ink",
                  )}
                  onMouseDown={(e) => e.preventDefault()}
                  onPointerMove={() => {
                    if (o.already) return;
                    setPointing(true);
                    if (i !== current) setActive(i);
                  }}
                  onClick={() => !o.already && add(o.view)}
                >
                  <BotLogo view={o.view} size={26} />
                  <span className="min-w-0 flex-1">
                    <BotName view={o.view} />
                    <span
                      className="block truncate text-[12px] text-ink-4"
                      title={o.note ? undefined : newest(o.view)}
                    >
                      {o.note ?? (o.already ? "On the list already" : facts(o.view) || "A GitHub App")}
                    </span>
                  </span>
                  {!o.already && (
                    <span
                      aria-hidden="true"
                      className={cn(
                        "inline-flex shrink-0 items-center gap-1 rounded-sm px-1.5 py-0.5 text-[12px] font-medium transition-colors duration-150",
                        lit ? "text-ink" : "text-ink-3",
                      )}
                    >
                      <Plus className="size-3.5" />
                      Add
                    </span>
                  )}
                </div>
              );
            })}
          </div>
        </div>
      )}
      {error && (
        <p className="animate-rise text-[12.5px] text-danger [overflow-wrap:anywhere]" role="alert">
          {error}
        </p>
      )}
      <p className="sr-only" aria-live="polite">
        {said}
      </p>
    </fieldset>
  );
}

/** An app on the list: kept, added or taken off it since the project saved it. */
function ListedBot({
  project,
  login,
  state,
  seen,
  lookUp,
  onRemove,
  onUndo,
}: {
  project: string;
  login: string;
  state: "kept" | "added" | "removed";
  seen?: G.ReviewBot;
  /** Whether to ask GitHub for its name and logo, where nothing else says them. */
  lookUp: boolean;
  onRemove: (v: BotView) => void;
  onUndo: (v: BotView) => void;
}) {
  // Apps the Reviews tab doesn't know, nor saw lately, are asked about once: their name and logo.
  const found = useFound(project, login, lookUp && !knownBot(login) && !seen);
  const view = botView(login, seen, found.data);
  const removed = state === "removed";
  return (
    <li
      className={cn(
        "flex items-center gap-3 px-3.5 py-2.5 transition-colors duration-200",
        state === "added" && "animate-rise bg-accent-soft/35 [animation-duration:280ms]",
      )}
    >
      <BotLogo view={view} size={30} className={cn(removed && "opacity-40 grayscale")} />
      <div className="min-w-0 flex-1">
        <BotName view={view} link struck={removed} />
        <p className="truncate text-[12px] text-ink-4" title={removed ? undefined : newest(view)}>
          {removed
            ? "Taken off the list when you save: its pull requests will wait until someone asks"
            : facts(view) || "A GitHub App"}
        </p>
      </div>
      {state === "added" && (
        <span className="shrink-0 rounded-sm bg-highlight px-1.5 py-px text-[11px] font-medium text-highlight-ink">
          New
        </span>
      )}
      {removed ? (
        <Button size="sm" variant="ghost" onClick={() => onUndo(view)}>
          Undo
        </Button>
      ) : (
        <IconButton label={`Take ${view.name} off the list`} type="button" onClick={() => onRemove(view)}>
          <X className="size-4" />
        </IconButton>
      )}
    </li>
  );
}

/** The app's name, and its bot where the name isn't the bot's: "Renovate renovate[bot]". */
function BotName({ view, link, struck }: { view: BotView; link?: boolean; struck?: boolean }) {
  // Apps nobody named go by their bot.
  const named = view.name !== botSlug(view.login);
  const title = named ? view.name : view.login;
  return (
    <span className="flex min-w-0 items-baseline gap-2">
      {link ? (
        <a
          href={view.url}
          target="_blank"
          rel="noreferrer"
          title={`${title} on GitHub`}
          className={cn(
            "truncate text-[13.5px] font-medium text-ink underline-offset-2 hover:underline",
            !named && "font-mono text-[12.5px]",
            struck && "text-ink-4 line-through",
          )}
        >
          {title}
        </a>
      ) : (
        <span className={cn("truncate text-[13.5px] font-medium", !named && "font-mono text-[12.5px]")}>
          {title}
        </span>
      )}
      {named && (
        <span className="shrink-0 font-mono text-[11.5px] text-ink-4 max-sm:hidden">{view.login}</span>
      )}
    </span>
  );
}

/** The app's logo, `size` pixels square; a robot where nobody said where its logo is, or it doesn't load. */
function BotLogo({ view, size, className }: { view: BotView; size: number; className?: string }) {
  const [broken, setBroken] = useState<string>();
  const box = cn(
    "shrink-0 rounded-md border border-line bg-raised transition-[opacity,filter] duration-200",
    className,
  );
  if (!view.logo || broken === view.logo) {
    return (
      <span
        aria-hidden="true"
        className={cn(box, "inline-flex items-center justify-center text-ink-4")}
        style={{ width: size, height: size }}
      >
        <Bot className="size-[58%]" />
      </span>
    );
  }
  return (
    <img
      src={sized(view.logo, size * 2)}
      alt=""
      width={size}
      height={size}
      className={cn(box, "object-cover")}
      referrerPolicy="no-referrer"
      onError={() => setBroken(view.logo)}
    />
  );
}

/** Its logo where one is known, for sentences that name the apps on the list. */
export function BotMark({ login, size = 16 }: { login: string; size?: number }) {
  return <BotLogo view={botView(login)} size={size} className="rounded-[4px]" />;
}

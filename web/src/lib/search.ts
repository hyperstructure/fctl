// Searching all sessions (GET /api/v1/search): when a search goes to the
// control plane, and how a result says where it found what was typed.

import { keepPreviousData, useQuery } from "@tanstack/react-query";
import { useEffect, useState } from "react";
import type * as G from "@/gen";
import { api } from "./api";
import { keys } from "./queries";

/**
 * Whether the control plane searches sessions for `q`: three characters or
 * more, or two in a script that writes words in two (Chinese, Japanese,
 * Korean). Fewer find too much to be of use.
 */
export function searchable(q: string): boolean {
  const t = q.trim().replace(/\s+/g, " ");
  const n = [...t].length;
  return (
    n >= 3 || (n === 2 && /[\p{Script=Han}\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]/u.test(t))
  );
}

/** `value`, once it stopped changing for `ms`. */
export function useSettled<T>(value: T, ms: number): T {
  const [settled, setSettled] = useState(value);
  useEffect(() => {
    const t = setTimeout(() => setSettled(value), ms);
    return () => clearTimeout(t);
  }, [value, ms]);
  return settled;
}

/**
 * The sessions that say `q`, once typing paused, and what they say (`for`):
 * the last results stay while the next ones load, so that `for` can be what
 * was typed before. Nothing for what isn't [`searchable`].
 */
export function useSessionSearch(
  q: string,
  { enabled = true, limit = 8 }: { enabled?: boolean; limit?: number } = {},
) {
  const typed = useSettled(q.trim(), 220);
  const on = enabled && searchable(typed);
  const result = useQuery({
    queryKey: keys.search(typed, limit),
    queryFn: async () => ({ for: typed, ...(await api.search(typed, { limit })) }),
    enabled: on,
    staleTime: 30_000,
    placeholderData: keepPreviousData,
  });
  return {
    results: on ? result.data : undefined,
    // Typing that hasn't settled yet counts as searching.
    searching: enabled && searchable(q) && (q.trim() !== typed || result.isFetching),
    failed: on && result.isError,
  };
}

/** Who said what a search found, as a result line names them: "You", their name, fctl or the agent. */
export function said(m: G.SearchMatch, me?: G.UserId): string {
  switch (m.kind) {
    case "title":
      return "Title";
    case "description":
      return "Description";
    case "reply":
      return "Agent";
    case "message":
      return !m.author ? "fctl" : m.author.id === me ? "You" : m.author.name;
  }
}

/** A snippet in pieces, the matches among them marked. `ranges` are UTF-16 offsets, as the API gives them. */
export function marked(text: string, ranges: [number, number][]): { text: string; hit: boolean }[] {
  const out: { text: string; hit: boolean }[] = [];
  let at = 0;
  for (const [s, e] of [...ranges].sort((a, b) => a[0] - b[0])) {
    if (s < at || e <= s || e > text.length) continue;
    if (s > at) out.push({ text: text.slice(at, s), hit: false });
    out.push({ text: text.slice(s, e), hit: true });
    at = e;
  }
  if (at < text.length) out.push({ text: text.slice(at), hit: false });
  return out;
}

/**
 * A snippet that starts at most `lead` characters before its first match, at
 * a word where there is one, so that a line too narrow for the whole of it
 * still shows the match.
 */
export function around(m: G.SearchMatch, lead: number): G.SearchMatch {
  const first = m.ranges.reduce((a, [s]) => Math.min(a, s), Number.POSITIVE_INFINITY);
  if (!Number.isFinite(first) || first <= lead + 1) return m;
  let cut = first - lead;
  const space = m.text.indexOf(" ", cut);
  if (space >= 0 && space < first) cut = space + 1;
  // Not inside a surrogate pair.
  const code = m.text.charCodeAt(cut);
  if (code >= 0xdc00 && code <= 0xdfff) cut++;
  const shift = cut - 1;
  return {
    ...m,
    text: `…${m.text.slice(cut)}`,
    ranges: m.ranges.filter(([s]) => s >= cut).map(([s, e]) => [s - shift, e - shift]),
  };
}

/** Where a result opens its session: its find bar on what was typed, at the newest message or reply it matched. */
export function landing(hit: G.SessionMatches, q: string): { find: string; at?: number } {
  const at = hit.matches.find((m) => m.seq !== undefined)?.seq;
  return { find: q.trim(), at };
}

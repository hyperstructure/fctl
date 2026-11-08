// What of a session's transcript the reader hasn't seen, as the session page counts it
// (routes/session.tsx): the items that came after a moment, by the events they started
// with. Earlier activity that loads above the reader came before it, and a message on its way
// isn't stored yet: neither counts.

import type { Item } from "./transcript";

/** A message sent from this page and not stored yet: it has no place among the events. */
const onItsWay = (it: Item) => it.kind === "user" && !!it.pending;

/** The newest stored event the transcript has, by its seq; 0 without any. */
export function newestSeq(items: readonly Item[]): number {
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (it && !onItsWay(it)) return it.seq;
  }
  return 0;
}

/**
 * How many of the transcript's items came after the event `seq`. Items keep the seq of the event
 * they started with, so a call whose result came since doesn't count again.
 */
export function cameAfter(items: readonly Item[], seq: number): number {
  let n = 0;
  for (let i = items.length - 1; i >= 0; i--) {
    const it = items[i];
    if (!it || onItsWay(it)) continue;
    if (it.seq <= seq) break;
    n++;
  }
  return n;
}

// A live session: the viewer WebSocket, the transcript, presence, and sends.
//
// One store per open session page. React reads it with useSyncExternalStore;
// every update replaces `state` with a new object (fields keep identity when
// unchanged).

import { useEffect, useMemo, useSyncExternalStore } from "react";
import type * as G from "@/gen";
import { api, wsUrl } from "./api";
import { patchSession, queryClient } from "./queries";
import { parts } from "./runs";
import {
  applyStream,
  emptyLive,
  type Item,
  type LiveTurn,
  settleLive,
  TranscriptBuilder,
} from "./transcript";

export type Connection = "connecting" | "live" | "reconnecting" | "closed";

/** Events in a page of earlier activity, at most. */
const PAGE = 400;
/**
 * About as many parts of the transcript (`parts`: a message, a line of the
 * agent's work, a milestone) as fill the page. A page of events can show
 * next to nothing: the tool calls the agent makes in a row fold into one
 * line, and older agents stored Claude Code's word on how far its thinking
 * got, about once a second.
 */
export const SCREENFUL = 12;
/** Pages of events one load of earlier activity fetches at most, however little they show. */
export const MAX_PAGES = 5;

/** How many parts (`parts`) the transcript of these events shows. */
const shown = (events: G.SessionEvent[]) => parts(new TranscriptBuilder().add(events)).length;

export interface LiveState {
  session?: G.Session;
  role?: G.SessionRole;
  items: Item[];
  live: LiveTurn;
  viewers: G.Presence[];
  connection: Connection;
  runnerConnected: boolean;
  /** While the runner isn't connected: when it was last heard from, and when this page learned that. */
  runnerAway?: { lastSeenAt: number; noticedAt: number };
  /** The connected runner can't reach the agent in the sandbox. */
  agentUnreachable?: G.AgentUnreachable;
  hasMoreBefore: boolean;
  loadingOlder: boolean;
  /** The last load of earlier activity brought nothing, though there is more: a page failed. */
  olderFailed: boolean;
  firstSeq: number;
  lastSeq: number;
  git?: G.GitState;
  model?: string;
  /** Tokens per second over the last few seconds, for the orb. */
  activity: number;
  error?: string;
}

type Listener = () => void;

export class LiveSession {
  readonly id: string;
  state: LiveState;
  private listeners = new Set<Listener>();
  private ws?: WebSocket;
  private builder = new TranscriptBuilder();
  private events: G.SessionEvent[] = [];
  private retry = 0;
  private timer?: ReturnType<typeof setTimeout>;
  private closed = false;
  private counter = 0;
  private lastTyping = 0;
  private pending = new Map<string, { resolve: () => void; reject: (e: Error) => void }>();
  /** Sent messages not stored yet, shown after the transcript (never inside the builder). */
  private outbox: Item[] = [];
  private deltaTimes: number[] = [];
  private ping?: ReturnType<typeof setInterval>;
  /** Whether the session's page is open. */
  private shown = true;
  /** What the server last heard: whether the person can see the session (it assumes so). */
  private seen = true;

  constructor(id: string, initial?: G.Session) {
    this.id = id;
    this.state = {
      session: initial,
      items: [],
      live: emptyLive(),
      viewers: [],
      connection: "connecting",
      runnerConnected: false,
      hasMoreBefore: false,
      loadingOlder: false,
      olderFailed: false,
      firstSeq: 0,
      lastSeq: 0,
      activity: 0,
    };
  }

  subscribe = (fn: Listener) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };

  getSnapshot = () => this.state;

  private set(patch: Partial<LiveState>) {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }

  /** The runner's state, as the snapshot or a `runner_connected` message has it. */
  private runner(connected: boolean, lastSeenAt?: number, agent?: G.AgentUnreachable): Partial<LiveState> {
    const noticedAt = this.state.runnerAway?.noticedAt ?? Date.now();
    return {
      runnerConnected: connected,
      runnerAway: !connected && lastSeenAt ? { lastSeenAt, noticedAt } : undefined,
      agentUnreachable: connected ? agent : undefined,
    };
  }

  /** The transcript plus unsent messages; stored messages retire their optimistic copies. */
  private show(items: Item[], added: G.SessionEvent[] = []): Item[] {
    const ids = (files?: G.Attachment[]) => (files ?? []).map((f) => f.id).join(",");
    for (const e of added) {
      if (e.kind !== "user_message") continue;
      const i = this.outbox.findIndex(
        (x) =>
          x.kind === "user" &&
          x.text === e.data.text.trim() &&
          ids(x.attachments) === ids(e.data.attachments),
      );
      const done = this.outbox[i];
      if (!done) continue;
      // The stored message loads its images from the server.
      if (done.kind === "user")
        for (const url of Object.values(done.previews ?? {})) URL.revokeObjectURL(url);
      this.outbox = this.outbox.filter((_, n) => n !== i);
    }
    return this.outbox.length ? items.concat(this.outbox) : items;
  }

  connect() {
    this.closed = false;
    const after = this.state.lastSeq ? `?after=${this.state.lastSeq}` : "";
    const ws = new WebSocket(wsUrl(`/sessions/${encodeURIComponent(this.id)}/ws${after}`));
    this.ws = ws;
    ws.onopen = () => {
      this.retry = 0;
      this.ping = setInterval(() => this.raw({ type: "ping" }), 25_000);
      this.seen = true;
      this.reportVisibility();
    };
    document.addEventListener("visibilitychange", this.reportVisibility);
    ws.onmessage = (m) => {
      try {
        this.onMessage(JSON.parse(m.data) as G.ViewerServer);
      } catch (e) {
        console.warn("fctl: unreadable message", e);
      }
    };
    ws.onclose = (e) => {
      clearInterval(this.ping);
      if (this.closed) return;
      if (e.code === 1008 || e.code === 4403) {
        this.set({ connection: "closed", error: "You no longer have access to this session." });
        return;
      }
      if (e.code === 4404) {
        this.set({ connection: "closed", error: "This session was deleted with its project." });
        return;
      }
      this.set({ connection: "reconnecting" });
      const delay = Math.min(10_000, 400 * 2 ** this.retry++) * (0.75 + Math.random() * 0.5);
      this.timer = setTimeout(() => this.connect(), delay);
    };
  }

  close() {
    this.closed = true;
    document.removeEventListener("visibilitychange", this.reportVisibility);
    clearTimeout(this.timer);
    clearInterval(this.ping);
    this.ws?.close(1000);
    for (const p of this.pending.values()) p.reject(new Error("closed"));
    this.pending.clear();
  }

  /**
   * Tells the server whether the person can see the session: its page is
   * open and the tab visible. People who can see it get no notifications
   * about it.
   */
  private reportVisibility = () => {
    const visible = this.shown && document.visibilityState === "visible";
    if (visible === this.seen || this.ws?.readyState !== WebSocket.OPEN) return;
    this.seen = visible;
    this.raw({ type: "visibility", visible });
  };

  /** The session's page opened or closed (the socket outlives it for a while). */
  setShown(shown: boolean) {
    this.shown = shown;
    this.reportVisibility();
  }

  private raw(msg: G.ViewerClient) {
    if (this.ws?.readyState === WebSocket.OPEN) this.ws.send(JSON.stringify(msg));
  }

  private onMessage(m: G.ViewerServer) {
    switch (m.type) {
      case "snapshot": {
        const fresh = !this.state.lastSeq;
        const events = m.events;
        if (fresh) {
          this.events = events;
          this.builder = new TranscriptBuilder();
        } else {
          this.events = this.events.concat(events.filter((e) => e.seq > this.state.lastSeq));
        }
        const added = fresh ? events : events.filter((e) => e.seq > this.state.lastSeq);
        const items = this.show(this.builder.add(added), added);
        let live = emptyLive();
        for (const s of m.live) live = applyStream(live, s);
        this.set({
          session: m.session,
          role: m.role,
          items,
          live,
          viewers: m.viewers,
          connection: "live",
          ...this.runner(m.runner_connected, m.runner_last_seen_at, m.agent_unreachable),
          hasMoreBefore: fresh ? m.has_more_before : this.state.hasMoreBefore,
          firstSeq: this.events[0]?.seq ?? 0,
          lastSeq: Math.max(this.state.lastSeq, events[events.length - 1]?.seq ?? 0, m.next_seq - 1),
          git: this.builder.latestGit,
          model: this.builder.model ?? m.session.model,
          error: undefined,
        });
        patchSession(queryClient, m.session);
        // The newest events may show next to nothing: earlier ones fill the page.
        if (fresh) void this.loadOlder(SCREENFUL);
        return;
      }
      case "events": {
        const events = m.events.filter((e) => e.seq > this.state.lastSeq);
        if (!events.length) return;
        this.events = this.events.concat(events);
        const items = this.show(this.builder.add(events), events);
        this.set({
          items,
          live: settleLive(this.state.live, events),
          lastSeq: events[events.length - 1]?.seq ?? this.state.lastSeq,
          git: this.builder.latestGit,
          model: this.builder.model ?? this.state.model,
        });
        return;
      }
      case "stream": {
        const now = performance.now();
        this.deltaTimes.push(now);
        while (this.deltaTimes.length && now - (this.deltaTimes[0] ?? now) > 3000) this.deltaTimes.shift();
        this.set({ live: applyStream(this.state.live, m.event), activity: this.deltaTimes.length / 3 });
        return;
      }
      case "session": {
        // Each viewer's summary has their role, which changes when their access does.
        const role = m.session.role ?? this.state.role;
        this.set({ session: { ...m.session, role }, role });
        patchSession(queryClient, m.session);
        return;
      }
      case "presence":
        this.set({ viewers: m.viewers });
        return;
      case "runner_connected":
        this.set(this.runner(m.connected, m.last_seen_at, m.agent_unreachable));
        return;
      case "result": {
        const p = this.pending.get(m.client_id);
        if (!p) return;
        this.pending.delete(m.client_id);
        if (m.ok) p.resolve();
        else p.reject(new Error(m.error?.message ?? "Not sent"));
        return;
      }
      case "error":
        this.set({ error: m.error.message });
        return;
      case "pong":
        return;
    }
  }

  /**
   * Sends a message with its uploaded files; shows it right away (images
   * from their local previews) and resolves once it is stored.
   */
  send(
    text: string,
    author?: G.UserRef,
    files?: { attachments: G.Attachment[]; previews: Record<string, string> },
  ): Promise<void> {
    const client_id = `web-${Date.now().toString(36)}-${++this.counter}`;
    const attachments = files?.attachments ?? [];
    const optimistic: Item = {
      kind: "user",
      key: `pending-${client_id}`,
      seq: Number.MAX_SAFE_INTEGER,
      ts: Date.now(),
      author,
      text: text.trim(),
      source: "web",
      attachments,
      previews: files?.previews,
      pending: true,
    };
    const ids = attachments.map((a) => a.id);
    this.outbox = [...this.outbox, optimistic];
    this.set({ items: this.show(this.builder.items) });
    const drop = () => {
      this.outbox = this.outbox.filter((i) => i.key !== optimistic.key);
      this.set({ items: this.show(this.builder.items) });
    };
    return new Promise((resolve, reject) => {
      if (this.ws?.readyState === WebSocket.OPEN) {
        this.pending.set(client_id, {
          resolve,
          reject: (e) => {
            drop();
            reject(e);
          },
        });
        this.raw({ type: "send", client_id, text, attachments: ids });
      } else {
        // Not connected: fall back to HTTP (idempotent by client id).
        api.sendMessage(this.id, { text, client_id, attachments: ids }).then(
          () => resolve(),
          (e) => {
            drop();
            reject(e);
          },
        );
      }
    });
  }

  interrupt() {
    const client_id = `web-int-${++this.counter}`;
    if (this.ws?.readyState === WebSocket.OPEN) this.raw({ type: "interrupt", client_id });
    else void api.interrupt(this.id);
  }

  typing() {
    const now = Date.now();
    if (now - this.lastTyping < 2500) return;
    this.lastTyping = now;
    this.raw({ type: "typing" });
  }

  /**
   * Loads the events before the first one we have, a page at a time, until
   * the transcript shows `want` parts (by default a screenful more than it
   * does) or starts where the session did, for at most `MAX_PAGES` pages. The
   * transcript changes once, when they are all in, so that the reader's
   * place can be kept (`useKeepPlace`).
   */
  async loadOlder(want?: number) {
    const { loadingOlder, hasMoreBefore, firstSeq } = this.state;
    if (loadingOlder || !hasMoreBefore || !firstSeq) return;
    const now = parts(this.builder.items).length;
    const target = want ?? now + SCREENFUL;
    if (now >= target) return;
    this.set({ loadingOlder: true, olderFailed: false });
    let older: G.SessionEvent[] = [];
    let more = true;
    try {
      for (let n = 0; n < MAX_PAGES && more; n++) {
        const page = await api.events(this.id, { before: older[0]?.seq ?? firstSeq, limit: PAGE });
        older = page.events.concat(older);
        more = page.has_more;
        if (shown(older.concat(this.events)) >= target) break;
      }
    } catch {
      // What came in shows; the rest loads when the reader asks for it.
    }
    if (!older.length) {
      // There is more and nothing came (the reader may be offline): the page
      // doesn't ask again by itself until the reader comes back to it.
      this.set({ loadingOlder: false, hasMoreBefore: more, olderFailed: more });
      return;
    }
    this.events = older.concat(this.events);
    this.builder = new TranscriptBuilder();
    const items = this.show(this.builder.add(this.events));
    this.set({
      items,
      hasMoreBefore: more,
      firstSeq: this.events[0]?.seq ?? firstSeq,
      loadingOlder: false,
      git: this.builder.latestGit,
    });
  }
}

const sessions = new Map<
  string,
  { store: LiveSession; refs: number; closeTimer?: ReturnType<typeof setTimeout> }
>();

/** The live store for a session (shared between components, closed shortly after the last one leaves). */
export function useLiveSession(id: string, initial?: G.Session): [LiveState, LiveSession] {
  const store = useMemo(() => {
    let e = sessions.get(id);
    if (!e) {
      e = { store: new LiveSession(id, initial), refs: 0 };
      sessions.set(id, e);
      e.store.connect();
    }
    return e.store;
  }, [id, initial]);
  useEffect(() => {
    const e = sessions.get(id);
    if (!e) return;
    e.refs++;
    clearTimeout(e.closeTimer);
    e.store.setShown(true);
    return () => {
      e.refs--;
      if (e.refs <= 0) {
        e.store.setShown(false);
        e.closeTimer = setTimeout(() => {
          e.store.close();
          sessions.delete(id);
        }, 15_000);
      }
    };
  }, [id]);
  const state = useSyncExternalStore(store.subscribe, store.getSnapshot);
  return [state, store];
}

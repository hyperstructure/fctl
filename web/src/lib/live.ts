// A live session: the viewer WebSocket, the transcript, presence, and sends.
//
// One store per open session page. React reads it with useSyncExternalStore;
// every update replaces `state` with a new object (fields keep identity when
// unchanged).

import { useEffect, useMemo, useSyncExternalStore } from "react";
import type * as G from "@/gen";
import { api, wsUrl } from "./api";
import { patchSession, queryClient } from "./queries";
import {
  applyStream,
  emptyLive,
  type Item,
  type LiveTurn,
  settleLive,
  TranscriptBuilder,
} from "./transcript";

export type Connection = "connecting" | "live" | "reconnecting" | "closed";

export interface LiveState {
  session?: G.Session;
  role?: G.SessionRole;
  items: Item[];
  live: LiveTurn;
  viewers: G.Presence[];
  connection: Connection;
  runnerConnected: boolean;
  hasMoreBefore: boolean;
  loadingOlder: boolean;
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
    };
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
    clearTimeout(this.timer);
    clearInterval(this.ping);
    this.ws?.close(1000);
    for (const p of this.pending.values()) p.reject(new Error("closed"));
    this.pending.clear();
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
          runnerConnected: m.runner_connected,
          hasMoreBefore: fresh ? m.has_more_before : this.state.hasMoreBefore,
          firstSeq: this.events[0]?.seq ?? 0,
          lastSeq: Math.max(this.state.lastSeq, events[events.length - 1]?.seq ?? 0, m.next_seq - 1),
          git: this.builder.latestGit,
          model: this.builder.model ?? m.session.model,
          error: undefined,
        });
        patchSession(queryClient, m.session);
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
      case "session":
        this.set({ session: { ...m.session, role: this.state.role } });
        patchSession(queryClient, m.session);
        return;
      case "presence":
        this.set({ viewers: m.viewers });
        return;
      case "runner_connected":
        this.set({ runnerConnected: m.connected });
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

  /** Loads the page of events before the first one we have. */
  async loadOlder() {
    if (this.state.loadingOlder || !this.state.hasMoreBefore || !this.state.firstSeq) return;
    this.set({ loadingOlder: true });
    try {
      const page = await api.events(this.id, { before: this.state.firstSeq, limit: 400 });
      this.events = [...page.events, ...this.events];
      this.builder = new TranscriptBuilder();
      const items = this.show(this.builder.add(this.events));
      this.set({
        items,
        hasMoreBefore: page.has_more,
        firstSeq: this.events[0]?.seq ?? this.state.firstSeq,
        loadingOlder: false,
        git: this.builder.latestGit,
      });
    } catch {
      this.set({ loadingOlder: false });
    }
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
    return () => {
      e.refs--;
      if (e.refs <= 0) {
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

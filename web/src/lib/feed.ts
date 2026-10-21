// The dashboard feed: live session summaries, runners, activity pulses and
// the person's Claude plan usage.

import { useEffect, useSyncExternalStore } from "react";
import type * as G from "@/gen";
import { wsUrl } from "./api";
import { keys, patchSession, queryClient } from "./queries";

export interface Pulse {
  sessionId: string;
  projectId: string;
  kind: string;
  weight: number;
  at: number;
}

type PulseListener = (p: Pulse) => void;

interface FeedState {
  connected: boolean;
  runners: G.Runner[];
  /** Sessions updated through the feed, newest summary per id. */
  sessions: Map<string, G.Session>;
}

class Feed {
  state: FeedState = { connected: false, runners: [], sessions: new Map() };
  private listeners = new Set<() => void>();
  private pulses = new Set<PulseListener>();
  private ws?: WebSocket;
  private retry = 0;
  private refs = 0;
  private timer?: ReturnType<typeof setTimeout>;
  private ping?: ReturnType<typeof setInterval>;

  subscribe = (fn: () => void) => {
    this.listeners.add(fn);
    return () => this.listeners.delete(fn);
  };
  getSnapshot = () => this.state;

  onPulse(fn: PulseListener) {
    this.pulses.add(fn);
    return () => this.pulses.delete(fn);
  }

  private set(patch: Partial<FeedState>) {
    this.state = { ...this.state, ...patch };
    for (const l of this.listeners) l();
  }

  retain() {
    this.refs++;
    if (this.refs === 1) this.open();
  }

  release() {
    this.refs--;
    if (this.refs <= 0) {
      clearTimeout(this.timer);
      clearInterval(this.ping);
      this.ws?.close(1000);
      this.ws = undefined;
    }
  }

  private open() {
    const ws = new WebSocket(wsUrl("/feed/ws"));
    this.ws = ws;
    ws.onopen = () => {
      this.retry = 0;
      this.set({ connected: true });
      this.ping = setInterval(() => ws.readyState === WebSocket.OPEN && ws.send('{"type":"ping"}'), 25_000);
    };
    ws.onmessage = (m) => {
      let msg: G.FeedServer;
      try {
        msg = JSON.parse(m.data);
      } catch {
        return;
      }
      switch (msg.type) {
        case "hello":
          this.set({ runners: msg.runners });
          queryClient.setQueryData<G.RunnerList>(keys.runners, { runners: msg.runners });
          break;
        case "runner": {
          const runners = this.state.runners.filter((r) => r.id !== msg.runner.id).concat(msg.runner);
          runners.sort((a, b) => a.created_at - b.created_at);
          this.set({ runners });
          queryClient.setQueryData<G.RunnerList>(keys.runners, { runners });
          break;
        }
        case "session": {
          const before = this.state.sessions.get(msg.session.id);
          const sessions = new Map(this.state.sessions);
          sessions.set(msg.session.id, msg.session);
          this.set({ sessions });
          patchSession(queryClient, msg.session);
          // New sessions and status changes may move lists; refresh them lazily.
          void queryClient.invalidateQueries({ queryKey: ["sessions"], refetchType: "active" });
          void queryClient.invalidateQueries({ queryKey: keys.projects, refetchType: "active" });
          // Agents keep the memory and record retrospectives as turns end.
          if (before && before.status !== msg.session.status)
            for (const k of [
              keys.memories(msg.session.project_id),
              keys.projectRetrospectives(msg.session.project_id),
              keys.sessionRetrospectives(msg.session.id),
            ])
              void queryClient.invalidateQueries({ queryKey: k, refetchType: "active" });
          break;
        }
        case "activity": {
          const p: Pulse = {
            sessionId: msg.session_id,
            projectId: msg.project_id,
            kind: msg.kind,
            weight: msg.weight,
            at: performance.now(),
          };
          for (const l of this.pulses) l(p);
          break;
        }
        case "claude_login":
          queryClient.setQueryData(["claude-login", msg.job.id], msg.job);
          if (msg.job.state === "succeeded") void queryClient.invalidateQueries({ queryKey: keys.me });
          break;
        case "workspaces":
          // Joined, left, invited, or an admin changed one: what you see changes.
          for (const k of [
            keys.workspaces,
            keys.projects,
            ["sessions"],
            keys.runners,
            ["workspace"],
            keys.me,
          ])
            void queryClient.invalidateQueries({ queryKey: k });
          break;
        case "claude_usage":
          queryClient.setQueryData<G.Me | null>(keys.me, (me) =>
            me
              ? {
                  ...me,
                  claude: me.claude.map((c) => (c.id === msg.credential_id ? { ...c, usage: msg.usage } : c)),
                }
              : me,
          );
          break;
        default:
          break;
      }
    };
    ws.onclose = () => {
      clearInterval(this.ping);
      this.set({ connected: false });
      if (this.refs <= 0) return;
      const delay = Math.min(15_000, 500 * 2 ** this.retry++);
      this.timer = setTimeout(() => this.open(), delay);
    };
  }
}

export const feed = new Feed();

/** Keeps the feed connected while mounted. */
export function useFeed(): FeedState {
  useEffect(() => {
    feed.retain();
    return () => feed.release();
  }, []);
  return useSyncExternalStore(feed.subscribe, feed.getSnapshot);
}

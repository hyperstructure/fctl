import { QueryClient } from "@tanstack/react-query";
import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { keys, patchProjectActivity, patchSession } from "./queries";

const project = (id: string, last_activity_at?: number) => ({ id, last_activity_at }) as G.Project;
const session = (project_id: string, more: Partial<G.Session> = {}) =>
  ({ id: "ses_1", project_id, created_at: 100, last_activity_at: 500, ...more }) as G.Session;

describe("a project's last activity", () => {
  const cached = () => {
    const qc = new QueryClient();
    qc.setQueryData<G.ProjectList>(keys.projects, {
      projects: [project("prj_web", 300), project("prj_api")],
    });
    return {
      qc,
      activity: () => qc.getQueryData<G.ProjectList>(keys.projects)?.projects.map((p) => p.last_activity_at),
    };
  };

  it("moves on with each step of its sessions' agents", () => {
    const { qc, activity } = cached();
    patchProjectActivity(qc, session("prj_web"));
    expect(activity()).toEqual([500, undefined]);
    patchProjectActivity(qc, session("prj_api", { last_activity_at: undefined }));
    expect(activity()).toEqual([500, 100]);
  });

  it("doesn't go back, or count review agents' sessions", () => {
    const { qc, activity } = cached();
    const before = qc.getQueryData(keys.projects);
    patchProjectActivity(qc, session("prj_web", { last_activity_at: 200 }));
    patchProjectActivity(
      qc,
      session("prj_web", { last_activity_at: 900, review: {} as G.Session["review"] }),
    );
    patchProjectActivity(qc, session("prj_other"));
    expect(activity()).toEqual([300, undefined]);
    expect(qc.getQueryData(keys.projects)).toBe(before);
  });
});

describe("a session's summary from the feed", () => {
  const question = { text: "Whole kroner, or øre?", options: ["Whole kroner", "Øre"], asked_at: 400 };
  const cached = (s: G.Session) => {
    const qc = new QueryClient();
    qc.setQueryData<G.SessionList>(keys.sessions({ status: "active" }), { sessions: [s] });
    qc.setQueryData<G.Session>(keys.session(s.id), s);
    return {
      qc,
      now: () => [
        qc.getQueryData<G.SessionList>(keys.sessions({ status: "active" }))?.sessions[0],
        qc.getQueryData<G.Session>(keys.session(s.id)),
      ],
    };
  };

  it("takes away a question it doesn't have, which was answered or taken back", () => {
    const asking = session("prj_web", { status: "idle", role: "owner", question });
    const { qc, now } = cached(asking);
    // The feed leaves out what is empty, and the caller's role.
    const { question: _, role: __, ...withdrawn } = asking;
    patchSession(qc, withdrawn as G.Session);
    for (const s of now()) {
      expect(s).not.toHaveProperty("question");
      expect(s?.role).toBe("owner");
      expect(s?.status).toBe("idle");
    }
  });

  it("keeps the question while the summaries still have it", () => {
    const asking = session("prj_web", { status: "idle", question });
    const { qc, now } = cached(asking);
    patchSession(qc, { ...asking, last_activity_at: 600 });
    for (const s of now()) {
      expect(s?.question).toEqual(question);
      expect(s?.last_activity_at).toBe(600);
    }
  });
});

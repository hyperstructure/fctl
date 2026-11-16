import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  type AuditNames,
  type AuditPart,
  auditActor,
  describeAudit,
  sessionRefs,
  withArticle,
} from "./audit";

const entry = (action: string, target?: string, data?: unknown, actor?: string): G.AuditEntry => ({
  id: `aud_${action}`,
  ts: 0,
  action,
  target,
  data,
  actor: actor ? { id: `usr_${actor.toLowerCase()}`, name: actor } : undefined,
});

/** An entry's sentence as text, with what it links marked: "[shop#232](session)". */
const said = (parts: AuditPart[]) =>
  parts
    .map((p) =>
      typeof p === "string" ? p : p.to ? `[${p.text}](${"session" in p.to ? "session" : "project"})` : p.text,
    )
    .join("");

const names: AuditNames = {
  user: (id) => ({ usr_kofi: "Kofi Mensah", usr_asha: "Asha Patel" })[id],
  project: (key) =>
    key === "prj_farm" || key === "farm-portal" ? { name: "Farm portal", slug: "farm-portal" } : undefined,
};

describe("an audit log entry in words", () => {
  it("names a session it started by its project and number, and links it", () => {
    const e = entry("session.created", "ses_1", { project: "shop", number: 232 }, "Tomás Ferreira");
    expect(said(describeAudit(e))).toBe("started [shop#232](session)");
    expect(auditActor(e)).toBe("Tomás Ferreira");
  });

  it("names a session that an earlier entry started, and says 'a session' when none did", () => {
    const started = entry("session.created", "ses_1", { project: "shop", number: 231 });
    const refs = sessionRefs([entry("session.stopped", "ses_1"), started]);
    const stopped = entry("session.stopped", "ses_1", undefined, "Asha Patel");
    expect(said(describeAudit(stopped, { session: (id) => refs.get(id) }))).toBe(
      "stopped [shop#231](session)",
    );
    expect(said(describeAudit(entry("session.stopped", "ses_2")))).toBe("stopped [a session](session)");
  });

  it("says fctl started what a schedule or an autopilot's wake started, and whose autopilot started the rest", () => {
    const scheduled = entry("session.created", "ses_1", { project: "infra", number: 38, schedule: "sch_1" });
    expect(auditActor(scheduled)).toBe("fctl");
    expect(said(describeAudit(scheduled))).toBe("started [infra#38](session) on a schedule");
    const crew = entry(
      "session.created",
      "ses_2",
      { project: "farm-portal", number: 13, autopilot: "apl_1" },
      "Noor",
    );
    expect(said(describeAudit(crew))).toBe("'s autopilot started [farm-portal#13](session)");
  });

  it("names projects, people and runners by what the page knows, and links projects", () => {
    expect(
      said(describeAudit(entry("project.secret_set", "prj_farm", { name: "RESEND_API_KEY" }), names)),
    ).toBe("set the secret RESEND_API_KEY in [Farm portal](project)");
    expect(
      said(
        describeAudit(
          entry("project.member_added", "prj_farm", { user: "usr_asha", role: "maintainer" }),
          names,
        ),
      ),
    ).toBe("added Asha Patel to [Farm portal](project) as a maintainer");
    expect(said(describeAudit(entry("autopilot.paused", "prj_farm"), names))).toBe(
      "paused [Farm portal](project)'s autopilot",
    );
    expect(
      said(describeAudit(entry("workspace.member_role", "usr_kofi", { from: "member", to: "admin" }), names)),
    ).toBe("made Kofi Mensah an admin");
    expect(
      said(describeAudit(entry("workspace.runner_added", "rnr_1", { name: "turnip", personal: false }))),
    ).toBe("added the runner turnip");
    expect(
      said(describeAudit(entry("workspace.runner_updated", "rnr_1", { name: "turnip", draining: true }))),
    ).toBe("drained the runner turnip");
  });

  it("falls back to what the entry holds when the page doesn't know an id", () => {
    // A project the page doesn't list may be archived or gone: named, not linked.
    expect(said(describeAudit(entry("project.created", "prj_new", { slug: "bookings" })))).toBe(
      "created the project bookings",
    );
    expect(said(describeAudit(entry("workspace.member_role", "usr_gone", { to: "member" })))).toBe(
      "made someone a member",
    );
    expect(said(describeAudit(entry("memory.added", "mem_1", { project_id: "prj_gone" })))).toBe(
      "added a note to a project's memory",
    );
  });

  it("shows the key and target of an action it doesn't know, as the log did before", () => {
    expect(said(describeAudit(entry("workspace.something_new", "wsp_1")))).toBe(
      "workspace.something_new wsp_1",
    );
  });

  it("says an agent published a screenshot, and a review's findings", () => {
    const shot = entry("screenshot.published", "ses_9", { name: "reminders.webp" });
    expect(auditActor(shot)).toBe("An agent");
    expect(said(describeAudit(shot, { session: () => "farm-portal#11" }))).toBe(
      "published reminders.webp from [farm-portal#11](session)",
    );
    expect(
      said(
        describeAudit(
          entry("review.posted", "ses_3", { pull_request: "romanescohq/shop#1291", findings: 1 }),
        ),
      ),
    ).toBe("posted a review of romanescohq/shop#1291, with 1 issue");
  });

  it("puts the right article before a role", () => {
    expect(withArticle("admin")).toBe("an admin");
    expect(withArticle("editor")).toBe("an editor");
    expect(withArticle("member")).toBe("a member");
  });

  it("tells a schedule's Run now from its time coming", () => {
    const now = entry("session.created", "ses_5", { project: "shop", number: 5, schedule: "sch_1" }, "Noor");
    expect(said(describeAudit(now))).toBe("ran a schedule now, which started [shop#5](session)");
  });

  it("says who was moved when an address isn't allowed any more, rather than blaming the admin", () => {
    const left = entry("workspace.left", "usr_kofi", { email: "kofi@example.com", disallowed: true }, "Ada");
    expect(said(describeAudit(left, names))).toBe(
      "removed Kofi Mensah from the workspace, as their address isn't allowed any more",
    );
    expect(
      said(describeAudit(entry("workspace.left", "usr_kofi", { disconnected: true }, "Kofi"), names)),
    ).toBe("left the workspace");
    const taken = entry(
      "user.email_disconnected",
      "usr_kofi",
      { email: "kofi@example.com", disallowed: true },
      "Ada",
    );
    expect(said(describeAudit(taken, names))).toBe(
      "took the address kofi@example.com from Kofi Mensah, as it isn't allowed any more",
    );
  });

  it("names who takes over an instance or a workspace, not who hands it on", () => {
    expect(
      said(describeAudit(entry("instance.transferred", "usr_kofi", { from: "usr_asha" }, "Asha"), names)),
    ).toBe("made Kofi Mensah this instance's primary owner");
    expect(
      said(describeAudit(entry("workspace.transferred", "usr_kofi", { from: "usr_asha" }, "Ada"), names)),
    ).toBe("made Kofi Mensah the primary admin");
    // The instance's log knows the workspace and who was primary admin, not who is now.
    const assigned = entry("workspace.primary_assigned", "wsp_1", { from: "usr_asha", by: "usr_ada" }, "Ada");
    expect(said(describeAudit(assigned, names))).toBe(
      "chose a new primary admin for a workspace, in place of Asha Patel",
    );
  });

  it("credits review agents and runners with what they did themselves", () => {
    const posted = entry("review.posted", "ses_3", { pull_request: "acme/web#12", findings: 0 });
    expect(auditActor(posted)).toBe("fctl's review agent");
    const enrolled = entry("workspace.runner_enrolled", "rnr_1", {
      name: "kohlrabi",
      enrollment_token_name: "Office",
      pending: true,
    });
    expect(auditActor(enrolled)).toBe("The runner kohlrabi");
    expect(said(describeAudit(enrolled))).toBe("enrolled with the token Office, to wait for approval");
    const left = entry("workspace.runner_removed", "rnr_1", { name: "turnip", by: "host" });
    expect(`${auditActor(left)} ${said(describeAudit(left))}`).toBe("The runner turnip left the workspace");
    const swept = entry("workspace.runner_removed", "rnr_1", { name: "turnip", by: "fctl", offline: true });
    expect(`${auditActor(swept)} ${said(describeAudit(swept))}`).toBe(
      "fctl removed the runner turnip, offline too long",
    );
    const drained = entry("runner.drained", "rnr_1", { by: "host" });
    expect(`${auditActor(drained)} ${said(describeAudit(drained))}`).toBe("A runner drained itself");
  });

  it("names a workspace's autopilot from its scope, as it lives on its home project", () => {
    const on = entry("autopilot.on", "prj_farm", { scope: "workspace" }, "Noor");
    expect(said(describeAudit(on, names))).toBe(
      "put the workspace on autopilot, from [Farm portal](project)",
    );
  });

  it("says why GitHub refused, in words", () => {
    const refused = entry("workspace.github_refused", "wsp_1", { reason: "no_installation" }, "Noor");
    expect(said(describeAudit(refused))).toBe(
      "couldn't connect a GitHub account: fctl's GitHub App isn't installed there",
    );
  });

  it("words the Claude token kept out of sandboxes", () => {
    expect(said(describeAudit(entry("claude.stand_in_on", "usr_1", undefined, "Noor")))).toBe(
      "kept their Claude token out of sandboxes",
    );
  });
});

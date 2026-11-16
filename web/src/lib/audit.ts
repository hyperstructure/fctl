// What an audit log entry says, as a sentence that follows who did it: "started shop#232", "added the
// runner turnip", "set the secret RESEND_API_KEY in Farm portal". The control plane records an
// action's key (session.created), its target (an id, or an address) and some data; the sentence
// names what it can, from the data or from what the page knows, and links what can be opened. An
// action the app doesn't know yet shows its key and target, as every entry did before.

import type * as G from "@/gen";

/** Where something an entry names leads: a session's page, or a project's. */
export type AuditTo = { session: string } | { project: string };

/** A piece of an entry's sentence: words, or a thing it names, linked where it can be opened. */
export type AuditPart = string | { text: string; to?: AuditTo; code?: boolean };

/** What the page knows, to name what an entry's ids point at. */
export interface AuditNames {
  /** A person's name, by their id. */
  user?: (id: string) => string | undefined;
  /** A person as the page knows them, picture included, by their id: the log's own leave it out. */
  person?: (id: string) => G.UserRef | undefined;
  /** A project's name and slug, by its id or its slug. */
  project?: (idOrSlug: string) => { name: string; slug: string } | undefined;
  /** A session's reference, "shop#231", by its id. */
  session?: (id: string) => string | undefined;
}

type Data = Record<string, unknown>;

const str = (v: unknown) => (typeof v === "string" && v ? v : undefined);

/** "a member", "an admin", "an editor", "an owner". */
export const withArticle = (word: string) => `${/^[aeiou]/i.test(word) ? "an" : "a"} ${word}`;

/** The "'s" of a name, or "'" after one that ends in s: "Shop's", "Benchmarks'". */
const of = (name: string) => (/s$/i.test(name) ? "'" : "'s");

/** Sessions' references by id ("shop#231"), from the entries that started them. */
export function sessionRefs(entries: readonly G.AuditEntry[]): Map<string, string> {
  const refs = new Map<string, string>();
  for (const e of entries) {
    const ref = e.action === "session.created" ? refFrom(e.data as Data) : undefined;
    if (e.target && ref) refs.set(e.target, ref);
  }
  return refs;
}

function refFrom(d: Data | undefined): string | undefined {
  const project = str(d?.project);
  return project && typeof d?.number === "number" ? `${project}#${d.number}` : undefined;
}

/**
 * Who did what an entry records, when it names nobody: a session's agent, fctl's review agent (its
 * review's findings and summary), a runner's host (`by: "host"`, or enrolling itself with a token),
 * or fctl itself, which starts reviews.
 */
export function auditActor(e: G.AuditEntry): string {
  if (e.actor) return e.actor.name;
  const d = (e.data ?? {}) as Data;
  if (e.action.startsWith("screenshot.")) return "An agent";
  // The review agent posts and sums up its review; fctl starts it, and passes its findings on.
  if (e.action === "review.posted" || e.action === "review.summarized") return "fctl's review agent";
  if (byHost(e)) return str(d.name) ? `The runner ${d.name}` : "A runner";
  return "fctl";
}

/** Whether a runner did it itself, through its host: draining, leaving a workspace, enrolling. */
const byHost = (e: G.AuditEntry) =>
  !e.actor && ((e.data as Data | undefined)?.by === "host" || e.action === "workspace.runner_enrolled");

/** Why GitHub refused to connect an account (`fctl_domain::github::Refusal`), after a colon. */
const REFUSALS: Record<string, string> = {
  // Not installed, or reaching no repository they can see: GitHub doesn't tell the two apart for us.
  no_installation: ": GitHub showed them no installation of fctl's app there",
  no_repos: ": the app reaches no repository there that they can see",
  no_admin: ": they aren't an admin of any repository the app reaches there",
  not_admin: ": they aren't an admin of some of the repositories they named",
  unreachable: ": the app doesn't reach some of the repositories they named",
};

/** What an entry's actor did, in words and the things it names. */
export function describeAudit(e: G.AuditEntry, names: AuditNames = {}): AuditPart[] {
  const d = (e.data ?? {}) as Data;
  const t = e.target;
  const named = (v: unknown, fallback: string): AuditPart => {
    const s = str(v);
    return s ? { text: s } : fallback;
  };
  const code = (v: unknown, fallback: string): AuditPart => {
    const s = str(v);
    return s ? { text: s, code: true } : fallback;
  };
  const session = (id = t): AuditPart =>
    id ? { text: names.session?.(id) ?? refFrom(d) ?? "a session", to: { session: id } } : "a session";
  const project = (key = t, slug = str(d.project) ?? str(d.slug)): AuditPart => {
    const p = (key && names.project?.(key)) || (slug && names.project?.(slug));
    if (p) return { text: p.name, to: { project: p.slug } };
    // One the page doesn't list (archived, hidden, deleted) by its name alone: a link could lead nowhere.
    return slug ? { text: slug } : "a project";
  };
  /** The project's own something: "Farm portal's autopilot". */
  const projects = (what: string, key = t): AuditPart[] => {
    const p = project(key);
    return typeof p === "string" ? [`a project's ${what}`] : [p, `${of(p.text)} ${what}`];
  };
  const user = (id: unknown, fallback = "someone"): AuditPart => {
    const s = str(id);
    const name = s && names.user?.(s);
    return name ? { text: name } : fallback;
  };
  const role = (r: unknown) => (str(r) ? ` as ${withArticle(String(r))}` : "");
  const runner = named(d.name, "a runner");

  switch (e.action) {
    // Sessions.
    case "session.created":
      // An autopilot starts its sessions as its owner, who is the entry's actor.
      if (e.actor && str(d.autopilot)) return ["'s autopilot started ", session()];
      // Someone who presses a schedule's Run now starts its session; its time coming, fctl does.
      if (e.actor && str(d.schedule)) return ["ran a schedule now, which started ", session()];
      return [
        "started ",
        session(),
        str(d.schedule) ? " on a schedule" : "",
        str(d.autopilot) ? " for the autopilot" : "",
      ];
    case "session.stopped":
      return ["stopped ", session()];
    case "session.resumed":
      return ["resumed ", session()];
    case "session.archived":
      return ["archived ", session()];
    case "session.unarchived":
      return ["unarchived ", session()];
    case "session.settings_changed": {
      const model = d.model !== undefined && d.model !== null;
      const effort = d.effort !== undefined && d.effort !== null;
      const what = model && effort ? "the model and effort" : effort ? "the effort" : "the model";
      return [`changed ${what} of `, session()];
    }
    case "session.shared":
      return ["shared ", session(), " with ", user(d.user), role(d.role)];
    case "session.unshared":
      return ["stopped sharing ", session(), " with ", user(d.user)];
    case "session.taken_over":
      return ["moved ", session(), " onto their Claude subscription"];
    case "screenshot.published":
      return ["published ", code(d.name, "a screenshot"), " from ", session()];
    case "screenshot.deleted":
      return ["deleted the screenshot ", code(d.name, "")];
    case "follow_up.comment":
      return [
        "passed ",
        str(d.github_user) ? { text: `${d.github_user}'s comment` } : "a comment",
        " on ",
        code(d.pull_request, "a pull request"),
        " to ",
        session(),
      ];

    // Projects.
    case "project.created":
      return ["created the project ", project()];
    case "project.deleted":
      return ["deleted the project ", named(d.slug, "")];
    case "project.updated":
      return ["changed ", ...projects("settings")];
    case "project.member_added":
      return ["added ", user(d.user), " to ", project(), role(d.role)];
    case "project.repo_added":
      return [
        "added ",
        code(d.repo, "a repository"),
        " to ",
        project(),
        d.access === "read" ? ", read only" : "",
      ];
    case "project.repo_removed":
      return ["removed ", code(d.repo, "a repository"), " from ", project()];
    case "project.secret_set":
      return ["set the secret ", code(d.name, ""), " in ", project()];
    case "project.secret_deleted":
      return ["deleted the secret ", code(d.name, ""), " from ", project()];
    case "memory.added":
      return ["added a note to ", ...projects("memory", str(d.project_id))];
    case "memory.edited":
      return ["edited a note in ", ...projects("memory", str(d.project_id))];
    case "memory.deleted":
      return ["deleted a note from ", ...projects("memory", str(d.project_id))];
    case "retrospective.deleted":
      return ["deleted a retrospective in ", project(str(d.project_id))];
    case "schedule.created":
      return ["created the schedule ", named(d.name, ""), " in ", project(undefined)];
    case "schedule.updated":
      return ["changed the schedule ", named(d.name, "")];
    case "schedule.deleted":
      return ["deleted the schedule ", named(d.name, "")];
    case "schedule.paused":
      return ["paused the schedule ", named(d.name, "")];
    case "schedule.resumed":
      return ["resumed the schedule ", named(d.name, "")];
    // An autopilot lives on a project, a workspace's on its home project, the entry's target.
    case "autopilot.on":
      return d.scope === "workspace"
        ? ["put the workspace on autopilot, from ", project()]
        : ["put ", project(), " on autopilot"];
    case "autopilot.off":
      return ["took ", project(), " off autopilot"];
    case "autopilot.paused":
      return ["paused ", ...projects("autopilot")];
    case "autopilot.resumed":
      return ["resumed ", ...projects("autopilot")];
    case "autopilot.changed":
      return ["changed ", ...projects("autopilot")];
    case "reviews.enabled":
      return ["turned review agents on in ", project()];
    case "reviews.disabled":
      return ["turned review agents off in ", project()];
    case "reviews.updated":
      return ["changed the review agents of ", project()];
    case "reviews.taken_over":
      return ["moved the review agents of ", project(), " onto their Claude subscription"];
    case "review.started":
      return ["started a review of ", code(d.pull_request, "a pull request")];
    case "review.posted":
      return [
        "posted a review of ",
        code(d.pull_request, "a pull request"),
        typeof d.findings === "number"
          ? d.findings === 0
            ? ", with no issues"
            : `, with ${d.findings === 1 ? "1 issue" : `${d.findings} issues`}`
          : "",
      ];
    case "review.summarized":
      return ["summed up the review of ", code(d.pull_request, "a pull request")];
    case "review.passed_on":
    case "review.held_passed_on":
    case "review.unsent_passed_on":
      return ["passed a review's findings to ", session()];

    // The workspace.
    case "workspace.created":
      return ["created the workspace"];
    case "workspace.updated":
      return ["changed the workspace's settings"];
    case "workspace.avatar_set":
      return ["changed the workspace's picture"];
    case "workspace.avatar_removed":
      return ["removed the workspace's picture"];
    case "workspace.deleted":
      return ["deleted the workspace ", named(d.name, "")];
    case "workspace.invited":
      return ["invited ", named(t, "someone"), role(d.role)];
    case "workspace.invitation_revoked":
      return ["took back the invitation to ", named(t, "someone")];
    case "workspace.invitation_declined":
      return ["declined the invitation"];
    case "workspace.joined":
      return ["joined", role(d.role)];
    case "workspace.left":
      return d.disallowed
        ? [
            "removed ",
            user(t, str(d.email) ?? "someone"),
            " from the workspace, as their address isn't allowed any more",
          ]
        : ["left the workspace"];
    case "workspace.member_removed":
      return ["removed ", user(t, str(d.email) ?? "someone"), " from the workspace"];
    case "workspace.member_role":
      return str(d.to)
        ? ["made ", user(t), ` ${withArticle(String(d.to))}`]
        : ["changed the role of ", user(t)];
    case "workspace.transferred":
      return ["made ", user(t), " the primary admin"];
    // In the instance's log, about the workspace: who is primary admin now is in the workspace's.
    case "workspace.primary_assigned":
      return str(d.from)
        ? ["chose a new primary admin for a workspace, in place of ", user(d.from)]
        : ["chose a new primary admin for a workspace"];
    case "workspace.own_runners":
      return [
        d.own_runners
          ? "chose to run their sessions only on their own runners"
          : "let their sessions run on the workspace's runners",
      ];
    case "workspace.runner_added":
      return ["added the runner ", runner, d.personal ? " for their own sessions" : ""];
    case "workspace.runner_removed":
      if (byHost(e)) return ["left the workspace"];
      return ["removed the runner ", runner, d.offline ? ", offline too long" : ""];
    case "workspace.runner_approved":
      return ["approved the runner ", runner];
    case "workspace.runner_rejected":
      return ["turned down the runner ", runner];
    case "workspace.runner_expired":
      return ["removed the runner ", runner, ", which nobody approved in time"];
    case "workspace.runner_enrolled":
      return [
        "enrolled",
        str(d.enrollment_token_name) ? ` with the token ${d.enrollment_token_name}` : "",
        d.pending ? ", to wait for approval" : "",
      ];
    case "workspace.runner_token_rotated":
      return ["made a new token for the runner ", runner];
    case "workspace.runner_updated":
      if (d.draining === true) return ["drained the runner ", runner];
      if (d.draining === false) return ["stopped draining the runner ", runner];
      if (d.enabled === false) return ["disabled the runner ", runner];
      if (d.enabled === true) return ["enabled the runner ", runner];
      return ["changed the runner ", runner];
    case "workspace.enrollment_token_created":
      return ["created the enrollment token ", named(d.name, "")];
    case "workspace.enrollment_token_revoked":
      return ["revoked the enrollment token ", named(d.name, "")];
    case "workspace.github_connected":
      return ["connected the GitHub account ", named(d.account, "")];
    case "workspace.github_disconnected":
      return ["disconnected a GitHub account"];
    case "workspace.github_refused":
      return ["couldn't connect a GitHub account", REFUSALS[String(d.reason)] ?? ""];
    case "workspace.linear_connected":
      return ["connected the Linear workspace ", named(d.name, "")];
    case "workspace.linear_disconnected":
      return ["disconnected Linear"];

    // People and the instance (Admin's log).
    case "user.signup":
      return ["signed up", d.via === "google" ? " with Google" : ""];
    case "user.login":
      return ["signed in", d.via === "google" ? " with Google" : d.passkey ? " with a passkey" : ""];
    case "user.verified":
      return ["confirmed it's them"];
    case "user.password_changed":
      return ["changed their password"];
    case "user.password_reset":
      return ["chose a new password with a reset link"];
    case "user.reset_link":
      return ["made a password reset link for ", user(t)];
    case "user.updated":
      if (str(d.status)) return [d.status === "disabled" ? "disabled " : "enabled ", user(t)];
      return str(d.role) ? ["made ", user(t), ` ${withArticle(String(d.role))}`] : ["changed ", user(t)];
    case "user.deleted":
      return d.self ? ["deleted their account"] : ["deleted the account of ", named(d.email, "someone")];
    case "user.exported":
      return ["exported their data"];
    case "user.email_connected":
      return ["added the address ", named(d.email, "")];
    case "user.email_disconnected":
      return d.disallowed
        ? [
            "took the address ",
            named(d.email, "an address"),
            " from ",
            user(t),
            ", as it isn't allowed any more",
          ]
        : ["removed the address ", named(d.email, "")];
    case "user.primary_email":
      return ["made ", named(d.to, "another address"), " their primary address"];
    case "user.linear_linked":
      return ["linked their Linear account"];
    case "user.linear_unlinked":
      return ["unlinked their Linear account"];
    case "two_factor.enabled":
      return ["turned on two-factor sign-in"];
    case "two_factor.disabled":
      return ["turned off two-factor sign-in"];
    case "two_factor.replaced":
      return ["replaced their authenticator app"];
    case "two_factor.updated":
      return ["changed their two-factor sign-in"];
    case "two_factor.recovery_codes":
      return ["made new recovery codes"];
    case "two_factor.reset":
      return ["reset the two-factor sign-in of ", user(t)];
    case "two_factor.locked":
      return [
        "locked the two-factor sign-in of ",
        user(t),
        typeof d.failures === "number" ? ` after ${d.failures} wrong codes` : "",
      ];
    case "passkey.added":
      return ["added the passkey ", named(d.name, "")];
    case "passkey.removed":
      return ["removed the passkey ", named(d.name, "")];
    case "passkey.counter_went_back":
      return ["flagged the passkey ", named(d.name, ""), ": its counter went back"];
    case "token.created":
      return ["created the access token ", named(d.name, "")];
    case "identity.linked":
      return ["linked their ", d.provider === "google" ? "Google" : named(d.provider, "other"), " sign-in"];
    case "identity.unlinked":
      return ["unlinked a sign-in"];
    case "device.approved":
      return ["signed in ", named(d.client, "a device"), " with a code"];
    case "device.denied":
      return ["turned down a sign-in from ", named(d.client, "a device")];
    case "invite.created":
      return ["invited ", named(t, "someone"), " to this instance"];
    case "invite.revoked":
      return ["took back the invitation to ", named(t, "someone")];
    case "allowlist.added":
      return ["let ", named(t, "someone"), " sign up", role(d.role)];
    case "allowlist.removed":
      return ["stopped letting ", named(t, "someone"), " sign up"];
    case "access_request.removed":
      return ["removed the request for early access from ", named(t, "someone")];
    case "reserved_slug.added":
      return ["reserved the name ", code(t, "")];
    case "reserved_slug.removed":
      return ["released the name ", code(t, "")];
    case "instance.setup":
      return ["set up this instance"];
    case "instance.settings_updated":
      return ["changed this instance's settings"];
    case "instance.transferred":
      return ["made ", user(t), " this instance's primary owner"];
    case "claude.token_added":
      return ["connected a Claude subscription"];
    case "claude.removed":
      return ["removed a Claude connection"];
    case "claude.on_runners_added":
      return ["kept their Claude token on their runners"];
    case "claude.stand_in_on":
      return ["kept their Claude token out of sandboxes"];
    case "claude.stand_in_off":
      return ["let their Claude token into sandboxes again"];
    case "runner.created":
      return ["registered the runner ", runner];
    case "runner.deleted":
      return ["removed a runner"];
    case "runner.drained":
      return [byHost(e) ? "drained itself" : "drained a runner"];
    case "runner.undrained":
      return [byHost(e) ? "stopped draining itself" : "stopped draining a runner"];
    case "runner.token_rotated":
      return ["made a new token for a runner"];
    case "runner.updated":
      return ["changed a runner"];
    case "github.disconnected":
      return ["disconnected a GitHub account from a workspace"];
  }
  // An action this app doesn't know yet: its key and its target, as the log showed every entry.
  return [{ text: e.action, code: true }, t ? " " : "", t ? { text: t, code: true } : ""];
}

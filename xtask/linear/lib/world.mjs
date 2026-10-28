// The Linear workspace the stand-in serves: Acme, with its people, teams,
// issues and the agent sessions on them, and the tokens it handed out. It
// is made up the first time and kept in the state directory from then on,
// as Linear keeps what fctl wrote there, so that the control plane's tokens
// and agent sessions outlive a restart.

import { randomUUID } from "node:crypto";
import { existsSync, mkdirSync, readFileSync, renameSync, writeFileSync } from "node:fs";
import path from "node:path";

/** What a kept world must say to be read back; another starts afresh. */
const VERSION = 1;

/** Its ids are UUIDs, as Linear's are and fctl's ticket settings require (`fctl_domain::tickets::is_linear_id`). */
export const ORGANIZATION = { id: "d3169bac-d292-4f63-8dad-b089afb9484e", name: "Acme", urlKey: "acme" };

/** fctl's app user, which people delegate issues to and mention. */
export const APP_USER = "7b5c7d5b-0d08-442d-8c58-0d8079f385c4";

/** The person who signs in to Linear, as `cargo xtask dev`'s owner does to fctl. */
export const DEV_USER = "660d5178-0339-4b99-a83d-dd841dd7432c";

const GRACE = "0153b353-a761-46f5-ba3f-ff91f227607c";
const ENG = "0ab655aa-2e07-486f-ac77-e8c0ae7a5af4";
const DES = "96a85a65-7c54-498e-b45f-b11aa19f56e8";
const WEBSITE = "79a8a551-4658-4e5b-9bda-03b655e25b2a";

const PRIORITIES = ["No priority", "Urgent", "High", "Medium", "Low"];

/** A team's workflow, as a new Linear team has it. */
function workflow() {
  return [
    ["Backlog", "backlog", "#bec2c8"],
    ["Todo", "unstarted", "#e2e2e2"],
    ["In Progress", "started", "#f2c94c"],
    ["In Review", "started", "#5e6ad2"],
    ["Done", "completed", "#5e6ad2"],
    ["Canceled", "canceled", "#95a2b3"],
  ].map(([name, type, color], position) => ({ id: randomUUID(), name, type, color, position }));
}

/** Acme as it starts: two people, two teams and a few issues to delegate. */
export function seed(now = new Date()) {
  const at = now.toISOString();
  const users = [
    {
      id: APP_USER,
      name: "fctl",
      displayName: "fctl",
      email: `${APP_USER}@oauthapp.linear.app`,
      app: true,
      timezone: null,
    },
    {
      id: DEV_USER,
      name: "Dev Owner",
      displayName: "dev",
      email: "dev@fctl.test",
      app: false,
      timezone: "Europe/Stockholm",
    },
    {
      id: GRACE,
      name: "Grace Hopper",
      displayName: "grace",
      email: "grace@fctl.test",
      app: false,
      timezone: "America/New_York",
    },
  ].map((u) => ({ ...u, avatarUrl: null, createdAt: at }));
  const teams = [
    { id: ENG, key: "ENG", name: "Engineering", color: "#5e6ad2", states: workflow(), issueCount: 0 },
    { id: DES, key: "DES", name: "Design", color: "#f2994a", states: workflow(), issueCount: 0 },
  ];
  const labels = [
    { id: "535aa80c-b5d6-458e-ae3f-fa765a228197", name: "Bug", color: "#eb5757", teamId: ENG },
    { id: "4892a119-6443-42d2-a7b2-d29d41ba48a2", name: "Feature", color: "#bb87fc", teamId: ENG },
    // A workspace label, which every team's issues can have: the one that
    // asks fctl for an exploration (`DEFAULT_EXPLORE_LABEL`).
    { id: "3a33a44c-d103-4e56-a485-49d1f3721131", name: "explore", color: "#4cb782", teamId: null },
  ];
  const projects = [
    { id: WEBSITE, name: "Website", color: "#26b5ce", teamIds: [ENG], completedAt: null, canceledAt: null },
  ];
  const w = {
    version: VERSION,
    users,
    teams,
    labels,
    projects,
    issues: [],
    comments: [],
    sessions: [],
    tokens: {},
  };
  const label = (name) => labels.find((l) => l.name === name).id;
  // Their words are what fake-claude goes by (docs/development.md).
  const notes = addIssue(w, {
    teamId: ENG,
    title: "Notes on what the playground holds",
    description: "Write notes.md about what the repository holds, and push it.",
    labelIds: [label("Feature")],
    creatorId: GRACE,
    priority: 3,
  });
  addIssue(w, {
    teamId: ENG,
    title: "Add a section on the tests",
    description: "Once the notes are in, they should say how to run the tests.",
    parentId: notes.id,
    creatorId: GRACE,
  });
  addIssue(w, {
    teamId: ENG,
    title: "Explore how sessions start",
    description: "Explore how a session starts, then say what you found.",
    labelIds: [label("explore")],
    creatorId: DEV_USER,
  });
  addIssue(w, {
    teamId: ENG,
    title: "Check the pause note",
    description: "Hit the usage limit, then carry on.",
    creatorId: DEV_USER,
    priority: 2,
  });
  addIssue(w, {
    teamId: ENG,
    title: "Fix the footer's links",
    description: "Two links in the footer lead nowhere.",
    projectId: WEBSITE,
    labelIds: [label("Bug")],
    creatorId: GRACE,
  });
  addIssue(w, {
    teamId: DES,
    title: "Draw the empty states",
    description: "The lists that are empty show nothing at all.",
    creatorId: GRACE,
  });
  addComment(w, { issueId: notes.id, userId: GRACE, body: "The README says nothing about the tests yet." });
  return w;
}

/** A URL slug of a title, as Linear puts in issue links. */
function slug(title) {
  return (
    title
      .toLowerCase()
      .replace(/[^a-z0-9]+/g, "-")
      .replace(/^-|-$/g, "")
      .slice(0, 60) || "issue"
  );
}

export function issueUrl(w, issue) {
  return `https://linear.app/${ORGANIZATION.urlKey}/issue/${identifier(w, issue)}/${slug(issue.title)}`;
}

export function identifier(w, issue) {
  return `${team(w, issue.teamId).key}-${issue.number}`;
}

export function priorityLabel(priority) {
  return PRIORITIES[priority] ?? PRIORITIES[0];
}

export const user = (w, id) => w.users.find((u) => u.id === id);
export const team = (w, id) => w.teams.find((t) => t.id === id);

/** A team by its id or its key, as `team(id:)` takes either. */
export function teamByIdOrKey(w, id) {
  return w.teams.find((t) => t.id === id || t.key.toLowerCase() === String(id).toLowerCase());
}

/** A person by their id, handle, name or first name, as `--as` names them. */
export function person(w, name) {
  const n = String(name ?? "").toLowerCase();
  return w.users.find(
    (u) =>
      !u.app &&
      (u.id === name ||
        u.displayName.toLowerCase() === n ||
        u.name.toLowerCase() === n ||
        u.name.split(" ")[0].toLowerCase() === n),
  );
}

export const state = (w, id) => w.teams.flatMap((t) => t.states).find((s) => s.id === id);
export const project = (w, id) => w.projects.find((p) => p.id === id);
export const label = (w, id) => w.labels.find((l) => l.id === id);
export const comment = (w, id) => w.comments.find((c) => c.id === id);
export const session = (w, id) => w.sessions.find((s) => s.id === id);

/** An issue by its id or its key (`ENG-1`, in any case), as `issue(id:)` takes either. */
export function issue(w, id) {
  const key = /^([a-z]+)-(\d+)$/i.exec(String(id).trim());
  if (key) {
    const t = teamByIdOrKey(w, key[1]);
    return t && w.issues.find((i) => i.teamId === t.id && i.number === Number(key[2]));
  }
  return w.issues.find((i) => i.id === id);
}

/** Files an issue in a team: in its first unstarted state unless it says. */
export function addIssue(w, input, now = new Date()) {
  const t = team(w, input.teamId);
  t.issueCount += 1;
  const at = now.toISOString();
  const issue = {
    id: randomUUID(),
    teamId: t.id,
    number: t.issueCount,
    title: input.title,
    description: input.description ?? null,
    stateId: input.stateId ?? t.states.find((s) => s.type === "unstarted").id,
    priority: input.priority ?? 0,
    estimate: input.estimate ?? null,
    dueDate: input.dueDate ?? null,
    projectId: input.projectId ?? null,
    labelIds: input.labelIds ?? [],
    assigneeId: input.assigneeId ?? null,
    delegateId: input.delegateId ?? null,
    creatorId: input.creatorId ?? null,
    parentId: input.parentId ?? null,
    attachments: [],
    createdAt: at,
    updatedAt: at,
  };
  w.issues.push(issue);
  return issue;
}

export function addComment(w, input, now = new Date()) {
  const c = {
    id: randomUUID(),
    issueId: input.issueId,
    body: input.body,
    userId: input.userId,
    parentId: input.parentId ?? null,
    createdAt: now.toISOString(),
    ...(input.anchor ? { anchor: true } : {}),
  };
  w.comments.push(c);
  return c;
}

/**
 * The comment Linear starts a delegation's agent session thread with, in
 * its own words: the delivery carries it as the session's comment, which
 * agents tell from a mention by those words (`fctl_domain::linear`).
 */
export function addAnchor(w, issue, now = new Date()) {
  const app = user(w, APP_USER);
  return addComment(
    w,
    {
      issueId: issue.id,
      userId: app.id,
      body: `This thread is for an agent session with ${app.name}.`,
      anchor: true,
    },
    now,
  );
}

/**
 * An agent session on an issue, as a delegation or a mention makes one:
 * tied to a comment thread, the mention's or the one Linear starts
 * (`addAnchor`).
 */
export function addSession(w, { issue, creatorId, commentId }, now = new Date()) {
  const at = now.toISOString();
  const s = {
    id: randomUUID(),
    issueId: issue.id,
    creatorId,
    commentId,
    appUserId: APP_USER,
    createdAt: at,
    updatedAt: at,
    externalUrls: [],
    plan: null,
    activities: [],
  };
  w.sessions.push(s);
  return s;
}

/** An activity in an agent session: the app's, or a person's prompt. */
export function addActivity(s, activity, now = new Date()) {
  const at = now.toISOString();
  const a = {
    id: randomUUID(),
    createdAt: at,
    ephemeral: false,
    signal: null,
    signalMetadata: null,
    ...activity,
  };
  s.activities.push(a);
  s.updatedAt = at;
  return a;
}

/**
 * An agent session's status, as Linear derives it from its activities:
 * pending until the app answers, then by the app's latest activity.
 */
export function sessionStatus(s) {
  const last = s.activities.findLast((a) => a.userId === APP_USER);
  const prompted = s.activities.findLast((a) => a.userId !== APP_USER);
  if (!last) return "pending";
  if (prompted && prompted.createdAt > last.createdAt && prompted.signal === "stop") return "stopping";
  switch (last.content.type) {
    case "elicitation":
      return "awaitingInput";
    case "response":
      return "complete";
    case "error":
      return "error";
    default:
      return "active";
  }
}

/** The world kept in `dir`, or a new one when there is none (or an older kind). */
export function load(dir) {
  const file = dir && path.join(dir, "world.json");
  if (file && existsSync(file)) {
    const kept = JSON.parse(readFileSync(file, "utf8"));
    if (kept.version === VERSION) return kept;
  }
  return seed();
}

/** Keeps the world in `dir`: written whole, then moved into place. */
export function save(w, dir) {
  if (!dir) return;
  mkdirSync(dir, { recursive: true });
  const file = path.join(dir, "world.json");
  writeFileSync(`${file}.new`, JSON.stringify(w, null, 2));
  renameSync(`${file}.new`, file);
}

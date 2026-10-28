// Linear's GraphQL API, as the stand-in answers it: graphql-js validates
// and executes each query on Linear's own schema (schema.graphql), so that
// a query or a variable Linear wouldn't take fails here too, and refuses a
// query that costs more than Linear allows. What the schema leaves loose,
// the JSON of agent activities and plans, is checked against Linear's
// documented shapes. A field the stand-in doesn't answer fails by name,
// rather than as a null Linear would never send.

import { randomUUID } from "node:crypto";
import { readFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import {
  buildSchema,
  execute,
  GraphQLError,
  isListType,
  isNonNullType,
  isObjectType,
  isScalarType,
  Kind,
  NoDeprecatedCustomRule,
  parse,
  validate,
} from "graphql";
import * as W from "./world.mjs";

const here = path.dirname(fileURLToPath(import.meta.url));

/** Linear's schema, from the copy beside the stand-in. */
export function loadSchema(file = path.join(here, "../schema.graphql")) {
  return buildSchema(readFileSync(file, "utf8"), { assumeValidSDL: true });
}

/** Linear refuses a query that costs more than this many points (linear.app/developers/rate-limiting). */
export const MAX_COMPLEXITY = 10_000;

// ───────────────────────────── errors, as Linear words them ─────────────────────────────

export function notFound(entity) {
  return new GraphQLError(`Entity not found: ${entity}`, {
    extensions: {
      type: "invalid input",
      code: "INPUT_ERROR",
      userPresentableMessage: `Could not find referenced ${entity}.`,
    },
  });
}

export function invalid(what) {
  return new GraphQLError("Argument Validation Error", {
    extensions: { type: "invalid input", code: "INPUT_ERROR", userPresentableMessage: what },
  });
}

export function forbidden(what) {
  return new GraphQLError(what, {
    extensions: { type: "forbidden", code: "FORBIDDEN", userPresentableMessage: what },
  });
}

/** What the stand-in doesn't answer yet: add it here when fctl starts to ask. */
export function unsupported(what) {
  return new GraphQLError(
    `The Linear stand-in doesn't answer ${what}: add it to xtask/linear/lib/graphql.mjs`,
    {
      extensions: { code: "STANDIN_UNSUPPORTED" },
    },
  );
}

/** Whether an error is Linear saying something isn't there, an answer rather than a fault. */
export function isNotFound(error) {
  return error?.extensions?.code === "INPUT_ERROR" && /^Entity not found/.test(error.message ?? "");
}

// ───────────────────────────── connections and filters ─────────────────────────────

/** A connection over `items` (views with ids), paged as Linear pages them: 50 unless asked. */
export function connection(items, { first, last, after, before } = {}) {
  let from = 0;
  let to = items.length;
  if (after != null) from = items.findIndex((x) => x.id === after) + 1;
  if (before != null) {
    const i = items.findIndex((x) => x.id === before);
    if (i >= 0) to = i;
  }
  let nodes = items.slice(from, Math.max(from, to));
  let hasNextPage = to < items.length;
  let hasPreviousPage = from > 0;
  if (last != null && first == null) {
    hasPreviousPage ||= nodes.length > last;
    nodes = last > 0 ? nodes.slice(-last) : [];
  } else {
    const n = first ?? 50;
    hasNextPage ||= nodes.length > n;
    nodes = nodes.slice(0, n);
  }
  return {
    nodes,
    edges: nodes.map((node) => ({ node, cursor: node.id })),
    pageInfo: {
      hasNextPage,
      hasPreviousPage,
      startCursor: nodes[0]?.id ?? null,
      endCursor: nodes.at(-1)?.id ?? null,
    },
  };
}

const COMPARATORS = {
  eq: (v, x) => v === x,
  neq: (v, x) => v !== x,
  in: (v, x) => x.includes(v),
  nin: (v, x) => !x.includes(v),
  lt: (v, x) => v < x,
  lte: (v, x) => v <= x,
  gt: (v, x) => v > x,
  gte: (v, x) => v >= x,
  null: (v, x) => (v == null) === x,
  eqIgnoreCase: (v, x) => String(v).toLowerCase() === String(x).toLowerCase(),
  neqIgnoreCase: (v, x) => String(v).toLowerCase() !== String(x).toLowerCase(),
  contains: (v, x) => String(v ?? "").includes(x),
  notContains: (v, x) => !String(v ?? "").includes(x),
  containsIgnoreCase: (v, x) =>
    String(v ?? "")
      .toLowerCase()
      .includes(String(x).toLowerCase()),
  startsWith: (v, x) => String(v ?? "").startsWith(x),
  endsWith: (v, x) => String(v ?? "").endsWith(x),
};

/** A view's field: a value, or what its resolver gives. */
function field(view, name) {
  const v = view?.[name];
  return typeof v === "function" ? v({}) : v;
}

/**
 * Whether `view` matches a Linear filter: comparators on its fields, the
 * filters of the entities it refers to, `some`/`every` (or the fields
 * themselves, for some) on its collections, and `and`/`or`. A field the
 * view lacks fails by name.
 */
export function matches(view, filter, what = "it") {
  if (filter == null) return true;
  return Object.entries(filter).every(([key, cond]) => {
    if (cond == null) return true;
    if (key === "and") return cond.every((f) => matches(view, f, what));
    if (key === "or") return cond.some((f) => matches(view, f, what));
    if (key in COMPARATORS) return COMPARATORS[key](view, cond);
    if (view == null) return false;
    if (!(key in view)) throw unsupported(`a filter on ${what}'s ${key}`);
    const value = field(view, key);
    if (value && typeof value === "object" && Array.isArray(value.nodes)) {
      const { some, every, length, ...rest } = cond;
      return (
        (some == null || value.nodes.some((n) => matches(n, some, key))) &&
        (every == null || value.nodes.every((n) => matches(n, every, key))) &&
        (length == null || matches(value.nodes.length, length, key)) &&
        (Object.keys(rest).length === 0 || value.nodes.some((n) => matches(n, rest, key)))
      );
    }
    // An entity, by its own filter.
    if (value && typeof value === "object") return matches(value, cond, key);
    // A value, or an entity that isn't there, which matches no filter on its fields.
    return Object.entries(cond).every(([op, x]) => {
      if (op in COMPARATORS) return COMPARATORS[op](value, x);
      if (value == null) return false;
      throw unsupported(`the filter ${op} on ${what}'s ${key}`);
    });
  });
}

// ───────────────────────────── views ─────────────────────────────

/** Views are what graphql-js reads: values, and functions for the fields that take arguments or refer on. */
export function views(w) {
  const organization = () => ({
    id: W.ORGANIZATION.id,
    name: W.ORGANIZATION.name,
    urlKey: W.ORGANIZATION.urlKey,
    logoUrl: null,
    createdAt: "2026-01-05T09:00:00.000Z",
    teams: (a) => connection(w.teams.map(team), a),
    users: (a) => connection(w.users.map(user), a),
  });
  const user = (u) =>
    u && {
      id: u.id,
      name: u.name,
      displayName: u.displayName,
      email: u.email,
      avatarUrl: u.avatarUrl,
      app: u.app,
      active: true,
      admin: !u.app,
      guest: false,
      timezone: u.timezone,
      url: `https://linear.app/${W.ORGANIZATION.urlKey}/profiles/${u.displayName}`,
      createdAt: u.createdAt,
      updatedAt: u.createdAt,
      organization,
    };
  const state = (s) => s && { ...s, createdAt: "2026-01-05T09:00:00.000Z" };
  const label = (l) =>
    l && {
      id: l.id,
      name: l.name,
      color: l.color,
      isGroup: false,
      createdAt: "2026-01-05T09:00:00.000Z",
      team: () => (l.teamId ? team(W.team(w, l.teamId)) : null),
    };
  const project = (p) =>
    p && {
      id: p.id,
      name: p.name,
      color: p.color,
      completedAt: p.completedAt,
      canceledAt: p.canceledAt,
      url: `https://linear.app/${W.ORGANIZATION.urlKey}/project/${p.name.toLowerCase()}-${p.id.slice(-12)}`,
      createdAt: "2026-01-05T09:00:00.000Z",
      teams: (a) =>
        connection(
          p.teamIds.map((id) => team(W.team(w, id))),
          a,
        ),
    };
  const team = (t) =>
    t && {
      id: t.id,
      key: t.key,
      name: t.name,
      displayName: t.name,
      color: t.color,
      issueCount: t.issueCount,
      timezone: "Europe/Stockholm",
      createdAt: "2026-01-05T09:00:00.000Z",
      organization,
      states: (a) =>
        connection(
          t.states.map(state).filter((s) => matches(s, a.filter, "a state")),
          a,
        ),
      projects: (a) =>
        connection(
          w.projects
            .filter((p) => p.teamIds.includes(t.id))
            .map(project)
            .filter((p) => matches(p, a.filter, "a project")),
          a,
        ),
      labels: (a) =>
        connection(
          w.labels
            .filter((l) => l.teamId === t.id)
            .map(label)
            .filter((l) => matches(l, a.filter, "a label")),
          a,
        ),
    };
  const comment = (c) =>
    c && {
      id: c.id,
      body: c.body,
      url: `${W.issueUrl(w, W.issue(w, c.issueId))}#comment-${c.id.slice(0, 8)}`,
      createdAt: c.createdAt,
      updatedAt: c.createdAt,
      botActor: null,
      user: () => user(W.user(w, c.userId)),
      parent: () => comment(W.comment(w, c.parentId)),
      issue: () => issue(W.issue(w, c.issueId)),
    };
  const issue = (i) =>
    i && {
      id: i.id,
      identifier: W.identifier(w, i),
      number: i.number,
      title: i.title,
      description: i.description,
      url: W.issueUrl(w, i),
      branchName: `${W.identifier(w, i).toLowerCase()}`,
      priority: i.priority,
      priorityLabel: W.priorityLabel(i.priority),
      estimate: i.estimate,
      dueDate: i.dueDate,
      createdAt: i.createdAt,
      updatedAt: i.updatedAt,
      labelIds: i.labelIds,
      cycle: null,
      state: () => state(W.state(w, i.stateId)),
      team: () => team(W.team(w, i.teamId)),
      project: () => project(W.project(w, i.projectId)),
      assignee: () => user(W.user(w, i.assigneeId)),
      delegate: () => user(W.user(w, i.delegateId)),
      creator: () => user(W.user(w, i.creatorId)),
      parent: () => issue(W.issue(w, i.parentId)),
      labels: (a) =>
        connection(
          i.labelIds
            .map((id) => label(W.label(w, id)))
            .filter(Boolean)
            .filter((l) => matches(l, a.filter, "a label")),
          a,
        ),
      children: (a) =>
        connection(
          w.issues
            .filter((c) => c.parentId === i.id)
            .map(issue)
            .filter((c) => matches(c, a.filter, "an issue")),
          a,
        ),
      comments: (a) =>
        connection(
          w.comments
            .filter((c) => c.issueId === i.id)
            .map(comment)
            .filter((c) => matches(c, a.filter, "a comment")),
          a,
        ),
      attachments: (a) => connection(i.attachments, a),
    };
  const activity = (a) => ({
    id: a.id,
    createdAt: a.createdAt,
    updatedAt: a.createdAt,
    ephemeral: a.ephemeral,
    signal: a.signal,
    signalMetadata: a.signalMetadata,
    content: {
      __typename: `AgentActivity${a.content.type[0].toUpperCase()}${a.content.type.slice(1)}Content`,
      ...a.content,
    },
    user: () => user(W.user(w, a.userId)),
  });
  const session = (s) =>
    s && {
      id: s.id,
      createdAt: s.createdAt,
      updatedAt: s.updatedAt,
      status: W.sessionStatus(s),
      plan: s.plan,
      url: `${W.issueUrl(w, W.issue(w, s.issueId))}#agent-session-${s.id.slice(0, 8)}`,
      issue: () => issue(W.issue(w, s.issueId)),
      creator: () => user(W.user(w, s.creatorId)),
      appUser: () => user(W.user(w, s.appUserId)),
      comment: () => comment(W.comment(w, s.commentId)),
      activities: (a) => connection(s.activities.map(activity), a),
    };
  return { organization, user, team, state, label, project, issue, comment, activity, session };
}

// ───────────────────────────── what fctl posts ─────────────────────────────

/** What an agent activity's content holds for each type it may post (linear.app/developers/agent-interaction). */
const CONTENT = {
  thought: { required: ["body"], optional: [] },
  elicitation: { required: ["body"], optional: [] },
  response: { required: ["body"], optional: [] },
  error: { required: ["body"], optional: [] },
  action: { required: ["action", "parameter"], optional: ["result"] },
};

const PLAN_STATUSES = ["pending", "inProgress", "completed", "canceled"];

/** Why Linear wouldn't take an agent activity (`AgentActivityCreateInput`), or nothing. */
export function activityProblem(input) {
  const c = input.content;
  if (c == null || typeof c !== "object" || Array.isArray(c)) return "content must be an object";
  if (c.type === "prompt") return "an agent can't post a prompt: they are people's";
  const shape = CONTENT[c.type];
  if (!shape)
    return `content.type must be one of ${Object.keys(CONTENT).join(", ")}, not ${JSON.stringify(c.type)}`;
  for (const k of shape.required) {
    if (typeof c[k] !== "string") return `a ${c.type} needs content.${k}, a string`;
  }
  for (const k of shape.optional) {
    if (c[k] != null && typeof c[k] !== "string") return `content.${k} must be a string`;
  }
  const extra = Object.keys(c).filter(
    (k) => k !== "type" && !shape.required.includes(k) && !shape.optional.includes(k),
  );
  if (extra.length) return `a ${c.type} has no ${extra.join(", ")}`;
  if (input.ephemeral && !["thought", "action"].includes(c.type)) {
    return `only thoughts and actions may be ephemeral, not a ${c.type}`;
  }
  const meta = input.signalMetadata;
  switch (input.signal) {
    case undefined:
    case null:
      return meta == null ? null : "signalMetadata needs a signal";
    case "auth":
      if (c.type !== "elicitation") return "the auth signal goes with an elicitation";
      if (typeof meta?.url !== "string" || !/^https?:\/\//.test(meta.url))
        return "the auth signal needs signalMetadata.url";
      if (meta.userId != null && typeof meta.userId !== "string")
        return "signalMetadata.userId must be a string";
      if (meta.providerName != null && typeof meta.providerName !== "string") {
        return "signalMetadata.providerName must be a string";
      }
      return null;
    case "select":
      if (c.type !== "elicitation") return "the select signal goes with an elicitation";
      if (!Array.isArray(meta?.options) || meta.options.length === 0)
        return "the select signal needs signalMetadata.options";
      for (const o of meta.options) {
        if (typeof o?.value !== "string") return "each of signalMetadata.options needs a value, a string";
        if (o.label != null && typeof o.label !== "string") return "an option's label must be a string";
      }
      return null;
    default:
      return `an agent sends the ${input.signal} signal to nobody: people send it`;
  }
}

/** Why Linear wouldn't take an agent session's plan, or nothing: a whole list of steps. */
export function planProblem(plan) {
  if (!Array.isArray(plan)) return "plan must be the whole list of steps";
  for (const step of plan) {
    if (typeof step?.content !== "string") return "each step of the plan needs content, a string";
    if (!PLAN_STATUSES.includes(step.status)) {
      return `a step's status must be one of ${PLAN_STATUSES.join(", ")}, not ${JSON.stringify(step.status)}`;
    }
    const extra = Object.keys(step).filter((k) => k !== "content" && k !== "status");
    if (extra.length) return `a step of the plan has no ${extra.join(", ")}`;
  }
  return null;
}

/** Why Linear wouldn't take an agent session's links, or nothing. */
export function linksProblem(links) {
  const urls = links.map((l) => l.url);
  if (urls.some((u) => !/^https?:\/\//.test(u))) return "each external URL must be an http(s) address";
  if (new Set(urls).size !== urls.length) return "an agent session's external URLs must differ";
  return null;
}

// ───────────────────────────── resolvers ─────────────────────────────

/**
 * What the stand-in answers, for `actor` (`{ userId, app, scopes }`, from
 * the token): the root fields of queries and mutations. Mutations say in
 * `effects` that the world changed, and how, in a line for people.
 */
export function root(w, actor, effects = { changed: false, notes: [] }) {
  const v = views(w);
  const now = () => new Date();
  const by = W.user(w, actor.userId)?.name ?? "someone";
  const did = (note) => {
    effects.changed = true;
    effects.notes.push(`${by} ${note}`);
  };
  const keyOf = (i) => W.identifier(w, i);
  const write = () => {
    if (!actor.scopes.includes("write") && !actor.scopes.includes("admin")) {
      throw forbidden("Invalid scope: `write` required");
    }
  };
  const asApp = (what) => {
    if (!actor.app) throw forbidden(`Only an app installed with actor=app can ${what}`);
  };
  const theIssue = (id) => W.issue(w, id) ?? fail(notFound("Issue"));
  const theSession = (id) => W.session(w, id) ?? fail(notFound("AgentSession"));
  const theUser = (id) => (id == null ? null : (W.user(w, id) ?? fail(notFound("User"))));
  const synced = () => ({ success: true, lastSyncId: Date.now() });
  /** Applies `input` to an issue: the fields fctl sets, and none other silently. */
  const apply = (i, input) => {
    for (const [k, x] of Object.entries(input)) {
      switch (k) {
        case "title":
          if (!x?.trim()) throw invalid("title must not be empty");
          i.title = x;
          break;
        case "description":
          i.description = x;
          break;
        case "teamId":
          break;
        case "stateId": {
          const t = W.team(w, i.teamId);
          if (!t.states.some((s) => s.id === x)) throw invalid(`the state isn't one of ${t.key}'s`);
          i.stateId = x;
          break;
        }
        case "priority":
          if (!Number.isInteger(x) || x < 0 || x > 4) throw invalid("priority must be 0 to 4");
          i.priority = x;
          break;
        case "estimate":
        case "dueDate":
          i[k] = x;
          break;
        case "projectId":
          if (x != null && !W.project(w, x)?.teamIds.includes(i.teamId))
            throw invalid("the project isn't the team's");
          i.projectId = x;
          break;
        case "parentId":
          if (x != null && (x === i.id || !W.issue(w, x))) throw notFound("Issue");
          i.parentId = x == null ? null : W.issue(w, x).id;
          break;
        case "assigneeId":
        case "delegateId":
          i[k] = theUser(x)?.id ?? null;
          break;
        case "labelIds":
        case "addedLabelIds":
        case "removedLabelIds": {
          for (const id of x) {
            const l = W.label(w, id);
            if (!l || (l.teamId && l.teamId !== i.teamId)) throw notFound("IssueLabel");
          }
          if (k === "labelIds") i.labelIds = [...x];
          if (k === "addedLabelIds") i.labelIds = [...new Set([...i.labelIds, ...x])];
          if (k === "removedLabelIds") i.labelIds = i.labelIds.filter((id) => !x.includes(id));
          break;
        }
        default:
          throw unsupported(`an issue's ${k}`);
      }
    }
    i.updatedAt = now().toISOString();
  };
  const fields = {
    // Queries.
    viewer: () => v.user(W.user(w, actor.userId)),
    organization: () => v.organization(),
    teams: (a) =>
      connection(
        w.teams.map(v.team).filter((t) => matches(t, a.filter, "a team")),
        a,
      ),
    team: ({ id }) => v.team(W.teamByIdOrKey(w, id) ?? fail(notFound("Team"))),
    project: ({ id }) => v.project(W.project(w, id) ?? fail(notFound("Project"))),
    issueLabels: (a) =>
      connection(
        w.labels.map(v.label).filter((l) => matches(l, a.filter, "a label")),
        a,
      ),
    issue: ({ id }) => v.issue(theIssue(id)),
    issues: (a) => connection(issues(w, v, a), a),
    searchIssues: (a) => {
      const words = a.term.toLowerCase().split(/\s+/).filter(Boolean);
      const found = issues(w, v, a)
        .filter((i) => !a.teamId || i.team().id === a.teamId)
        .filter((i) => {
          const text = `${i.identifier} ${i.title} ${i.description ?? ""}`.toLowerCase();
          return words.every((word) => text.includes(word));
        });
      return { ...connection(found, a), totalCount: found.length, archivePayload: synced() };
    },
    agentSession: ({ id }) => v.session(theSession(id)),
    issueRepositorySuggestions: ({ issueId, candidateRepositories }) => {
      const i = theIssue(issueId);
      const text = `${i.title} ${i.description ?? ""}`.toLowerCase();
      // Linear ranks the candidates by the issue; here, by whether it names them.
      const suggestions = candidateRepositories
        .filter((c) => {
          const full = c.repositoryFullName.toLowerCase();
          return text.includes(full) || text.includes(full.split("/").pop());
        })
        .map((c) => ({ repositoryFullName: c.repositoryFullName, hostname: c.hostname, confidence: 0.9 }));
      return { suggestions };
    },

    // Mutations.
    issueCreate: ({ input }) => {
      write();
      const t = W.teamByIdOrKey(w, input.teamId) ?? fail(notFound("Team"));
      if (!input.title?.trim()) throw invalid("title must not be empty");
      const { teamId: _, title, ...rest } = input;
      const i = W.addIssue(w, { teamId: t.id, title, creatorId: actor.userId });
      try {
        apply(i, rest);
      } catch (e) {
        w.issues.pop();
        t.issueCount -= 1;
        throw e;
      }
      did(`filed ${keyOf(i)}: ${i.title}`);
      return { ...synced(), issue: v.issue(i) };
    },
    issueUpdate: ({ id, input }) => {
      write();
      const i = theIssue(id);
      // All of it or none: a change Linear refuses changes nothing.
      const changedIssue = structuredClone(i);
      apply(changedIssue, input);
      Object.assign(i, changedIssue);
      const what = Object.keys(input).map((k) =>
        k === "stateId" ? `moved it to ${W.state(w, i.stateId).name}` : k,
      );
      did(`changed ${keyOf(i)}: ${what.join(", ")}`);
      return { ...synced(), issue: v.issue(i) };
    },
    commentCreate: ({ input }) => {
      write();
      const { issueId, body, parentId, ...rest } = input;
      for (const k of Object.keys(rest)) throw unsupported(`a comment's ${k}`);
      const i = theIssue(issueId);
      if (typeof body !== "string" || !body.trim()) throw invalid("body must not be empty");
      const parent = parentId == null ? null : W.comment(w, parentId);
      if (parentId != null && parent?.issueId !== i.id) throw notFound("Comment");
      const c = W.addComment(w, { issueId: i.id, body, userId: actor.userId, parentId: parent?.id });
      did(`commented on ${keyOf(i)}${parent ? " in a thread" : ""}: ${clip(body)}`);
      return { ...synced(), comment: v.comment(c) };
    },
    agentActivityCreate: ({ input }) => {
      asApp("post in an agent session");
      const s = theSession(input.agentSessionId);
      const problem = activityProblem(input);
      if (problem) throw invalid(problem);
      const a = W.addActivity(s, {
        userId: actor.userId,
        content: input.content,
        ephemeral: input.ephemeral ?? false,
        signal: input.signal ?? null,
        signalMetadata: input.signalMetadata ?? null,
      });
      const c = input.content;
      const text =
        c.type === "action" ? `${c.action} ${c.parameter}${c.result ? ` → ${c.result}` : ""}` : c.body;
      const extra = [input.ephemeral && "ephemeral", input.signal].filter(Boolean).join(", ");
      did(`in ${keyOf(W.issue(w, s.issueId))}: ${c.type}${extra ? ` (${extra})` : ""}: ${clip(text)}`);
      return { ...synced(), agentActivity: v.activity(a) };
    },
    agentSessionUpdate: ({ id, input }) => {
      asApp("change an agent session");
      const s = structuredClone(theSession(id));
      for (const [k, x] of Object.entries(input)) {
        switch (k) {
          case "externalUrls":
          case "addedExternalUrls": {
            const links = k === "externalUrls" ? x : [...s.externalUrls, ...x];
            const problem = linksProblem(links);
            if (problem) throw invalid(problem);
            s.externalUrls = links.map(({ label, url }) => ({ label, url }));
            break;
          }
          case "removedExternalUrls":
            s.externalUrls = s.externalUrls.filter((l) => !x.includes(l.url));
            break;
          case "plan": {
            const problem = planProblem(x);
            if (problem) throw invalid(problem);
            s.plan = x;
            break;
          }
          case "summary":
          case "externalLink":
          case "dismissedAt":
            s[k] = x;
            break;
          default:
            throw unsupported(`an agent session's ${k}`);
        }
      }
      s.updatedAt = now().toISOString();
      Object.assign(theSession(id), s);
      did(`changed ${keyOf(W.issue(w, s.issueId))}'s agent session: ${Object.keys(input).join(", ")}`);
      return { ...synced(), agentSession: v.session(theSession(id)) };
    },
    attachmentLinkURL: ({ issueId, url, title }) => {
      write();
      const i = theIssue(issueId);
      if (!/^https?:\/\//.test(url)) throw invalid("url must be an http(s) address");
      let a = i.attachments.find((x) => x.url === url);
      if (!a) {
        a = { id: randomUUID(), url, title: title ?? url, createdAt: now().toISOString() };
        i.attachments.push(a);
      }
      i.updatedAt = now().toISOString();
      did(`attached ${url} to ${keyOf(i)}`);
      return { ...synced(), attachment: a };
    },
  };
  return new Proxy(fields, {
    get: (target, name) => {
      if (name in target || typeof name !== "string" || name === "then") return target[name];
      return () => {
        throw unsupported(name);
      };
    },
  });
}

function fail(error) {
  throw error;
}

/** A line of text, short enough for a log line. */
function clip(text) {
  const line = String(text ?? "")
    .replace(/\s+/g, " ")
    .trim();
  return line.length > 140 ? `${line.slice(0, 139)}…` : line;
}

/** The issues an `issues` or `searchIssues` query asks for, in its order. */
function issues(w, v, a) {
  const order = a.orderBy === "updatedAt" ? "updatedAt" : "createdAt";
  return w.issues
    .map(v.issue)
    .filter((i) => matches(i, a.filter, "an issue"))
    .sort((x, y) => y[order].localeCompare(x[order]));
}

// ───────────────────────────── complexity ─────────────────────────────

/**
 * What Linear charges for an operation: a field 0.1 point, an object 1, and
 * a connection its children's points times how many it asks for (`first`
 * or `last`, else 50), rounded up, as fctl_domain::linear's tests count it.
 */
export function complexity(document, variables = {}, operationName) {
  const fragments = Object.fromEntries(
    document.definitions.filter((d) => d.kind === Kind.FRAGMENT_DEFINITION).map((d) => [d.name.value, d]),
  );
  const operation = document.definitions.find(
    (d) => d.kind === Kind.OPERATION_DEFINITION && (!operationName || d.name?.value === operationName),
  );
  const value = (node) => {
    if (!node) return null;
    if (node.kind === Kind.VARIABLE) return variables[node.name.value] ?? null;
    if (node.kind === Kind.INT) return Number(node.value);
    return null;
  };
  // Tenths of a point, and whether the set lists nodes, as a connection's does.
  const set = (selectionSet) => {
    let cost = 0;
    let lists = false;
    for (const s of selectionSet.selections) {
      if (s.kind === Kind.FIELD) {
        lists ||= s.name.value === "nodes" || s.name.value === "edges";
        if (!s.selectionSet) {
          cost += 1;
          continue;
        }
        const arg = (name) => value(s.arguments?.find((x) => x.name.value === name)?.value);
        const asked = arg("first") ?? arg("last");
        const [children, nodes] = set(s.selectionSet);
        cost += asked != null ? asked * children : nodes ? 50 * children : 10 + children;
      } else {
        const inner =
          s.kind === Kind.FRAGMENT_SPREAD ? fragments[s.name.value]?.selectionSet : s.selectionSet;
        if (inner) {
          const [c, l] = set(inner);
          cost += c;
          lists ||= l;
        }
      }
    }
    return [cost, lists];
  };
  return operation ? Math.ceil(set(operation.selectionSet)[0] / 10) : 0;
}

// ───────────────────────────── webhook payloads ─────────────────────────────

/** What a value of each of Linear's scalars is in JSON. */
const SCALARS = {
  String: "string",
  ID: "string",
  DateTime: "string",
  TimelessDate: "string",
  Float: "number",
  Int: "number",
  Boolean: "boolean",
  JSONObject: "object",
};

/**
 * How `value` differs from type `name` of Linear's schema, as Linear's
 * webhook payloads are typed there (`AgentSessionEventWebhookPayload`): a
 * field it lacks, a field the type lacks, a value of the wrong kind.
 */
export function differences(schema, name, value) {
  const walk = (type, v, at) => {
    if (isNonNullType(type)) return v == null ? [`${at} is missing`] : walk(type.ofType, v, at);
    if (v == null) return [];
    if (isListType(type)) {
      return Array.isArray(v)
        ? v.flatMap((x, i) => walk(type.ofType, x, `${at}[${i}]`))
        : [`${at} isn't a list`];
    }
    if (isObjectType(type)) {
      if (typeof v !== "object" || Array.isArray(v)) return [`${at} isn't an object`];
      const fields = type.getFields();
      return [
        ...Object.keys(v)
          .filter((k) => !(k in fields))
          .map((k) => `${at}.${k} isn't a field of ${type.name}`),
        ...Object.values(fields).flatMap((f) => walk(f.type, v[f.name], `${at}.${f.name}`)),
      ];
    }
    if (isScalarType(type) && SCALARS[type.name] && typeof v !== SCALARS[type.name]) {
      return [`${at} should be a ${type.name}`];
    }
    return [];
  };
  return walk(schema.getType(name), value, name);
}

// ───────────────────────────── answering ─────────────────────────────

/** The fields an operation asks for at its root (`issue`, `agentActivityCreate`), which name it. */
function rootFields(document, operationName) {
  const operation = document.definitions.find(
    (d) => d.kind === Kind.OPERATION_DEFINITION && (!operationName || d.name?.value === operationName),
  );
  return (operation?.selectionSet.selections ?? [])
    .filter((s) => s.kind === Kind.FIELD)
    .map((s) => s.name.value);
}

/**
 * Answers a GraphQL request as Linear would, for `actor`: its HTTP status
 * and body, the fields it asked for at its root, and the uses of what
 * Linear deprecated, which still work.
 */
export async function answer(schema, w, actor, request, effects) {
  const { query, variables, operationName } = request ?? {};
  if (typeof query !== "string") {
    return {
      status: 400,
      body: { errors: [graphqlError("A request needs a query", "BAD_REQUEST")] },
      deprecated: [],
    };
  }
  let document;
  try {
    document = parse(query);
  } catch (e) {
    return {
      status: 400,
      body: { errors: [graphqlError(e.message, "GRAPHQL_PARSE_FAILED")] },
      deprecated: [],
    };
  }
  const fields = rootFields(document, operationName);
  const invalidQuery = validate(schema, document);
  if (invalidQuery.length) {
    return {
      status: 400,
      body: { errors: invalidQuery.map((e) => graphqlError(e.message, "GRAPHQL_VALIDATION_FAILED", e)) },
      fields,
      deprecated: [],
    };
  }
  const deprecated = validate(schema, document, [NoDeprecatedCustomRule]).map((e) => e.message);
  const points = complexity(document, variables ?? {}, operationName);
  if (points > MAX_COMPLEXITY) {
    const message = `Query too complex: it costs ${points} points, and Linear takes at most ${MAX_COMPLEXITY}`;
    return {
      status: 400,
      body: { errors: [graphqlError(message, "QUERY_TOO_COMPLEX")] },
      fields,
      deprecated,
    };
  }
  const result = await execute({
    schema,
    document,
    rootValue: root(w, actor, effects),
    variableValues: variables ?? {},
    operationName,
  });
  const body = { ...result };
  if (result.errors) {
    // Errors outside any field are about the variables, which Linear refuses whole.
    body.errors = result.errors.map((e) =>
      graphqlError(e.message, e.path ? "INTERNAL_SERVER_ERROR" : "BAD_USER_INPUT", e),
    );
  }
  // Linear refuses variables it can't take with 400, and answers with 200 once it ran the query.
  return { status: "data" in result ? 200 : 400, body, fields, deprecated };
}

/** Resolvers that check what the schema leaves loose, and answer nothing. */
function dryRoot() {
  const fields = {
    agentActivityCreate: ({ input }) => {
      const problem = activityProblem(input);
      if (problem) throw invalid(problem);
      return null;
    },
    agentSessionUpdate: ({ input }) => {
      const problem =
        ("plan" in input ? planProblem(input.plan) : null) ??
        (input.externalUrls ? linksProblem(input.externalUrls) : null);
      if (problem) throw invalid(problem);
      return null;
    },
  };
  return new Proxy(fields, {
    get: (target, name) => (name in target || name === "then" ? target[name] : () => null),
  });
}

/**
 * What Linear would refuse of `operations` (`{ name, query, variables }`),
 * as the stand-in would, without running them: the query, against Linear's
 * schema; its variables, against the types it declares; what it costs with
 * them; and the JSON the schema leaves loose. Also what of Linear's schema
 * they use that Linear deprecated, which still works.
 */
export async function check(schema, operations) {
  const problems = [];
  const deprecated = [];
  for (const { name, query, variables } of operations) {
    let document;
    try {
      document = parse(query);
    } catch (e) {
      problems.push({ name, message: e.message });
      continue;
    }
    const refused = validate(schema, document);
    if (refused.length) {
      problems.push(...refused.map((e) => ({ name, message: e.message })));
      continue;
    }
    deprecated.push(
      ...validate(schema, document, [NoDeprecatedCustomRule]).map((e) => ({ name, message: e.message })),
    );
    const points = complexity(document, variables ?? {});
    if (points > MAX_COMPLEXITY) {
      problems.push({
        name,
        message: `it costs ${points} points, and Linear takes at most ${MAX_COMPLEXITY}`,
      });
    }
    const result = await execute({ schema, document, rootValue: dryRoot(), variableValues: variables ?? {} });
    for (const e of result.errors ?? []) {
      // Errors outside any field are about the variables; INPUT_ERROR is the checks'.
      if (!e.path || e.extensions?.code === "INPUT_ERROR") {
        const said = e.extensions?.userPresentableMessage;
        problems.push({ name, message: said ?? e.message });
      }
    }
  }
  return { problems, deprecated };
}

function graphqlError(message, code, e) {
  const out = { message };
  if (e?.locations) out.locations = e.locations;
  if (e?.path) out.path = e.path;
  out.extensions = { type: "graphql error", code, ...(e?.extensions ?? {}) };
  return out;
}

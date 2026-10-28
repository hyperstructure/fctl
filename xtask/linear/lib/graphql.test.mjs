import assert from "node:assert/strict";
import { test } from "node:test";
import { parse } from "graphql";
import { activityProblem, answer, complexity, isNotFound, loadSchema, planProblem } from "./graphql.mjs";
import * as W from "./world.mjs";

const schema = loadSchema();
const app = { userId: W.APP_USER, app: true, scopes: ["read", "write", "app:assignable", "app:mentionable"] };
const person = { userId: W.DEV_USER, app: false, scopes: ["read"] };

const ask = (w, actor, query, variables) => answer(schema, w, actor, { query, variables });

test("counts complexity as Linear's docs and fctl_domain::linear's tests do", () => {
  const cost = (q, vars) => complexity(parse(q), vars);
  assert.equal(cost(`query WhoAmI { user(id: "me") { name } }`), 2);
  const mine = `query MyCreatedIssues { user(id: "me") { createdIssues { nodes { id title createdAt } } } }`;
  assert.equal(cost(mine), 66);
  assert.equal(cost(mine.replace("createdIssues", "createdIssues(first: 10)")), 14);
  // The teams query of fctl 0.22, as Linear refused it.
  const teams =
    "query { teams(first: 100) { nodes { id key name color states(first: 25) { nodes { id name type position } } projects(first: 50) { nodes { id name completedAt canceledAt } } } } }";
  assert.equal(cost(teams), 10_640);
  assert.equal(cost("query($n: Int!) { teams(first: $n) { nodes { id } } }", { n: 250 }), 275);
});

test("refuses what Linear's schema doesn't have, and queries that cost too much", async () => {
  const w = W.seed();
  const unknown = await ask(w, app, "query { viewer { id handle } }");
  assert.equal(unknown.status, 400);
  assert.equal(unknown.body.errors[0].extensions.code, "GRAPHQL_VALIDATION_FAILED");
  assert.match(unknown.body.errors[0].message, /Cannot query field "handle" on type "User"/);
  const wrongType = await ask(w, app, "query($id: Int!) { issue(id: $id) { id } }", { id: 1 });
  assert.equal(wrongType.status, 400);
  const tooMuch = await ask(
    w,
    app,
    "query { teams(first: 250) { nodes { states(first: 250) { nodes { id name } } } } }",
  );
  assert.equal(tooMuch.body.errors[0].extensions.code, "QUERY_TOO_COMPLEX");
  const missingVariable = await ask(w, app, "query($id: String!) { issue(id: $id) { id } }", {});
  assert.equal(missingVariable.status, 400);
  assert.equal(missingVariable.body.errors[0].extensions.code, "BAD_USER_INPUT");
});

test("finds issues by key or id, and says so as Linear does when one isn't there", async () => {
  const w = W.seed();
  const q =
    "query($id: String!) { issue(id: $id) { id identifier title parent { identifier } team { key } } }";
  const byKey = await ask(w, app, q, { id: "eng-2" });
  assert.equal(byKey.body.data.issue.identifier, "ENG-2");
  assert.equal(byKey.body.data.issue.parent.identifier, "ENG-1");
  const byId = await ask(w, app, q, { id: byKey.body.data.issue.id });
  assert.equal(byId.body.data.issue.identifier, "ENG-2");
  const gone = await ask(w, app, q, { id: "ENG-99" });
  assert.equal(gone.status, 200);
  assert.ok(isNotFound(gone.body.errors[0]));
  assert.equal(gone.body.errors[0].extensions.userPresentableMessage, "Could not find referenced Issue.");
});

test("filters issues by team, state type and label, as fctl's ticket list does", async () => {
  const w = W.seed();
  const eng = W.teamByIdOrKey(w, "ENG");
  const q =
    "query($filter: IssueFilter, $first: Int!) { issues(filter: $filter, first: $first, orderBy: updatedAt) { nodes { identifier } pageInfo { hasNextPage } } }";
  const keys = async (filter, first = 50) =>
    (await ask(w, app, q, { filter, first })).body.data.issues.nodes.map((n) => n.identifier).sort();
  assert.deepEqual(await keys({ team: { id: { eq: eng.id } } }), [
    "ENG-1",
    "ENG-2",
    "ENG-3",
    "ENG-4",
    "ENG-5",
  ]);
  assert.deepEqual(await keys({ team: { id: { eq: eng.id } }, labels: { name: { eqIgnoreCase: "BUG" } } }), [
    "ENG-5",
  ]);
  assert.deepEqual(
    await keys({ team: { id: { eq: eng.id } }, state: { type: { in: ["completed", "canceled"] } } }),
    [],
  );
  assert.deepEqual(await keys({ project: { null: false } }), ["ENG-5"]);
  const page = await ask(w, app, q, { filter: null, first: 2 });
  assert.equal(page.body.data.issues.pageInfo.hasNextPage, true);
  const search = await ask(
    w,
    app,
    "query($term: String!, $first: Int!) { searchIssues(term: $term, first: $first) { nodes { identifier } } }",
    { term: "pause note", first: 10 },
  );
  assert.deepEqual(
    search.body.data.searchIssues.nodes.map((n) => n.identifier),
    ["ENG-4"],
  );
});

test("takes agent activities only in the shapes Linear documents", () => {
  const ok = (input) => assert.equal(activityProblem({ agentSessionId: "s", ...input }), null);
  const bad = (input, why) => assert.match(activityProblem({ agentSessionId: "s", ...input }) ?? "", why);
  ok({ content: { type: "thought", body: "Looking at ENG-1…" }, ephemeral: true });
  ok({ content: { type: "action", action: "Reading", parameter: "src/main.rs", result: "done" } });
  ok({ content: { type: "response", body: "Done." } });
  ok({
    content: { type: "elicitation", body: "Link your account" },
    signal: "auth",
    signalMetadata: {
      url: "http://localhost:8787/api/v1/linear/link?thread=s",
      providerName: "fctl",
      userId: "u",
    },
  });
  ok({
    content: { type: "elicitation", body: "Which project?" },
    signal: "select",
    signalMetadata: { options: [{ label: "Playground", value: "playground" }] },
  });
  bad({ content: { type: "prompt", body: "hi" } }, /people's/);
  bad({ content: { type: "thought" } }, /needs content.body/);
  bad({ content: { type: "action", action: "Reading" } }, /needs content.parameter/);
  bad({ content: { type: "response", body: "Done.", title: "x" } }, /has no title/);
  bad({ content: { type: "response", body: "Done." }, ephemeral: true }, /only thoughts and actions/);
  bad(
    { content: { type: "elicitation", body: "Link" }, signal: "auth", signalMetadata: {} },
    /needs signalMetadata.url/,
  );
  bad(
    { content: { type: "response", body: "Link" }, signal: "auth", signalMetadata: { url: "http://x" } },
    /elicitation/,
  );
  bad(
    { content: { type: "elicitation", body: "Pick" }, signal: "select", signalMetadata: { options: [] } },
    /options/,
  );
  bad({ content: { type: "response", body: "Stopped." }, signal: "stop" }, /people send it/);
  bad({ content: { type: "thought", body: "x" }, signalMetadata: { url: "http://x" } }, /needs a signal/);
});

test("takes a plan only as the whole list of its steps", () => {
  assert.equal(planProblem([{ content: "Read the code", status: "inProgress" }]), null);
  assert.match(planProblem({ steps: [] }), /whole list/);
  assert.match(planProblem([{ content: "Read", status: "in_progress" }]), /status must be one of/);
  assert.match(planProblem([{ status: "pending" }]), /needs content/);
});

test("posts in agent sessions only as the app, and writes only with the write scope", async () => {
  const w = W.seed();
  const i = W.issue(w, "ENG-1");
  const s = W.addSession(w, { issue: i, creatorId: W.DEV_USER, commentId: null });
  const post =
    "mutation($input: AgentActivityCreateInput!) { agentActivityCreate(input: $input) { success agentActivity { id content { ... on AgentActivityThoughtContent { body } } } } }";
  const input = {
    agentSessionId: s.id,
    content: { type: "thought", body: "Looking at ENG-1…" },
    ephemeral: true,
  };
  const asPerson = await ask(w, person, post, { input });
  assert.equal(asPerson.body.errors[0].extensions.code, "FORBIDDEN");
  const effects = { changed: false, notes: [] };
  const asApp = await answer(schema, w, app, { query: post, variables: { input } }, effects);
  assert.equal(asApp.body.data.agentActivityCreate.agentActivity.content.body, "Looking at ENG-1…");
  assert.equal(s.activities.length, 1);
  assert.equal(W.sessionStatus(s), "active");
  assert.deepEqual(effects.notes, ["fctl in ENG-1: thought (ephemeral): Looking at ENG-1…"]);
  const elsewhere = await ask(w, app, post, { input: { ...input, agentSessionId: "nope" } });
  assert.ok(isNotFound(elsewhere.body.errors[0]));
  const comment = await ask(
    w,
    person,
    "mutation($input: CommentCreateInput!) { commentCreate(input: $input) { success } }",
    { input: { issueId: i.id, body: "Hello" } },
  );
  assert.match(comment.body.errors[0].message, /`write` required/);
});

test("changes issues all at once or not at all", async () => {
  const w = W.seed();
  const i = W.issue(w, "ENG-1");
  const eng = W.team(w, i.teamId);
  const des = W.teamByIdOrKey(w, "DES");
  const update =
    "mutation($id: String!, $input: IssueUpdateInput!) { issueUpdate(id: $id, input: $input) { success issue { state { name } } } }";
  const started = eng.states.find((s) => s.name === "In Progress");
  const moved = await ask(w, app, update, { id: i.id, input: { stateId: started.id } });
  assert.equal(moved.body.data.issueUpdate.issue.state.name, "In Progress");
  const before = structuredClone(i);
  const refused = await ask(w, app, update, {
    id: i.id,
    input: { title: "Renamed", stateId: des.states[0].id },
  });
  assert.equal(refused.body.errors[0].extensions.code, "INPUT_ERROR");
  assert.deepEqual(i, before);
  const created = await ask(
    w,
    app,
    "mutation($input: IssueCreateInput!) { issueCreate(input: $input) { success issue { identifier creator { name } } } }",
    { input: { teamId: eng.id, title: "Tidy up", labelIds: ["nope"] } },
  );
  assert.ok(isNotFound(created.body.errors[0]));
  assert.equal(w.issues.length, 6);
});

test("says by name what it doesn't answer, rather than answer null", async () => {
  const w = W.seed();
  const res = await ask(w, app, "query { cycles(first: 1) { nodes { id } } }");
  assert.equal(res.body.errors[0].extensions.code, "STANDIN_UNSUPPORTED");
  assert.match(res.body.errors[0].message, /doesn't answer cycles/);
});

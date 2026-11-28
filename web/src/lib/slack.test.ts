import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  connectOutcome,
  DEFAULT_EVENT_KINDS,
  eventsSentence,
  githubStanding,
  integrationOf,
  isChannelId,
  kindsOf,
  linearStanding,
  linkOutcome,
  messageDid,
  noticeLine,
  placeName,
  projectSlackGist,
  SLACK_OUTCOMES,
  sessionPlace,
  sessionPlaceLabel,
  settingSaved,
  slackGist,
  slackStanding,
  someNames,
} from "./slack";

const all = () => true;

describe("which integration an address opens", () => {
  it("opens the one it names", () => {
    expect(integrationOf({ tab: "integrations", integration: "slack" }, all)).toBe("slack");
    expect(integrationOf({ tab: "integrations", integration: "linear" }, all)).toBe("linear");
  });

  it("opens GitHub's and Linear's old tabs there, as the control plane still sends people back to them", () => {
    expect(integrationOf({ tab: "github" }, all)).toBe("github");
    expect(integrationOf({ tab: "linear" }, all)).toBe("linear");
  });

  it("opens Slack for a way back from Slack that names no integration", () => {
    expect(integrationOf({ tab: "integrations", slack: "connected" }, all)).toBe("slack");
  });

  it("opens GitHub otherwise, and never one the workspace hasn't", () => {
    expect(integrationOf({ tab: "integrations" }, all)).toBe("github");
    expect(integrationOf({ integration: "slack" }, (i) => i !== "slack")).toBe("github");
    expect(integrationOf({ tab: "linear" }, (i) => i === "github")).toBe("github");
  });
});

describe("how an integration stands", () => {
  const linear = (statuses: G.LinearStatus[], configured = true): G.WorkspaceLinear => ({
    configured,
    webhook: true,
    connections: statuses.map((status, i) => ({
      organization: `o${i}`,
      name: ["Acme Inc", "Globex", "Initech"][i] ?? "x",
      url_key: "x",
      status,
      created_at: 0,
    })),
  });
  const slack = (statuses: G.SlackStatus[], configured = true): G.WorkspaceSlack => ({
    configured,
    events: true,
    socket_mode: false,
    match_email: false,
    instance_admin: false,
    connections: statuses.map(
      (status, i) => ({ team: `T${i}`, name: ["Acme", "Globex"][i] ?? "x", status }) as G.SlackConnection,
    ),
  });

  it("names what it's connected to", () => {
    expect(githubStanding(["acme"], true)).toEqual({
      state: "on",
      text: "Connected to acme",
      short: "Connected",
    });
    expect(githubStanding(["acme", "globex", "initech"], true).text).toBe("Connected to acme and 2 more");
    expect(linearStanding(linear(["active", "active"])).text).toBe("Connected to Acme Inc and Globex");
    expect(slackStanding(slack(["active"])).text).toBe("Connected to Acme");
  });

  it("says when nothing is connected yet, or the instance has no app", () => {
    expect(githubStanding([], true)).toEqual({ state: "off", text: "Not connected yet", short: "Not yet" });
    expect(githubStanding([], false).text).toBe("No GitHub App here");
    expect(linearStanding(linear([])).text).toBe("Not connected yet");
    expect(linearStanding(linear([], false)).text).toBe("No Linear app here");
    expect(slackStanding(slack([], false)).text).toBe("No Slack app yet");
  });

  it("says when the service took fctl's access back, and only then", () => {
    expect(slackStanding(slack(["revoked"]))).toEqual({
      state: "wrong",
      text: "Access revoked",
      short: "Revoked",
    });
    expect(linearStanding(linear(["revoked"])).state).toBe("wrong");
    // One that works is what counts.
    expect(slackStanding(slack(["revoked", "active"])).state).toBe("on");
  });

  it("warns while nothing people do in Slack reaches fctl", () => {
    expect(slackStanding({ ...slack(["active"]), events: false }).state).toBe("wrong");
    expect(slackStanding({ ...slack(["active"]), socket_mode: true, socket_connected: false }).text).toBe(
      "Connected, but nothing reaches fctl",
    );
    expect(slackStanding({ ...slack(["active"]), socket_mode: true, socket_connected: true }).state).toBe(
      "on",
    );
  });

  it("is loading until it's known", () => {
    expect(slackStanding(undefined).state).toBe("loading");
    expect(linearStanding(undefined).state).toBe("loading");
  });

  it("names a few, then counts", () => {
    expect(someNames(["a"])).toBe("a");
    expect(someNames(["a", "b"])).toBe("a and b");
    expect(someNames(["a", "b", "c", "d"])).toBe("a and 3 more");
  });
});

describe("coming back from Slack", () => {
  it("says what installing fctl's app led to, by whose doing", () => {
    expect(connectOutcome("connected", "Acme")?.tone).toBe("ok");
    expect(connectOutcome("connected", "Acme")?.text).toContain("fctl's app is in Acme now");
    // Choosing not to isn't anyone's mistake.
    expect(connectOutcome("denied")?.tone).toBe("info");
    expect(connectOutcome("expired")?.text).toContain("longer than ten minutes");
    expect(connectOutcome("other_user")?.text).toContain("another fctl account");
    expect(connectOutcome("enterprise")?.text).toContain("Enterprise Grid");
    expect(connectOutcome("failed")?.tone).toBe("error");
    // Linking's ways back aren't the workspace's.
    expect(connectOutcome("linked")).toBeUndefined();
    expect(connectOutcome("confirm")).toBeUndefined();
  });

  it("says what linking an account led to", () => {
    expect(linkOutcome("linked")?.text).toContain("what you ask fctl there starts sessions as you");
    expect(linkOutcome("denied")?.tone).toBe("info");
    expect(linkOutcome("failed")?.tone).toBe("error");
    // Confirming asks rather than tells.
    expect(linkOutcome("confirm")).toBeUndefined();
  });

  it("knows every way back the control plane sends", () => {
    for (const o of [
      "connected",
      "expired",
      "other_user",
      "failed",
      "denied",
      "enterprise",
      "linked",
      "confirm",
    ])
      expect(SLACK_OUTCOMES).toContain(o);
  });
});

describe("places and messages in Slack", () => {
  it("names where, as Slack does", () => {
    expect(placeName({ place: "channel", channel_name: "web-team" })).toBe("#web-team");
    expect(placeName({ place: "private", channel_name: "secret" })).toBe("#secret");
    expect(placeName({ place: "private" })).toBe("a private channel");
    expect(placeName({ place: "dm" })).toBe("direct messages");
  });

  it("names where a session started, and says it whole for tooltips", () => {
    const s = (place: G.SlackPlace, channel_name?: string): G.SessionSlack => ({
      team: "T1",
      team_name: "Acme",
      channel: "C1",
      channel_name,
      place,
      thread_ts: "1.000100",
      url: "https://acme.slack.com/archives/C1/p1000100",
    });
    expect(sessionPlace(s("channel", "web-team"))).toBe("#web-team");
    expect(sessionPlace(s("dm"))).toBe("Direct message");
    expect(sessionPlaceLabel(s("channel", "web-team"))).toBe("Started in #web-team in Slack in Acme");
    expect(sessionPlaceLabel(s("dm"))).toBe("Started in a direct message with fctl in Slack in Acme");
    expect(sessionPlaceLabel(s("private", "secret"))).toBe(
      "Started in the private channel #secret in Slack in Acme",
    );
  });

  it("says what someone did there, after their name", () => {
    const m = (event: G.SlackEvent, place: G.SlackPlace = "channel"): G.SlackMessage => ({
      event,
      place,
      channel_name: "web-team",
    });
    expect(messageDid(m("mentioned"))).toBe("mentioned fctl in #web-team");
    expect(messageDid(m("direct", "dm"))).toBe("wrote to fctl");
    expect(messageDid(m("replied"))).toBe("replied in the thread in #web-team");
    expect(messageDid(m("replied", "dm"))).toBe("replied in the thread");
    expect(messageDid(m("answered"))).toBe("answered the agent's question");
    expect(messageDid(m("shortcut"))).toBe("sent a message to fctl");
    expect(messageDid(m("command"))).toBe("asked with /fctl in #web-team");
  });

  it("says what fctl did in the thread for the agent", () => {
    const n = (place: G.SlackPlace, extra: Partial<G.SlackNotice> = {}): G.SlackNotice => ({
      place,
      channel_name: "web-team",
      ...extra,
    });
    expect(noticeLine("slack_replied", n("channel"))).toBe("Replied in #web-team");
    expect(noticeLine("slack_replied", n("dm"))).toBe("Replied in the direct messages with fctl");
    expect(noticeLine("slack_asked", n("channel"))).toBe("Asked in #web-team");
    expect(noticeLine("slack_posted", n("channel"))).toBe("Posted in #web-team");
    expect(noticeLine("slack_read", n("channel", { channel_name: "incidents", messages: 12 }))).toBe(
      "Read 12 messages in #incidents",
    );
    expect(noticeLine("slack_read", n("channel", { messages: 1 }))).toBe("Read 1 message in #web-team");
    expect(noticeLine("slack_failed", n("channel"))).toBe("Couldn't post in #web-team");
  });
});

describe("a project's events in a channel", () => {
  const events = (kinds?: string[]): G.SlackEvents => ({
    team: "T1",
    channel: "C1",
    channel_name: "web-team",
    kinds,
  });

  it("hears pull requests ready for review and failures unless it chose", () => {
    expect(kindsOf(events())).toEqual(DEFAULT_EVENT_KINDS);
    expect(kindsOf(events([]))).toEqual(DEFAULT_EVENT_KINDS);
    expect(kindsOf(events(["session.question"]))).toEqual(["session.question"]);
  });

  it("says where it posts and when, in a sentence", () => {
    expect(eventsSentence(events())).toBe(
      "Posts to #web-team when pull requests are ready for review or a session fails.",
    );
    expect(eventsSentence(events(["session.question", "pull_request.merged", "session.failed"]))).toBe(
      "Posts to #web-team when a session fails, an agent asks something or an agent's pull request merges.",
    );
  });

  it("sums the project's choices up for its list of settings", () => {
    expect(projectSlackGist({ slack: { read_channels: true, events: events() } } as G.ProjectSettings)).toBe(
      "Agents read the readable channels · events to #web-team",
    );
    expect(projectSlackGist({ slack: { read_channels: false } } as G.ProjectSettings)).toBe(
      "Agents read their thread alone",
    );
    // On by default: the workspace's admins chose those channels for agents to read.
    expect(projectSlackGist({} as G.ProjectSettings)).toBe("Agents read the readable channels");
  });

  it("takes a channel by Slack's id", () => {
    expect(isChannelId("C0123ABCD")).toBe(true);
    expect(isChannelId("G0123ABCD")).toBe(true);
    expect(isChannelId("#web-team")).toBe(false);
    expect(isChannelId("c0123abcd")).toBe(false);
  });
});

describe("what a change comes to", () => {
  it("says what follows each change", () => {
    expect(settingSaved({ channels: false })).toBe("Mentions, /fctl and Send to fctl… start nothing now");
    expect(settingSaved({ direct_messages: true })).toBe("Direct messages with fctl start sessions now");
    expect(settingSaved({ notifications: false })).toBe("Notifications stay out of Slack now");
    expect(settingSaved({ transcripts: "drop" })).toContain("drop what Slack's messages said");
    expect(settingSaved({ match_email: true })).toContain("verified address");
  });

  it("says which Slack workspaces you're linked in", () => {
    expect(slackGist(undefined)).toBeUndefined();
    expect(slackGist([])?.text).toBe("Not linked");
    expect(slackGist([{ team_name: "Acme" } as G.SlackAccount])?.text).toBe("Linked in Acme");
    expect(slackGist([{ team_name: "Acme" }, { team_name: "Globex" }] as G.SlackAccount[])?.text).toBe(
      "Linked in 2 Slack workspaces",
    );
  });
});

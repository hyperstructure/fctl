// Slack (docs/architecture.md#slack): what the web app says of fctl's Slack app. Where a Slack
// sign-in left the browser, how an integration of a workspace stands, where in Slack a message or a
// session is, what a Slack message did and what fctl did in Slack for an agent, the events a project
// posts to a channel, and what a change to a workspace's choices comes to.

import type * as G from "@/gen";

// ───────────────────────────── a workspace's integrations ─────────────────────────────

/** The integrations a workspace's Integrations tab moves between, in its order. */
export const INTEGRATIONS = ["github", "linear", "slack"] as const;
export type Integration = (typeof INTEGRATIONS)[number];

export const INTEGRATION_NAMES: Record<Integration, string> = {
  github: "GitHub",
  linear: "Linear",
  slack: "Slack",
};

/**
 * The integration an address opens on a workspace's page: the one it names (`?integration=`), or,
 * from before GitHub, Linear and Slack shared a tab, the tab it names (`?tab=github`, `?tab=linear`,
 * which the control plane still sends back to), or where a Slack sign-in came back to. The first one
 * the workspace has, otherwise: GitHub.
 */
export function integrationOf(
  search: { tab?: string; integration?: Integration; slack?: string },
  has: (i: Integration) => boolean,
): Integration {
  const asked: Integration | undefined =
    search.integration ??
    (search.tab === "github" || search.tab === "linear" ? search.tab : search.slack ? "slack" : undefined);
  if (asked && has(asked)) return asked;
  return INTEGRATIONS.find(has) ?? "github";
}

/** How an integration stands for a workspace, as the Integrations tab's switcher says it. */
export interface Standing {
  /** On (connected), off (not yet), wrong (access revoked), or still loading. */
  state: "on" | "off" | "wrong" | "loading";
  /** In a few words: "Connected to acme", "Not connected yet", "Access revoked". */
  text: string;
}

/** "acme", "acme and globex", "acme and 2 more". */
export function someNames(names: readonly string[]): string {
  if (names.length <= 2) return names.join(" and ");
  return `${names[0]} and ${names.length - 1} more`;
}

/** How GitHub stands: the accounts the workspace works in, which its summary lists. */
export function githubStanding(accounts: readonly string[], instanceHasApp: boolean): Standing {
  if (accounts.length > 0) return { state: "on", text: `Connected to ${someNames(accounts)}` };
  return { state: "off", text: instanceHasApp ? "Not connected yet" : "No GitHub App here" };
}

/** How Linear stands: the Linear workspaces it works in, and whether Linear took fctl's access back. */
export function linearStanding(l: G.WorkspaceLinear | undefined): Standing {
  if (!l) return { state: "loading", text: "" };
  if (!l.configured) return { state: "off", text: "No Linear app here" };
  const active = l.connections.filter((c) => c.status === "active").map((c) => c.name);
  if (active.length > 0) return { state: "on", text: `Connected to ${someNames(active)}` };
  if (l.connections.length > 0) return { state: "wrong", text: "Access revoked" };
  return { state: "off", text: "Not connected yet" };
}

/** How Slack stands: the Slack workspaces it works in, and whether Slack took fctl's access back. */
export function slackStanding(s: G.WorkspaceSlack | undefined): Standing {
  if (!s) return { state: "loading", text: "" };
  if (!s.configured) return { state: "off", text: "No Slack app yet" };
  const active = s.connections.filter((c) => c.status === "active").map((c) => c.name);
  if (active.length > 0) return { state: "on", text: `Connected to ${someNames(active)}` };
  if (s.connections.length > 0) return { state: "wrong", text: "Access revoked" };
  return { state: "off", text: "Not connected yet" };
}

// ───────────────────────────── coming back from Slack ─────────────────────────────

/**
 * Where a Slack sign-in left the browser (`?slack=`): installing fctl's app for a workspace
 * (api::slack::installed, back at the workspace's Integrations tab), linking someone's account
 * (api::slack::linked, back at Settings → Slack), or linking that waits for them to confirm it's them
 * (`confirm`, where the instance asks for that).
 */
export const SLACK_OUTCOMES = [
  "connected",
  "linked",
  "confirm",
  "expired",
  "other_user",
  "failed",
  "denied",
  "enterprise",
] as const;
export type SlackOutcome = (typeof SLACK_OUTCOMES)[number];

export interface Said {
  tone: "ok" | "info" | "warn" | "error";
  text: string;
}

/** What to tell an admin back from installing fctl's app in Slack, and how it went. */
export function connectOutcome(o: SlackOutcome, team?: string): Said | undefined {
  switch (o) {
    case "connected":
      return {
        tone: "ok",
        text: `fctl's app is in ${team ?? "your Slack workspace"} now: people there start sessions by mentioning it in a channel or writing to it, once they link their account. Choose below which channels start sessions in which projects, and which channels agents read.`,
      };
    case "denied":
      // Not anyone's mistake: they chose not to.
      return { tone: "info", text: "You didn't allow fctl's app in Slack, so nothing was connected." };
    case "expired":
      return {
        tone: "info",
        text: "That sign-in took longer than ten minutes, so nothing was connected. Add fctl to Slack again.",
      };
    case "other_user":
      return {
        tone: "info",
        text: "That link to add fctl to Slack was for another fctl account. Start again from here, signed in as yourself.",
      };
    case "enterprise":
      return {
        tone: "warn",
        text: "fctl doesn't take Enterprise Grid installs for a whole organization yet: add it to one Slack workspace of the organization instead.",
      };
    case "failed":
      return { tone: "error", text: "Slack didn't finish installing fctl's app. Try again in a minute." };
    default:
      return undefined;
  }
}

/** What to tell someone back from linking their Slack account; `confirm` asks rather than tells. */
export function linkOutcome(o: SlackOutcome): Said | undefined {
  switch (o) {
    case "linked":
      return {
        tone: "ok",
        text: "Your Slack account is linked: what you ask fctl there starts sessions as you, on your Claude subscription.",
      };
    case "denied":
      return { tone: "info", text: "You didn't allow the sign-in in Slack, so nothing was linked." };
    case "expired":
      return {
        tone: "info",
        text: "That sign-in took longer than ten minutes, so nothing was linked. Link your account again.",
      };
    case "failed":
      return { tone: "error", text: "Slack didn't finish the sign-in. Try again in a minute." };
    default:
      return undefined;
  }
}

// ───────────────────────────── places and messages ─────────────────────────────

/** Where in Slack, in a few words: "#web-team", "a private channel", "direct messages". */
export function placeName(p: { place: G.SlackPlace; channel_name?: string | null }): string {
  switch (p.place) {
    case "channel":
      return p.channel_name ? `#${p.channel_name}` : "a channel";
    case "private":
      return p.channel_name ? `#${p.channel_name}` : "a private channel";
    case "dm":
      return "direct messages";
    default:
      return "Slack";
  }
}

/** Where a session started, as its badge says it: "#web-team", "Direct message". */
export function sessionPlace(s: G.SessionSlack): string {
  return s.place === "dm" ? "Direct message" : placeName(s);
}

/** Where a session started, in a sentence, for tooltips and screen readers. */
export function sessionPlaceLabel(s: G.SessionSlack): string {
  const team = s.team_name ? ` in ${s.team_name}` : "";
  switch (s.place) {
    case "dm":
      return `Started in a direct message with fctl in Slack${team}`;
    case "private":
      return `Started in the private channel ${placeName(s)} in Slack${team}`;
    default:
      return `Started in ${placeName(s)} in Slack${team}`;
  }
}

/** What someone did in Slack that reached the agent, after their name: "mentioned fctl in #web-team". */
export function messageDid(m: G.SlackMessage): string {
  const where = m.place === "dm" ? "their direct messages with fctl" : placeName(m);
  switch (m.event) {
    case "mentioned":
      return `mentioned fctl in ${where}`;
    case "direct":
      return "wrote to fctl";
    case "replied":
      return m.place === "dm" ? "replied in the thread" : `replied in the thread in ${where}`;
    case "answered":
      return "answered the agent's question";
    case "shortcut":
      return "sent a message to fctl";
    case "command":
      return m.place === "dm" ? "asked with /fctl" : `asked with /fctl in ${where}`;
    default:
      return `wrote in ${where}`;
  }
}

/** For a session that starts: the thread's earlier messages that came with it, as information. */
export function contextWords(n: number | undefined): string | undefined {
  if (!n) return undefined;
  return n === 1
    ? "With 1 earlier message of the thread, as information"
    : `With ${n} earlier messages of the thread, as information`;
}

/**
 * What fctl did in a session's Slack thread for the agent (`slack_*` notices), in a line: "Replied in
 * #web-team", "Read 12 messages in #incidents".
 */
export function noticeLine(code: string, n: G.SlackNotice): string {
  const where = n.place === "dm" ? "the direct messages with fctl" : placeName(n);
  switch (code) {
    case "slack_replied":
      return `Replied in ${where}`;
    case "slack_asked":
      return `Asked in ${where}`;
    case "slack_posted":
      return `Posted in ${where}`;
    case "slack_read": {
      const count = n.messages;
      if (count === undefined) return `Read ${where}`;
      return `Read ${count === 1 ? "1 message" : `${count} messages`} in ${where}`;
    }
    case "slack_failed":
      return `Couldn't post in ${where}`;
    default:
      return `In ${where}`;
  }
}

// ───────────────────────────── a project's events ─────────────────────────────

/** The events a project may post to a channel, in the order its settings list them, as webhooks name them. */
export const SLACK_EVENT_KINDS: { type: string; label: string; phrase: string }[] = [
  {
    type: "session.ready_for_review",
    label: "Pull requests are ready for review",
    phrase: "pull requests are ready for review",
  },
  { type: "session.failed", label: "A session fails", phrase: "a session fails" },
  { type: "session.question", label: "An agent asks something", phrase: "an agent asks something" },
  { type: "session.finished", label: "An agent finishes a turn", phrase: "an agent finishes a turn" },
  { type: "pull_request.opened", label: "An agent opens a pull request", phrase: "an agent opens a pull request" },
  { type: "pull_request.merged", label: "An agent's pull request merges", phrase: "an agent's pull request merges" },
];

/** What a channel hears when nobody chose (`SlackEvents::DEFAULT`): pull requests ready for review, and failures. */
export const DEFAULT_EVENT_KINDS = ["session.ready_for_review", "session.failed"];

/** The events a channel hears: those chosen, or the default ones. */
export const kindsOf = (e: Pick<G.SlackEvents, "kinds"> | undefined): string[] =>
  e?.kinds?.length ? e.kinds : DEFAULT_EVENT_KINDS;

/** "a, b or c". */
function joined(items: readonly string[], word: string): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} ${word} ${items[items.length - 1]}`;
}

/** When a project posts to its channel, in a clause: "pull requests are ready for review or a session fails". */
export function eventsWhen(kinds: readonly string[]): string {
  const known = SLACK_EVENT_KINDS.filter((k) => kinds.includes(k.type)).map((k) => k.phrase);
  if (known.length === 0) return "something happens";
  if (known.length > 3) return `any of ${known.length} kinds of events happen`;
  return joined(known, "or");
}

/** What a project posts where: "Posts to #web-team when pull requests are ready for review or a session fails." */
export function eventsSentence(e: G.SlackEvents): string {
  return `Posts to #${e.channel_name} when ${eventsWhen(kindsOf(e))}.`;
}

/** What the project's Slack page says of it in the list of Settings' pages. */
export function projectSlackGist(s: G.ProjectSettings): string {
  const slack = s.slack;
  const reads = slack?.read_channels ?? true;
  return [
    reads ? "Agents read the readable channels" : "Agents read their thread alone",
    slack?.events ? `events to #${slack.events.channel_name}` : undefined,
  ]
    .filter(Boolean)
    .join(" · ");
}

// ───────────────────────────── a workspace's choices ─────────────────────────────

/** The channels agents read there, as their names with `#`. */
export const readableChannels = (c: Pick<G.SlackConnection, "channels">): G.SlackChannel[] =>
  (c.channels ?? []).filter((ch) => ch.readable && !ch.private);

/** The channels whose mentions start sessions in a project. */
export const mappedChannels = (c: Pick<G.SlackConnection, "channels">): G.SlackChannel[] =>
  (c.channels ?? []).filter((ch) => !!ch.project);

/** A channel's name as a choice shows it, without its `#`, whatever was typed. */
export const bareChannel = (name: string) => name.trim().replace(/^#/, "");

/** Whether something typed looks like Slack's id of a channel (`C0123ABCD`, `G…` for an older private one). */
export const isChannelId = (v: string) => /^[CG][A-Z0-9]{6,}$/.test(v.trim());

/** What a change to a workspace's Slack choices comes to, for its toast. */
export function settingSaved(b: G.UpdateSlackRequest): string {
  if (b.channels !== undefined)
    return b.channels
      ? "Mentions in channels start sessions now"
      : "Mentions, /fctl and Send to fctl… start nothing now";
  if (b.direct_messages !== undefined)
    return b.direct_messages
      ? "Direct messages with fctl start sessions now"
      : "Direct messages with fctl start nothing now";
  if (b.search !== undefined)
    return b.search
      ? "Agents search the readable channels now"
      : "Agents read the readable channels without searching them now";
  if (b.agent_replies !== undefined)
    return b.agent_replies ? "Agents post in their own threads now" : "Only fctl posts in sessions' threads now";
  if (b.notifications !== undefined)
    return b.notifications
      ? "People who choose it get their notifications in Slack now"
      : "Notifications stay out of Slack now";
  if (b.project_events !== undefined)
    return b.project_events ? "Projects post the events they choose to channels now" : "Projects post no events to Slack now";
  if (b.match_email !== undefined)
    return b.match_email
      ? "fctl knows people by their verified address in Slack now"
      : "fctl knows only the people who linked their Slack account now";
  if (b.transcripts !== undefined)
    return b.transcripts === "drop"
      ? "Saved: transcripts drop what Slack's messages said once their session ends"
      : "Saved: transcripts keep what Slack's messages said";
  return "Saved";
}

/** Your Slack accounts, as the list of Settings' pages says it. */
export function slackGist(accounts: G.SlackAccount[] | undefined): { text: string } | undefined {
  if (!accounts) return undefined;
  const [first] = accounts;
  if (!first) return { text: "Not linked" };
  return {
    text: accounts.length === 1 ? `Linked in ${first.team_name}` : `Linked in ${accounts.length} Slack workspaces`,
  };
}

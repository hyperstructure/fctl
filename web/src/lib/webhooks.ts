// Webhooks (docs/architecture.md#webhooks): what a webhook posts, said in a sentence as the CLI says
// it (fctl_domain::hooks::describe, word for word: the tests in both say the same sentences), how one
// stands, the choices its form starts from, and how a receiver checks what fctl posts.

import type * as G from "@/gen";

/** The host of a webhook's address, which it goes by without a name. */
export function hostOf(url: string): string {
  try {
    return new URL(url).hostname || url;
  } catch {
    return url;
  }
}

/** What people call a webhook: its name, else its address's host. */
export const webhookTitle = (w: Pick<G.Webhook, "name" | "url">) => w.name || hostOf(w.url);

/**
 * An event as a sentence's clause, from the label a list shows ("An agent finishes a turn" is "an
 * agent finishes a turn"); fctl's name stays as it is.
 */
export const phraseOf = (label: string) =>
  label.startsWith("fctl ") ? label : label.charAt(0).toLowerCase() + label.slice(1);

/** "a, b or c". */
function joined(items: readonly string[], word: string): string {
  if (items.length <= 1) return items[0] ?? "";
  return `${items.slice(0, -1).join(", ")} ${word} ${items[items.length - 1]}`;
}

const WHO: Record<G.WebhookOrigin, string | undefined> = {
  person: "people",
  schedule: "schedules",
  autopilot: "an autopilot",
  ticket: "tickets",
  service_account: "service accounts",
  other: undefined,
};

/**
 * What a webhook posts, and where, in a sentence: "Posts to hooks.slack.com when pull requests are
 * ready for review or an agent asks something, in Web and Docs, for sessions started by people."
 * `projects` are the names of the projects a workspace's webhook is narrowed to; `catalog` gives the
 * events their order and words.
 */
export function describeWebhook(
  url: string,
  events: readonly string[],
  projects: readonly string[],
  startedBy: readonly G.WebhookOrigin[],
  catalog: readonly G.WebhookEventInfo[],
): string {
  const host = hostOf(url);
  const kinds = catalog.filter((e) => events.includes(e.type));
  let out: string;
  if (kinds.length === 0) out = `Posts every event to ${host}`;
  else if (kinds.length <= 3)
    out = `Posts to ${host} when ${joined(
      kinds.map((k) => phraseOf(k.label)),
      "or",
    )}`;
  else out = `Posts ${kinds.length} kinds of events to ${host}`;
  if (projects.length > 3) out += `, in ${projects.length} projects`;
  else if (projects.length > 0) out += `, in ${joined(projects, "and")}`;
  const sessions = kinds.length === 0 || kinds.some((k) => k.sessions);
  const who = startedBy.map((o) => WHO[o]).filter((w): w is string => !!w);
  if (sessions && who.length > 0) out += `, for sessions started by ${joined(who, "or")}`;
  return `${out}.`;
}

/** How a webhook stands: off, failing, taking what it gets, or with nothing sent yet. */
export type HookState = "off" | "failing" | "ok" | "new";

export function hookState(w: G.Webhook): HookState {
  if (!w.enabled) return "off";
  if (w.health?.failing_since) return "failing";
  if (w.health?.last) return "ok";
  return "new";
}

export const STATE_WORDS: Record<HookState, string> = {
  off: "Off",
  failing: "Failing",
  ok: "On",
  new: "On",
};

/** An answer's status as people know it: "200 OK", "502 Bad Gateway". */
export function statusText(status: number): string {
  const words: Record<number, string> = {
    200: "OK",
    201: "Created",
    202: "Accepted",
    204: "No Content",
    301: "Moved Permanently",
    302: "Found",
    307: "Temporary Redirect",
    308: "Permanent Redirect",
    400: "Bad Request",
    401: "Unauthorized",
    403: "Forbidden",
    404: "Not Found",
    405: "Method Not Allowed",
    408: "Request Timeout",
    410: "Gone",
    413: "Payload Too Large",
    415: "Unsupported Media Type",
    422: "Unprocessable Content",
    429: "Too Many Requests",
    500: "Internal Server Error",
    501: "Not Implemented",
    502: "Bad Gateway",
    503: "Service Unavailable",
    504: "Gateway Timeout",
  };
  return words[status] ? `${status} ${words[status]}` : String(status);
}

/** Whether an attempt got a 2xx answer. */
export const delivered = (a?: G.WebhookAttempt) => !!a?.status && a.status >= 200 && a.status < 300;

/** "200 OK in 182 ms", "No answer within 10 seconds…", what a person's attempt says too. */
export function attemptWords(a: G.WebhookAttempt): string {
  if (a.status !== undefined && delivered(a)) return `${statusText(a.status)} in ${a.ms} ms`;
  if (a.status !== undefined) {
    const text = statusText(a.status);
    // The log's own words for a redirect say more than its status.
    const more = a.error && a.error !== `HTTP ${a.status}` ? `: ${a.error.replace(/^HTTP \d+: /, "")}` : "";
    return `${text} in ${a.ms} ms${more}`;
  }
  return a.error ?? "No attempt yet";
}

/** A ready set of events to start from in the form. */
export interface Preset {
  key: string;
  label: string;
  /** What it's for, in a few words. */
  line: string;
  events: string[];
}

export const PRESETS: Preset[] = [
  {
    key: "people",
    label: "When agents need people",
    line: "Questions, pull requests ready for review, failures",
    events: ["session.ready_for_review", "session.question", "session.failed"],
  },
  {
    key: "progress",
    label: "Sessions as they go",
    line: "Started, finished a turn, stopped, failed",
    events: ["session.started", "session.finished", "session.stopped", "session.failed"],
  },
  {
    key: "pulls",
    label: "Agents' pull requests",
    line: "Opened, merged, closed",
    events: ["pull_request.opened", "pull_request.merged", "pull_request.closed"],
  },
  {
    key: "reviews",
    label: "Reviews",
    line: "Queued, started, posted, failed, cancelled",
    events: ["review.queued", "review.started", "review.finished", "review.failed", "review.cancelled"],
  },
];

/** The preset whose events are exactly `events`, if one is. */
export function presetOf(events: readonly string[]): Preset | undefined {
  return PRESETS.find((p) => p.events.length === events.length && p.events.every((e) => events.includes(e)));
}

/** Sessions start from people, schedules, an autopilot or tickets: how the form names each. */
export const ORIGINS: { value: G.WebhookOrigin; label: string; line: string }[] = [
  { value: "person", label: "People and tools", line: "Started here, with the CLI or an access token" },
  { value: "schedule", label: "Schedules", line: "At their times, on events, or run now" },
  { value: "autopilot", label: "Autopilot", line: "Its own session and those it starts" },
  { value: "ticket", label: "Tickets", line: "Delegated to fctl in Linear" },
];

/**
 * Where an address would be refused, before the server says so: what to change, or nothing.
 * `reach` is where the instance lets webhooks post besides public https:// addresses; the server
 * checks the private networks and local names itself, and says why.
 */
export function urlProblem(url: string, reach?: G.WebhookReach): string | undefined {
  const u = url.trim();
  if (!u) return undefined;
  let parsed: URL;
  try {
    parsed = new URL(u);
  } catch {
    return "That isn't an address yet: it starts with https:// and a host.";
  }
  if (parsed.protocol === "http:" && !reach?.http)
    return "Use https://: fctl posts only over HTTPS, so that nobody on the way reads or changes what it sends.";
  if (parsed.protocol !== "https:" && parsed.protocol !== "http:")
    return reach?.http ? "Use an https:// or http:// address." : "Use an https:// address.";
  if (parsed.username || parsed.password)
    return "Leave the user name and password out: check fctl's signature instead, or put a token in its query.";
  return undefined;
}

/** How many of an instance's private networks the address field's hint names before "and N more". */
const LISTED = 4;

/**
 * What the address field says it takes: an https:// address on the public internet, and what else
 * the instance lets webhooks reach (`reach`, from its settings); for an http:// one, that what fctl
 * posts there travels unencrypted.
 */
export function addressHint(url: string, reach?: G.WebhookReach): string {
  const lead = "fctl posts each event there as JSON, signed with its secret";
  const http = !!reach?.http;
  const nets = reach?.private ?? [];
  if (nets.includes("*")) return `${lead}. This development instance also takes http:// and local addresses.`;
  if (http && url.trim().toLowerCase().startsWith("http://"))
    return "Over http://, anyone on the way can read and change what fctl posts: keep it to your own network.";
  const scheme = http ? "an https:// or http:// address" : "an https:// address";
  if (nets.length === 0) return `${lead}: ${scheme} on the public internet.`;
  const named =
    nets.length <= LISTED
      ? nets.join(", ")
      : `${nets.slice(0, LISTED).join(", ")} and ${nets.length - LISTED} more`;
  return `${lead}: ${scheme} on the public internet, or on this instance's network (${named}).`;
}

/** The languages the secret's examples come in. */
export type SnippetLanguage = "node" | "python" | "any";

/**
 * How a receiver checks that a post is fctl's, in `language`, for a webhook of workspace
 * `workspaceId` (and project `projectId`, a project's own): its signature, made with the secret
 * the receiver keeps in FCTL_WEBHOOK_SECRET, its timestamp, then where it says it comes from.
 */
export function verifySnippet(language: SnippetLanguage, workspaceId: string, projectId?: string): string {
  if (language === "node")
    return `import { createHmac, timingSafeEqual } from "node:crypto";

// fctl checks an address before a webhook posts there: answer that request with
// its challenge, before checking the signature, since a new webhook's secret
// reaches you only once fctl has added it.
export function challengeOf(body) {
  const event = JSON.parse(body);
  return event.type === "url_verification" ? event.challenge : undefined;
}

// \`body\` is the request's body as it came: the raw bytes, not parsed JSON.
export function fromFctl(headers, body) {
  const id = headers["webhook-id"];
  const timestamp = headers["webhook-timestamp"];
  // Older than 5 minutes: it may be a replay.
  if (Math.abs(Date.now() / 1000 - Number(timestamp)) > 300) return false;
  const key = Buffer.from(process.env.FCTL_WEBHOOK_SECRET.slice("whsec_".length), "base64");
  const expected = createHmac("sha256", key).update(\`\${id}.\${timestamp}.\`).update(body).digest();
  const signed = headers["webhook-signature"].split(" ").some((s) => {
    const given = Buffer.from(s.slice(s.indexOf(",") + 1), "base64");
    return given.length === expected.length && timingSafeEqual(given, expected);
  });
  if (!signed) return false;
  const event = JSON.parse(body);
  return event.workspace.id === "${workspaceId}"${projectId ? ` && event.project?.id === "${projectId}"` : ""};
}`;
  if (language === "python")
    return `import base64, hashlib, hmac, json, os, time

# fctl checks an address before a webhook posts there: answer that request with
# its challenge, before checking the signature, since a new webhook's secret
# reaches you only once fctl has added it.
def challenge_of(body: bytes):
    event = json.loads(body)
    return event.get("challenge") if event.get("type") == "url_verification" else None

# \`body\` is the request's body as it came: the raw bytes, not parsed JSON.
def from_fctl(headers, body: bytes) -> bool:
    msg_id = headers["webhook-id"]
    timestamp = headers["webhook-timestamp"]
    # Older than 5 minutes: it may be a replay.
    if abs(time.time() - int(timestamp)) > 300:
        return False
    key = base64.b64decode(os.environ["FCTL_WEBHOOK_SECRET"].removeprefix("whsec_"))
    mac = hmac.new(key, f"{msg_id}.{timestamp}.".encode() + body, hashlib.sha256)
    expected = base64.b64encode(mac.digest()).decode()
    signed = any(
        hmac.compare_digest(s.split(",", 1)[-1], expected)
        for s in headers["webhook-signature"].split()
    )
    if not signed:
        return False
    event = json.loads(body)
    return event["workspace"]["id"] == "${workspaceId}"${projectId ? ` and (event.get("project") or {}).get("id") == "${projectId}"` : ""}`;
  return `0. A request whose type is url_verification is fctl checking the address before a
   webhook posts there: answer it with 200 and its challenge, as text or as
   {"challenge": "…"}, before the steps below (a new webhook's secret reaches you only
   once fctl has added it).
1. Take the secret's bytes: base64-decode what follows whsec_.
2. Sign "<webhook-id>.<webhook-timestamp>.<body>" with HMAC-SHA256 and that key,
   the body byte for byte as it came, and base64-encode the result.
3. It's fctl's when one of the space-separated signatures in webhook-signature,
   after its "v1,", is that, compared in constant time.
4. Refuse a webhook-timestamp more than 5 minutes from your clock: it may be a replay.
5. Check where it says it comes from: workspace.id is ${workspaceId}${projectId ? `,\n   project.id is ${projectId}` : ""}.

Standard Webhooks' libraries do steps 1 to 4: new Webhook(secret).verify(body, headers).`;
}

/** The deliveries a log shows: all of them, or those in one state. */
export type DeliveryFilter = "all" | G.WebhookDeliveryState;

/** When a delivery goes again, in words: "in 4m", "in 2h". */
export function againIn(at: number, now = Date.now()): string {
  const s = Math.max(0, Math.round((at - now) / 1000));
  if (s < 60) return `in ${Math.max(1, s)}s`;
  const m = Math.round(s / 60);
  if (m < 60) return `in ${m}m`;
  const h = Math.floor(m / 60);
  return m % 60 ? `in ${h}h ${m % 60}m` : `in ${h}h`;
}

/** The body of a request or an answer, laid out to read when it's JSON. */
export function pretty(text: string): string {
  try {
    return JSON.stringify(JSON.parse(text), null, 2);
  } catch {
    return text;
  }
}

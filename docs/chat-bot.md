# A Slack bot of your own

At the end of this guide, Acme's people start agents from Slack: Ada
writes `@fctl Fix the login redirect after a password reset` in `#web`,
and a session of the project `web` starts for Ada, on Ada's Claude
subscription. Its thread is where people follow it: the bot posts what
the agent asks, with a button for each answer, the start of each reply,
the pull requests waiting for review and why it failed, and a reply in
the thread reaches the agent from whoever wrote it.

The bot is a program of Acme's own, about 150 lines of JavaScript with
[Bolt](https://docs.slack.dev/tools/bolt-js/), Slack's framework, on one
of Acme's machines. It reaches Slack in Socket Mode, so Slack needs no
address for it, and fctl through a [service account](automation.md#service-accounts),
which starts each session for the person who asked. Setting it up takes
about an hour, and three people's rights: an admin of the fctl workspace
`acme` makes the service account and the webhook, someone Acme's Slack
lets create and install apps installs it, and someone runs it on a
machine that reaches both.

```text
Slack                    the bot                     fctl
@fctl Fix the login… ──▶ who: ada@acme.dev
in #web                  project: web ─────────────▶ starts web#12 for Ada
thread: Started web#12 ◀ keeps web#12's thread
Grace replies ─────────▶ a message, as Grace ──────▶ the agent reads it
buttons: web#12 asks… ◀─ posts what fctl says ◀───── webhook: a question
```

The diagram: a mention in a channel goes to the bot, which looks up who
wrote it and which project the channel is for, then starts a session in
fctl for that person and keeps which thread the session belongs to.
Replies in the thread go to the session as messages; what fctl's webhook
posts about the session comes back to the thread.

## What it looks like

```text
#web
  Ada     @fctl Fix the login redirect after a password reset
    fctl  Started web#12 for @Ada. Reply in this thread to message its agent.
    fctl  web#12 asks: Should API clients keep the old redirect?
          [ Keep it for API clients ]  [ Drop it everywhere ]
    fctl  @Grace answered: Keep it for API clients
    fctl  web#12 finished its turn:
          > I kept the old redirect for API clients and fixed it for the web…
    fctl  web#12 has pull requests ready for review:
          • acme/web#45 Fix the login redirect after a password reset
```

Grace answers with a button, and the agent hears that Grace
answered. Anyone who may message the session in fctl can reply in the
thread, and someone who may not hears why, from fctl, in a message only
they see.

## What you need

- Node.js 22.13 or newer on a machine that reaches Slack and your fctl
  instance, and that fctl's webhooks reach (below).
- An admin of Acme's fctl workspace, for the service account and the
  webhook.
- Someone who can create and install apps in Acme's Slack workspace;
  where Slack's admins approve apps, one of them.
- A project for each channel the bot works in, with the channel's name
  as its slug: `#web` starts sessions in `web`. People start sessions
  only where they're an editor of the project, or more.

## Your values

Set these once, in the shell you run the commands in:

```sh
export FCTL_SERVER=https://fctl.acme.dev
export FCTL_WORKSPACE=acme
export BOT_URL=http://bot.corp.acme.dev:3000   # where fctl's webhook reaches the bot
```

## 1. Make the service account

An admin of `acme`, on the workspace's **Service accounts** tab or with
the CLI:

```sh
fctl service-accounts create "Slack" -w "$FCTL_WORKSPACE" \
  --description "Acme's Slack bot: @fctl in a project's channel starts a session for you"
```

It prints its secret (`fctl_svc_…`) once: keep it for step 4 as
`FCTL_SERVICE_ACCOUNT`. fctl asks each member on their Agent Floor
whether it may start sessions for them, with the description you gave.
On an instance of Acme's own whose people all work for Acme, its admins
can answer for everyone instead:

```sh
fctl admin set --service-account-consent off
```

**Check:** `fctl service-accounts people "Slack" -w "$FCTL_WORKSPACE"`
lists whom it may act for, and why not for the others.

## 2. Create the Slack app

In Slack, open [api.slack.com/apps](https://api.slack.com/apps),
choose **Create New App**, then **From a manifest**, pick Acme's
workspace, and paste:

```yaml
display_information:
  name: fctl
  description: Starts fctl agents for you from a project's channel
features:
  bot_user:
    display_name: fctl
    always_online: true
oauth_config:
  scopes:
    bot:
      - app_mentions:read  # @fctl in a channel
      - channels:history   # replies in the threads of public channels it's in
      - groups:history     # … and of private ones
      - channels:read      # a public channel's name, for its project
      - groups:read        # … and a private one's
      - chat:write         # posting in threads
      - users:read         # who wrote a message
      - users:read.email   # … and their address, which fctl knows them by
settings:
  event_subscriptions:
    bot_events:
      - app_mention
      - message.channels
      - message.groups
  interactivity:
    is_enabled: true
  socket_mode_enabled: true
```

Then:

1. **Install App** installs it in the workspace, and shows the **Bot User
   OAuth Token** (`xoxb-…`): keep it as `SLACK_BOT_TOKEN`.
2. Under **Basic Information → App-Level Tokens**, **Generate Token and
   Scopes** with the scope `connections:write`: keep the token
   (`xapp-…`) as `SLACK_APP_TOKEN`. It's what Socket Mode connects with.

**Check:** the app shows under **Apps** in Slack's sidebar, as `fctl`.

## 3. Write the bot

In a directory of its own:

```sh
npm init -y && npm pkg set type=module
npm install @slack/bolt standardwebhooks
```

`bot.js`:

```js
import { createServer } from "node:http";
import { DatabaseSync } from "node:sqlite";
import { App } from "@slack/bolt";
import { Webhook } from "standardwebhooks";

const FCTL = process.env.FCTL_SERVER;
const app = new App({
  token: process.env.SLACK_BOT_TOKEN,
  appToken: process.env.SLACK_APP_TOKEN,
  socketMode: true,
});

// Which thread each session is in, kept across restarts.
const db = new DatabaseSync(process.env.BOT_DB ?? "bot.db");
db.exec(`CREATE TABLE IF NOT EXISTS threads (
  session TEXT PRIMARY KEY, channel TEXT NOT NULL, ts TEXT NOT NULL, UNIQUE (channel, ts))`);
const remember = db.prepare("INSERT INTO threads (session, channel, ts) VALUES (?, ?, ?)");
const sessionIn = db.prepare("SELECT session FROM threads WHERE channel = ? AND ts = ?");
const threadOf = db.prepare("SELECT channel, ts FROM threads WHERE session = ?");

// A request to fctl for the person it acts for. A refusal throws fctl's own
// words for them: not allowed yet, not an editor of the project, no such project.
async function fctl(path, person, body) {
  const res = await fetch(`${FCTL}/api/v1${path}`, {
    method: "POST",
    headers: {
      Authorization: `Bearer ${process.env.FCTL_SERVICE_ACCOUNT}`,
      "Fctl-On-Behalf-Of": person,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
  });
  const answer = await res.json();
  if (!res.ok) throw new Error(answer.message);
  return answer;
}

// The address fctl knows a Slack user by: the one on their Slack profile.
async function addressOf(client, user) {
  const { user: u } = await client.users.info({ user });
  if (!u.profile?.email) throw new Error("Slack doesn't give this app your email address, so fctl can't tell who you are.");
  return u.profile.email;
}

// Slack escapes &, < and > in what people write, and fctl's text needs them escaped.
const esc = (text) => text.replace(/&/g, "&amp;").replace(/</g, "&lt;").replace(/>/g, "&gt;");
const unesc = (text) => text.replace(/&lt;/g, "<").replace(/&gt;/g, ">").replace(/&amp;/g, "&");

// What went wrong, to the person alone, in the thread.
const tell = (client, channel, user, thread_ts, text) =>
  client.chat.postEphemeral({ channel, user, thread_ts, text });

// "@fctl …" starts a session in the channel's project; in a session's
// thread, it's a message to that session.
app.event("app_mention", async ({ event, client }) => {
  const thread = event.thread_ts ?? event.ts;
  const text = unesc(event.text.replace(/<@[A-Z0-9]+>/g, "").trim());
  try {
    const person = await addressOf(client, event.user);
    const known = sessionIn.get(event.channel, thread);
    if (known) {
      await fctl(`/sessions/${known.session}/messages`, person, { text, client_id: `slack:${event.ts}` });
      return;
    }
    const { channel } = await client.conversations.info({ channel: event.channel });
    const session = await fctl(`/projects/${channel.name}/sessions`, person, { prompt: text });
    remember.run(session.id, event.channel, thread);
    await client.chat.postMessage({
      channel: event.channel,
      thread_ts: thread,
      unfurl_links: false,
      text: `Started <${FCTL}/s/${session.id}|${channel.name}#${session.number}> for <@${event.user}>. Reply in this thread to message its agent.`,
    });
  } catch (e) {
    await tell(client, event.channel, event.user, thread, e.message);
  }
});

// A reply in a session's thread is a message to its agent, from whoever wrote it.
const REPLIES = [undefined, "thread_broadcast", "file_share"]; // not edits, deletions or joins
app.message(async ({ message, client, context }) => {
  if (!message.thread_ts || !REPLIES.includes(message.subtype) || message.bot_id) return;
  if (message.text?.includes(`<@${context.botUserId}>`)) return; // app_mention has it
  const known = sessionIn.get(message.channel, message.thread_ts);
  if (!known) return;
  try {
    const person = await addressOf(client, message.user);
    await fctl(`/sessions/${known.session}/messages`, person, {
      text: unesc(message.text ?? ""),
      client_id: `slack:${message.ts}`,
    });
  } catch (e) {
    await tell(client, message.channel, message.user, message.thread_ts, e.message);
  }
});

// A click on one of a question's answers sends it, from whoever clicked.
// The first click answers: later ones carry the same client_id.
app.action(/^answer:/, async ({ ack, body, action, client }) => {
  await ack();
  const { session, answer } = JSON.parse(action.value);
  try {
    const person = await addressOf(client, body.user.id);
    await fctl(`/sessions/${session}/messages`, person, { text: answer, client_id: `slack:${body.message.ts}` });
    const answered = `<@${body.user.id}> answered: ${esc(answer)}`;
    await client.chat.update({
      channel: body.channel.id,
      ts: body.message.ts,
      text: answered,
      blocks: [body.message.blocks[0], { type: "context", elements: [{ type: "mrkdwn", text: answered }] }],
    });
  } catch (e) {
    await tell(client, body.channel.id, body.user.id, body.message.thread_ts, e.message);
  }
});

// What fctl's webhook says of a session, as a message in its thread.
function post(event) {
  const { session } = event.data;
  const name = `<${session.url}|${session.ref}>`;
  switch (event.type) {
    case "session.finished": {
      const reply = esc((event.data.message ?? "").slice(0, 600)).replaceAll("\n", "\n>");
      return { text: `${name} finished its turn:\n>${reply}` };
    }
    case "session.question": {
      const { text, options = [] } = session.question;
      const asks = { type: "section", text: { type: "mrkdwn", text: `${name} asks: ${esc(text)}` } };
      if (!options.length) return { text: `${session.ref} asks: ${text}`, blocks: [asks] };
      const buttons = options.map((option, i) => ({
        type: "button",
        action_id: `answer:${i}`,
        text: { type: "plain_text", text: option.slice(0, 75) },
        value: JSON.stringify({ session: session.id, answer: option }),
      }));
      return { text: `${session.ref} asks: ${text}`, blocks: [asks, { type: "actions", elements: buttons }] };
    }
    case "session.ready_for_review": {
      const pulls = event.data.ready.map((p) => `• <${p.url}|${p.repo}#${p.number}> ${esc(p.title ?? "")}`);
      return { text: `${name} has pull requests ready for review:\n${pulls.join("\n")}` };
    }
    case "session.failed":
      return { text: `${name} failed: ${esc(session.status_reason ?? "")}` };
  }
}

// fctl's webhook: checked, answered at once, then posted to the thread.
async function onWebhook(req, res) {
  const chunks = [];
  for await (const chunk of req) chunks.push(chunk);
  const body = Buffer.concat(chunks).toString("utf8");
  let event;
  try {
    const sent = JSON.parse(body);
    if (sent.type === "url_verification") return res.end(sent.challenge); // fctl checking the address
    event = new Webhook(process.env.FCTL_WEBHOOK_SECRET).verify(body, req.headers);
  } catch {
    res.statusCode = 400;
    return res.end();
  }
  res.statusCode = 204;
  res.end();
  const thread = threadOf.get(event.data?.session?.id ?? "");
  const message = thread && post(event);
  if (!message) return; // a session the bot didn't start, or an event it doesn't post
  await app.client.chat.postMessage({ channel: thread.channel, thread_ts: thread.ts, unfurl_links: false, ...message });
}

const port = Number(process.env.PORT ?? 3000);
createServer((req, res) =>
  onWebhook(req, res).catch((e) => {
    app.logger.error(`fctl's webhook: ${e.message}`);
    if (!res.headersSent) res.statusCode = 500;
    if (!res.writableEnded) res.end();
  }),
).listen(port);

await app.start();
console.log(`The bot is connected to Slack, and takes fctl's webhook on port ${port}`);
```

## 4. Run it

On the machine at `BOT_URL`, with the tokens from steps 1 and 2:

```sh
export SLACK_BOT_TOKEN=xoxb-… SLACK_APP_TOKEN=xapp-…
export FCTL_SERVER=https://fctl.acme.dev FCTL_SERVICE_ACCOUNT=fctl_svc_…
node bot.js
```

Keep it running as a service: a systemd unit, a container with
`bot.db` on a volume, or your platform's way. The webhook's secret comes
in the next step: until then, the bot answers fctl's challenge and turns
everything else away.

**Check:** it prints `The bot is connected to Slack`, and `@fctl` in a
channel the bot isn't in yet offers to invite it.

## 5. Have fctl tell the bot

The bot posts in a thread what fctl's webhook tells it. An admin of
`acme` adds the webhook, for the sessions service accounts start, with
the four events the bot posts:

```sh
fctl webhooks add "$BOT_URL" -w "$FCTL_WORKSPACE" --name "Slack bot" \
  --events session.finished,session.question,session.ready_for_review,session.failed \
  --started-by service-accounts
```

fctl posts the bot a challenge, which it answers, and then prints the
webhook's secret (`whsec_…`) once. Start the bot again with it:

```sh
export FCTL_WEBHOOK_SECRET=whsec_…
node bot.js
```

A webhook posts to `https://` addresses on the public internet. To
reach a bot on Acme's own network, or over plain `http://` as here, an
instance of Acme's own lists the bot's address in
`FCTL_WEBHOOK_PRIVATE_HOSTS`, with `FCTL_WEBHOOK_HTTP=1`
([Webhooks to your own network](self-hosting.md#webhooks-to-your-own-network)).
On app.fctl.sh, give the bot a public `https://` address.

**Check:** `fctl webhooks test "Slack bot" -w "$FCTL_WORKSPACE" --event
session.question` answers `204`. A test names a made-up session, so the
bot posts nothing for it.

## 6. Try it

Invite the bot to `#web` (`/invite @fctl`), then:

```text
@fctl Fix the login redirect after a password reset
```

**Check:** within seconds the bot answers in a thread, `Started web#12
for @Ada`, with a link to the session in fctl, where Ada follows it too.
Within a minute or two, the agent's first reply or question shows in the
thread.

## Other ways

- **One channel for every project?** Name the project in the message
  instead (`@fctl web: Fix the login redirect`), and take it from the
  text rather than from `conversations.info`.
- **Files?** The agent gets only what the bot sends it. To send a file
  someone attached in Slack, download it with the bot token (it needs the
  `files:read` scope), upload it to `POST /api/v1/projects/{project}/attachments`
  or `…/sessions/{id}/attachments`, and pass the ids in `attachments`
  ([The HTTP API](api.md#files)).
- **The whole thread as the task?** The agent reads only the message
  that mentions the bot. To start from a discussion, have the bot read
  the thread (`conversations.replies`) and send it, marked as what people
  said: `{"prompt": "…", "attachments": […]}` with the thread in a file,
  say.
- **Microsoft Teams?** See [below](#microsoft-teams).

## What goes wrong

The bot passes on fctl's refusals as fctl words them, to the person
alone:

| The person sees | Why | What to do |
|---|---|---|
| Ada hasn't allowed Slack to start sessions for them yet: fctl asks them when they next open it. | fctl asks each member first | Ada allows it on the Agent Floor, or under the workspace's **Service accounts** tab |
| Nobody by that id or address is a member of Acme. | Ada's Slack address isn't one of Ada's addresses in fctl | Ada adds it under **Settings → Email addresses** |
| Project not found | The channel's name isn't a project's slug, or Ada can't see it | Rename the channel, or map it to the project in the bot |
| Slack doesn't give this app your email address… | The app lacks `users:read.email`, or the user is another workspace's | Add the scope and reinstall the app |

And the bot's log:

- **`An API error occurred: not_in_channel`** as it posts: invite the bot
  to the channel.
- **`invalid_auth`** or **`not_authed`** at start: a token is wrong, or
  `SLACK_APP_TOKEN` lacks `connections:write`.
- **fctl says the address didn't answer its challenge** as you add the
  webhook: the bot isn't running, or fctl can't reach `BOT_URL`. The
  message under the address in fctl says which.
- **Nothing comes back to the thread**: the webhook's page in fctl
  (`fctl webhooks deliveries "Slack bot" -w acme --failed`) lists what
  went out and what the bot answered. A `400` is a signature that didn't
  check out: the bot runs without the webhook's current secret.

## Limits

- **The agent reads what the bot sends**, and nothing else of Slack: the
  message that started it and the replies in its thread. It can't read
  the channel or other threads. A Slack app of fctl's own, whose agents
  read the thread they were asked from, is on the roadmap.
- **Each person needs an fctl account**, in the workspace, with their
  Slack address among their addresses, and a Claude subscription
  connected: sessions run on theirs.
- **Webhooks need an address fctl reaches**: public `https://`, unless an
  instance of your own lets them reach your network.
- **Slack shows the bot what it's in**: every message in the channels
  it's invited to, the private ones too. The bot passes on only those in
  its sessions' threads.
- **One bot at a time.** Slack hands each event to one of an app's
  connections, and the bot keeps its threads in a file of its own: two
  copies would each miss what the other got.

## Microsoft Teams

A Teams bot works the same way, with the same service account and
webhook: it finds the person's address, starts the session on their
behalf, keeps the conversation the session belongs to, and posts there
what the webhook says, with an Adaptive Card's buttons for a question's
answers. Build it with Microsoft's
[Teams SDK](https://microsoft.github.io/teams-sdk/) (`@microsoft/teams.apps`).
The differences:

- **Teams posts to the bot.** It delivers messages to an `https://`
  address of the bot's that the internet reaches (`/api/messages`),
  registered for the bot in Teams' Developer Portal or with an Azure Bot
  resource. The Teams SDK's Socket Mode is for trying a bot out, not for
  running one.
- **The bot hears only what mentions it** in a channel, replies in its
  sessions' threads included, unless its app manifest asks for the
  `ChannelMessage.Read.Group` permission, which lets it read every
  message of the teams that install it.
- **The person's address** is their Microsoft Entra ID account's:
  `getMemberById` for the activity's sender gives its `email` and
  `userPrincipalName`. One of them has to be among their addresses in
  fctl.

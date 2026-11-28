// A stand-in for Slack, which `cargo xtask dev` runs and points the control
// plane's Slack app at (SLACK_URL, docs/development.md): Slack's install and
// sign-in pages and its Web API, for a made-up Slack workspace, Acme, with
// fctl's bot in it; and what Slack sends fctl's app, signed with the app's
// signing secret (events, interactions, /fctl), which its /standin/
// endpoints send as people in Acme mention fctl, write to it, reply, press
// buttons, run /fctl, use its shortcuts, open its Home tab and press stop.
//
//   node standin.mjs [--port 8791] [--state DIR] [--events URL]
//                    [--interactivity URL] [--commands URL]
//
// fctl's Slack app comes from the environment, as the control plane has it:
// SLACK_CLIENT_ID, SLACK_CLIENT_SECRET and SLACK_SIGNING_SECRET. --events,
// --interactivity and --commands are fctl's addresses
// (http://127.0.0.1:8787/api/v1/slack/events and the rest); --state keeps
// Acme, and a log of every request, in DIR.

import { createStandin } from "./lib/server.mjs";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};
const port = Number(opt("--port", "8791"));
const fctl = "http://127.0.0.1:8787/api/v1/slack";
const config = {
  clientId: process.env.SLACK_CLIENT_ID,
  clientSecret: process.env.SLACK_CLIENT_SECRET,
  signingSecret: process.env.SLACK_SIGNING_SECRET,
  events: opt("--events", `${fctl}/events`),
  interactivity: opt("--interactivity", `${fctl}/interactivity`),
  commands: opt("--commands", `${fctl}/commands`),
  self: `http://127.0.0.1:${port}`,
};
for (const [name, value] of Object.entries({
  SLACK_CLIENT_ID: config.clientId,
  SLACK_CLIENT_SECRET: config.clientSecret,
  SLACK_SIGNING_SECRET: config.signingSecret,
})) {
  if (!value) {
    console.error(
      `[slack] ${name} is missing: the stand-in signs fctl's app in, and signs what it sends, with the control plane's`,
    );
    process.exit(2);
  }
}

const { server } = createStandin({ state: opt("--state"), config });
server.listen(port, "127.0.0.1", () => {
  console.log(
    `[slack] Slack stand-in at http://127.0.0.1:${port}: Acme, with fctl's bot, Dev Owner, Grace Hopper and Ada Guest`,
  );
});
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close();
    process.exit(0);
  });
}

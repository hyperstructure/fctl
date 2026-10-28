// A stand-in for Linear, which `cargo xtask dev` runs and points the
// control plane at (docs/development.md): Linear's sign-in and GraphQL API,
// answered on Linear's own schema so that what Linear wouldn't take fails
// here too, for a made-up Linear workspace, Acme; and the webhook
// deliveries Linear signs, which its /standin/ endpoints send to fctl as
// people in Linear delegate issues, mention fctl, reply and press stop.
//
//   node standin.mjs [--port 8790] [--state DIR] [--webhook URL]
//
// fctl's Linear app comes from the environment, as the control plane has
// it: LINEAR_CLIENT_ID, LINEAR_CLIENT_SECRET and LINEAR_WEBHOOK_SECRET.
// --webhook is fctl's webhook (http://127.0.0.1:8787/api/v1/linear/webhook);
// --state keeps Acme, and a log of every request, in DIR.

import { loadSchema } from "./lib/graphql.mjs";
import { createStandin } from "./lib/server.mjs";

const args = process.argv.slice(2);
const opt = (name, fallback) => {
  const i = args.indexOf(name);
  return i >= 0 && args[i + 1] ? args[i + 1] : fallback;
};
const port = Number(opt("--port", "8790"));
const config = {
  clientId: process.env.LINEAR_CLIENT_ID,
  clientSecret: process.env.LINEAR_CLIENT_SECRET,
  webhookSecret: process.env.LINEAR_WEBHOOK_SECRET,
  webhook: opt("--webhook", "http://127.0.0.1:8787/api/v1/linear/webhook"),
};
for (const [name, value] of Object.entries({
  LINEAR_CLIENT_ID: config.clientId,
  LINEAR_CLIENT_SECRET: config.clientSecret,
  LINEAR_WEBHOOK_SECRET: config.webhookSecret,
})) {
  if (!value) {
    console.error(`[linear] ${name} is missing: the stand-in signs fctl's app in with the control plane's`);
    process.exit(2);
  }
}

const { server } = createStandin({ schema: loadSchema(), state: opt("--state"), config });
server.listen(port, "127.0.0.1", () => {
  console.log(
    `[linear] Linear stand-in at http://127.0.0.1:${port}: Acme, with fctl's app, Dev Owner and Grace Hopper`,
  );
});
for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => {
    server.close();
    process.exit(0);
  });
}

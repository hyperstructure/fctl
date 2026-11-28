// Checks the Block Kit fctl sends Slack against Slack's limits, as the
// stand-in takes it, without running anything: each message or view, with
// the arguments fctl sends, from stdin as JSON
// ({ "payloads": [{ "name", "method", "args" }] }, `method` one of
// chat.postMessage, chat.update, chat.postEphemeral, chat.stopStream,
// views.open, views.publish or response_url), as `cargo xtask slack check`
// would write them from fctl_domain::slack::blocks. Says what Slack would
// refuse, and fails when it would refuse anything.

import { readFileSync } from "node:fs";
import { blockProblems, messages, viewProblems } from "./lib/blocks.mjs";

/** What Slack would refuse of one payload, in its words. */
function refused({ method, args = {} }) {
  switch (method) {
    case "chat.postMessage":
    case "chat.update":
    case "chat.postEphemeral":
    case "chat.stopStream":
    case "response_url": {
      const found = [];
      const blocks = typeof args.blocks === "string" ? JSON.parse(args.blocks) : args.blocks;
      if (blocks !== undefined) found.push(...messages(blockProblems(blocks, "message")));
      if (args.markdown_text !== undefined && (args.text !== undefined || blocks !== undefined)) {
        found.push("markdown_text_conflict: markdown_text goes alone");
      }
      const empty = !args.text && !args.markdown_text && !(Array.isArray(blocks) && blocks.length);
      if (method !== "chat.stopStream" && empty) found.push("no_text: a message needs text or blocks");
      return found;
    }
    case "views.open":
    case "views.publish": {
      const view = typeof args.view === "string" ? JSON.parse(args.view) : args.view;
      const want = method === "views.open" ? "modal" : "home";
      if (view?.type !== want) return [`invalid_arguments: ${method} takes a ${want} view`];
      return messages(viewProblems(view));
    }
    default:
      return [`the check doesn't know ${method}: add it to xtask/slack/check.mjs`];
  }
}

const { payloads } = JSON.parse(readFileSync(0, "utf8"));
let failed = 0;
for (const p of payloads) {
  const found = refused(p);
  if (found.length) failed += 1;
  for (const f of found) console.error(`${p.name} (${p.method}): ${f}`);
}
if (failed) {
  console.error(
    `Slack refuses ${failed} of the ${payloads.length} messages and views fctl sends (xtask/slack/check.mjs)`,
  );
  process.exit(1);
}
console.log(`Slack takes all ${payloads.length} of the messages and views fctl sends`);

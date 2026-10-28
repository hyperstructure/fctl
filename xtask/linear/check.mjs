// Checks the GraphQL fctl sends Linear against Linear's schema, as the
// stand-in takes it, without running anything: each operation, with
// variables of the kinds fctl sends, from stdin as JSON
// ({ "operations": [{ "name", "query", "variables" }] }), as
// `cargo xtask linear check` writes them from fctl_domain::linear.
// Says what Linear would refuse, and fails when it would refuse anything.

import { readFileSync } from "node:fs";
import { check, loadSchema } from "./lib/graphql.mjs";

const { operations } = JSON.parse(readFileSync(0, "utf8"));
const { problems, deprecated } = await check(loadSchema(), operations);
for (const d of deprecated) console.error(`${d.name} uses what Linear deprecated: ${d.message}`);
for (const p of problems) console.error(`${p.name}: ${p.message}`);
if (problems.length) {
  console.error(
    `Linear's schema refuses ${problems.length} of what fctl sends (xtask/linear/schema.graphql)`,
  );
  process.exit(1);
}
const names = new Set(operations.map((o) => o.name));
console.log(
  `Linear's schema takes all ${names.size} of fctl's queries and mutations, in ${operations.length} forms`,
);

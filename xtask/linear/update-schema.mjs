// Brings schema.graphql up to date with Linear's GraphQL schema, as Linear
// publishes it in its SDK (linear/linear, packages/sdk/src/schema.graphql),
// and says which commit it came from in the file's first lines.
//
//   npm run update-schema --workspace xtask/linear

import { writeFileSync } from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { buildSchema } from "graphql";

const here = path.dirname(fileURLToPath(import.meta.url));
const file = "packages/sdk/src/schema.graphql";

const commits = await fetch(`https://api.github.com/repos/linear/linear/commits?path=${file}&per_page=1`, {
  headers: { Accept: "application/vnd.github+json", "User-Agent": "fctl-linear-standin" },
});
if (!commits.ok) {
  throw new Error(`GitHub answered ${commits.status} for linear/linear's commits`);
}
const [{ sha }] = await commits.json();
// By its commit, which raw.githubusercontent.com doesn't answer from a stale cache.
const res = await fetch(`https://raw.githubusercontent.com/linear/linear/${sha}/${file}`);
if (!res.ok) {
  throw new Error(`GitHub answered ${res.status} for ${file} at ${sha}`);
}
const sdl = await res.text();
buildSchema(sdl, { assumeValidSDL: true });
writeFileSync(
  path.join(here, "schema.graphql"),
  `# Linear's GraphQL schema, from linear/linear at ${sha}\n# (${file}). npm run update-schema --workspace xtask/linear updates it.\n\n${sdl}`,
);
console.log(`schema.graphql: linear/linear at ${sha}`);

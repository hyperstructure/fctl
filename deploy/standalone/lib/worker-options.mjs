// Prints, as JSON, the Miniflare options wrangler derives for the Worker
// from a wrangler config (its D1 database, Durable Objects, assets, rate
// limits): `node worker-options.mjs <wrangler.toml>`. The server runs it as
// a process of its own, since wrangler is large and it needs it only for
// this and the migrations.

// wrangler may log; stdout carries the JSON.
console.log = console.error;
console.info = console.error;
const { unstable_getMiniflareWorkerOptions } = await import("wrangler");
const { workerOptions } = unstable_getMiniflareWorkerOptions(process.argv[2]);
process.stdout.write(JSON.stringify(workerOptions));

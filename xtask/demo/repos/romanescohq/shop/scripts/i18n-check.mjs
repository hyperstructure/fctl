// Fails when a locale lacks a key another one has.
import { readdirSync, readFileSync } from "node:fs";
const files = readdirSync("messages").filter((f) => f.endsWith(".json"));
const keys = files.map((f) => new Set(Object.keys(JSON.parse(readFileSync(`messages/${f}`, "utf8")))));
const all = new Set(keys.flatMap((k) => [...k]));
const missing = files.flatMap((f, i) => [...all].filter((k) => !keys[i].has(k)).map((k) => `${f}: ${k}`));
if (missing.length) { console.error(missing.join("\n")); process.exit(1); }
console.log(`✓ ${all.size} keys in ${files.map((f) => f.replace(".json", "")).join(", ")}`);

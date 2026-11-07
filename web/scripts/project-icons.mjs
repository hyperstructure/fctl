// Adds IconMCP's icons to projects' icon set (src/lib/project-icon-data.ts), each in outline, solid
// and duotone as its MCP server's get_icon draws them at 16 px, with its label, set and tags.
//
//   node scripts/project-icons.mjs [--after=NAME] GROUP:NAME[:TAG,TAG…] …
//
// GROUP is where the picker shows the icon: Build, Business or Life. TAGs join IconMCP's, so that a
// search finds the icon by them too. New icons go after --after's icon, in the order given (each
// --after starts again), or else at the end of their group. An icon the set has is drawn again in
// its place, with the tags given. IconMCP answers about 100 calls in a row, then asks to wait.

import { readFileSync, writeFileSync } from "node:fs";

const FILE = new URL("../src/lib/project-icon-data.ts", import.meta.url);
const MARK = "export const ICONS: IconDef[] = ";
const GROUPS = ["Build", "Business", "Life"];
const STYLES = ["outline", "solid", "duotone"];
// Attributes the glyphs keep as numbers; the rest stay strings.
const NUMBERS = new Set(["x", "y", "width", "height", "rx", "ry", "cx", "cy", "r", "x1", "y1", "x2", "y2"]);

async function call(tool, args) {
  for (;;) {
    const res = await fetch("https://iconmcp.com/mcp", {
      method: "POST",
      // IconMCP refuses some clients' default user agents.
      headers: {
        "content-type": "application/json",
        accept: "application/json, text/event-stream",
        "user-agent": "fctl-project-icons",
      },
      body: JSON.stringify({
        jsonrpc: "2.0",
        id: 1,
        method: "tools/call",
        params: { name: tool, arguments: args },
      }),
    });
    if (res.status === 429) {
      const wait = Number(res.headers.get("retry-after") ?? 60);
      console.error(`IconMCP asks to wait ${wait} s`);
      await new Promise((done) => setTimeout(done, wait * 1000));
      continue;
    }
    if (!res.ok) throw new Error(`${tool}: HTTP ${res.status}`);
    const body = await res.json();
    const texts = (body.result?.content ?? []).map((c) => c.text);
    if (body.error || body.result?.isError)
      throw new Error(`${tool} ${JSON.stringify(args)}: ${texts.join(" ")}`);
    return texts;
  }
}

/** An SVG's elements as [tag, attrs, ...children], without the <svg> and the <g> around them. */
function parse(svg, name) {
  const root = { kids: [] };
  const stack = [root];
  for (const [, close, tag, attrs, selfClose] of svg.matchAll(/<(\/?)([a-zA-Z]+)([^>]*?)(\/?)>/g)) {
    if (close) {
      stack.pop();
      continue;
    }
    const a = {};
    for (const [, key, value] of attrs.matchAll(/([a-zA-Z][a-zA-Z0-9:-]*)="([^"]*)"/g)) {
      const prop = key.replace(/-([a-z])/g, (_, c) => c.toUpperCase());
      let v = value.replaceAll(`iconmcp-${name}-m`, "$m");
      // Duotone's tint is the text's color at its opacity; a mask's white and black stay.
      if ((prop === "fill" || prop === "stroke") && v.startsWith("#") && !/^#(fff|000)$/i.test(v))
        v = "currentColor";
      a[prop] = NUMBERS.has(prop) || prop === "fillOpacity" || prop === "strokeWidth" ? Number(v) : v;
    }
    const el = { tag, attrs: a, kids: [] };
    stack.at(-1).kids.push(el);
    if (!selfClose) stack.push(el);
  }
  const [svgEl] = root.kids;
  if (svgEl?.tag !== "svg") throw new Error(`${name}: no <svg>`);
  // A <g> without attributes only groups: its children take its place.
  const flat = (els) =>
    els.flatMap((e) => (e.tag === "g" && !Object.keys(e.attrs).length ? flat(e.kids) : [e]));
  const tuple = (e) => [e.tag, e.attrs, ...flat(e.kids).map(tuple)];
  return flat(svgEl.kids).map(tuple);
}

async function icon(name) {
  const def = {};
  for (const style of STYLES) {
    const [svg, meta] = await call("get_icon", { name, style, size: 16 });
    const m = JSON.parse(meta);
    Object.assign(def, { label: m.label, set: m.set, tags: m.tags });
    def[style] = parse(svg, name);
  }
  return def;
}

const source = readFileSync(FILE, "utf8");
const at = source.indexOf(MARK);
const icons = JSON.parse(
  source
    .slice(at + MARK.length)
    .trim()
    .replace(/;$/, ""),
);

let after;
let added = 0;
for (const arg of process.argv.slice(2)) {
  if (arg.startsWith("--after=")) {
    after = arg.slice("--after=".length);
    if (!icons.some((i) => i.name === after)) throw new Error(`${arg}: no icon named ${after}`);
    continue;
  }
  const [group, name, tags = ""] = arg.split(":");
  if (!GROUPS.includes(group) || !name)
    throw new Error(`${arg}: not GROUP:NAME[:TAGS], GROUP one of ${GROUPS}`);
  const drawn = await icon(name);
  const extra = tags.split(",").filter((t) => t && !drawn.tags.includes(t));
  const def = {
    name,
    label: drawn.label,
    group,
    set: drawn.set,
    tags: [...drawn.tags, ...extra],
    outline: drawn.outline,
    solid: drawn.solid,
    duotone: drawn.duotone,
  };
  const old = icons.findIndex((i) => i.name === name);
  if (old >= 0) {
    icons[old] = def;
    console.log(`drew ${name} again`);
    continue;
  }
  let place = icons.findLastIndex((i) => i.group === group) + 1;
  if (after) place = icons.findIndex((i) => i.name === after) + 1;
  icons.splice(place, 0, def);
  if (after) after = name;
  added++;
  console.log(`added ${name} (${drawn.label})`);
}

writeFileSync(FILE, `${source.slice(0, at + MARK.length)}${JSON.stringify(icons)};\n`);
console.log(`${added} added, ${icons.length} icons`);

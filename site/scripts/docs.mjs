// Renders the repository's docs/*.md into dist/docs/<slug>/index.html, using
// dist/docs.html (built by Vite, so it links the hashed CSS and JS) as the
// template. Code is highlighted at build time; pages ship no Markdown parser.

import { mkdir, readFile, rm, writeFile } from "node:fs/promises";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { Marked } from "marked";
import { createHighlighter } from "shiki";

const here = dirname(fileURLToPath(import.meta.url));
const root = join(here, "..", "..");
const dist = join(here, "..", "dist");
const REPO = "https://github.com/hyperstructure/fctl";

/** The docs, in navigation order. `slug` "" is /docs/. */
const PAGES = [
  { slug: "", file: "docs/README.md", nav: "Overview" },
  { slug: "getting-started", file: "docs/getting-started.md", nav: "Getting started" },
  { slug: "cli", file: "docs/cli.md", nav: "The CLI" },
  { slug: "self-hosting", file: "docs/self-hosting.md", nav: "Self-hosting" },
  { slug: "architecture", file: "docs/architecture.md", nav: "Architecture" },
  { slug: "security", file: "docs/security.md", nav: "Security" },
  { slug: "development", file: "docs/development.md", nav: "Development" },
];

const LANGS = ["sh", "bash", "toml", "json", "rust", "ts", "tsx", "sql", "yaml", "text"];

const slugify = (s) =>
  s
    .toLowerCase()
    .replace(/<[^>]+>/g, "")
    .replace(/&[a-z]+;/g, "")
    .replace(/[^a-z0-9]+/g, "-")
    .replace(/^-|-$/g, "");

const esc = (s) =>
  s.replace(/[&<>"]/g, (c) => ({ "&": "&amp;", "<": "&lt;", ">": "&gt;", '"': "&quot;" })[c]);

/** Links between docs become site URLs; other repository files point at GitHub. */
function rewriteHref(href, fromFile) {
  if (/^([a-z]+:|#|\/)/i.test(href)) return href;
  const [path, hash] = href.split("#");
  const resolved = join(dirname(fromFile), path).replaceAll("\\", "/");
  const page = PAGES.find((p) => p.file === resolved);
  if (page) return `/docs/${page.slug ? `${page.slug}/` : ""}${hash ? `#${hash}` : ""}`;
  return `${REPO}/blob/main/${resolved}${hash ? `#${hash}` : ""}`;
}

async function main() {
  const template = await readFile(join(dist, "docs.html"), "utf8");
  const highlighter = await createHighlighter({
    themes: ["github-dark-default", "github-light-default"],
    langs: LANGS,
  });

  for (const page of PAGES) {
    const source = await readFile(join(root, page.file), "utf8");
    let title = page.nav;
    let description = "";
    const marked = new Marked({
      gfm: true,
      renderer: {
        heading({ tokens, depth }) {
          const text = this.parser.parseInline(tokens);
          if (depth === 1) {
            title = text.replace(/<[^>]+>/g, "");
            return `<h1>${text}</h1>\n`;
          }
          const id = slugify(text);
          return `<h${depth} id="${id}"><a href="#${id}" style="color:inherit;text-decoration:none">${text}</a></h${depth}>\n`;
        },
        paragraph({ tokens }) {
          const html = this.parser.parseInline(tokens);
          if (!description) description = html.replace(/<[^>]+>/g, "").slice(0, 200);
          return `<p>${html}</p>\n`;
        },
        link({ href, title: t, tokens }) {
          const text = this.parser.parseInline(tokens);
          const url = rewriteHref(href, page.file);
          const external = /^https?:/.test(url) && !url.startsWith("https://fctl.sh");
          return `<a href="${esc(url)}"${t ? ` title="${esc(t)}"` : ""}${external ? ' rel="noreferrer"' : ""}>${text}</a>`;
        },
        code({ text, lang }) {
          const l = (lang ?? "").split(/\s/)[0] || "text";
          const language = LANGS.includes(l) ? l : l === "console" || l === "shell" ? "sh" : "text";
          return highlighter.codeToHtml(text, {
            lang: language,
            themes: { dark: "github-dark-default", light: "github-light-default" },
            defaultColor: false,
          });
        },
      },
    });
    const body = await marked.parse(source);
    const nav = PAGES.map((p) => {
      const href = `/docs/${p.slug ? `${p.slug}/` : ""}`;
      const current = p === page ? ' aria-current="page"' : "";
      return `<a href="${href}"${current}>${esc(p.nav)}</a>`;
    }).join("");
    const html = template
      .replace("<!--TITLE-->", esc(title))
      .replace("<!--DESCRIPTION-->", esc(description))
      .replace("<!--NAV-->", nav)
      .replace("<!--DOC-->", body)
      .replace("<!--SOURCE-->", `${REPO}/edit/main/${page.file}`);
    const dir = join(dist, "docs", page.slug);
    await mkdir(dir, { recursive: true });
    await writeFile(join(dir, "index.html"), html);
  }
  await rm(join(dist, "docs.html"));
  console.log(`rendered ${PAGES.length} docs pages`);
}

await main();

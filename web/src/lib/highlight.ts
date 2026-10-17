// Syntax highlighting with Shiki, loaded lazily (grammars load per language).

const cache = new Map<string, Promise<string>>();
const LANG_ALIASES: Record<string, string> = {
  sh: "bash",
  shell: "bash",
  zsh: "bash",
  console: "bash",
  yml: "yaml",
  rs: "rust",
  py: "python",
  ts: "typescript",
  js: "javascript",
  md: "markdown",
  golang: "go",
  "c++": "cpp",
  toml: "toml",
  diff: "diff",
};

export function highlight(code: string, lang: string): Promise<string> {
  const l = (LANG_ALIASES[lang.toLowerCase()] ?? lang.toLowerCase()) || "text";
  const key = `${l}\u0000${code}`;
  let p = cache.get(key);
  if (!p) {
    p = import("shiki").then(async ({ codeToHtml, bundledLanguages }) => {
      const known = l in bundledLanguages ? l : "text";
      return codeToHtml(code, {
        lang: known,
        themes: { light: "github-light", dark: "github-dark-default" },
        defaultColor: false,
      });
    });
    cache.set(key, p);
    if (cache.size > 400) {
      const first = cache.keys().next().value;
      if (first !== undefined) cache.delete(first);
    }
  }
  return p;
}

/** A best guess at the language of a file path. */
export function langOf(path: string): string {
  const ext = path.split(".").pop()?.toLowerCase() ?? "";
  const map: Record<string, string> = {
    rs: "rust",
    ts: "typescript",
    tsx: "tsx",
    js: "javascript",
    jsx: "jsx",
    mjs: "javascript",
    py: "python",
    go: "go",
    rb: "ruby",
    java: "java",
    kt: "kotlin",
    swift: "swift",
    c: "c",
    h: "c",
    cpp: "cpp",
    cs: "csharp",
    css: "css",
    scss: "scss",
    html: "html",
    vue: "vue",
    svelte: "svelte",
    json: "json",
    toml: "toml",
    yaml: "yaml",
    yml: "yaml",
    md: "markdown",
    sh: "bash",
    bash: "bash",
    sql: "sql",
    dockerfile: "dockerfile",
    tf: "hcl",
    lua: "lua",
    php: "php",
  };
  if (path.toLowerCase().endsWith("dockerfile")) return "dockerfile";
  return map[ext] ?? "text";
}

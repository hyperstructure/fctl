// Syntax highlighting with Shiki, loaded lazily (grammars load per language).

import { IGNORE_GRAMMAR, languageOf } from "./languages";

const THEMES = { light: "github-light", dark: "github-dark-default" } as const;

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
  env: "dotenv",
  gitignore: "ignore",
  dockerignore: "ignore",
};

/** Shiki's highlighter, with our own grammars (ignore files) loaded into it. */
const highlighter = (() => {
  let p: Promise<import("shiki").Highlighter> | undefined;
  return () => {
    p ??= import("shiki").then(async ({ getSingletonHighlighter }) => {
      const h = await getSingletonHighlighter({ themes: [THEMES.light, THEMES.dark] });
      await h.loadLanguage(IGNORE_GRAMMAR as never);
      return h;
    });
    return p;
  };
})();

/** The language Shiki knows `lang` as, loaded, or `text` for one it doesn't. */
async function loaded(lang: string): Promise<string> {
  const lower = lang.toLowerCase();
  // Own keys only: `constructor` is no language.
  const l = (Object.hasOwn(LANG_ALIASES, lower) ? LANG_ALIASES[lower] : lower) || "text";
  const h = await highlighter();
  if (l === "text" || h.getLoadedLanguages().includes(l)) return l;
  const { bundledLanguages } = await import("shiki");
  if (!Object.hasOwn(bundledLanguages, l)) return "text";
  await h.loadLanguage(l as never);
  return l;
}

const cache = new Map<string, Promise<string>>();

export function highlight(code: string, lang: string): Promise<string> {
  const key = `${lang}\u0000${code}`;
  let p = cache.get(key);
  if (!p) {
    p = loaded(lang).then(async (l) =>
      (await highlighter()).codeToHtml(code, { lang: l as never, themes: THEMES, defaultColor: false }),
    );
    remember(cache, key, p);
  }
  return p;
}

/** A piece of a highlighted line: its text, and its colors in both themes as CSS variables. */
export interface Token {
  content: string;
  style?: Record<string, string>;
}

const tokenCache = new Map<string, Promise<Token[][] | undefined>>();

/**
 * Highlights lines of code as one piece (a construct can span them):
 * each line's tokens, or none for a language Shiki doesn't know.
 */
export function tokenize(lines: string[], lang: string): Promise<Token[][] | undefined> {
  const code = lines.join("\n");
  const key = `${lang}\u0000${code}`;
  let p = tokenCache.get(key);
  if (!p) {
    p = loaded(lang).then(async (l) => {
      if (l === "text") return undefined;
      const { tokens } = (await highlighter()).codeToTokens(code, {
        lang: l as never,
        themes: THEMES,
        defaultColor: false,
      });
      return tokens.map((line) =>
        line.map((t) => ({ content: t.content, style: t.htmlStyle as Record<string, string> | undefined })),
      );
    });
    remember(tokenCache, key, p);
  }
  return p;
}

/** Keeps `value` under `key`, forgetting the oldest past 400. */
function remember<T>(map: Map<string, T>, key: string, value: T) {
  map.set(key, value);
  if (map.size > 400) {
    const first = map.keys().next().value;
    if (first !== undefined) map.delete(first);
  }
}

/** A best guess at the language of a file path, as Shiki knows it. */
export function langOf(path: string, firstLine?: string): string {
  return languageOf(path, firstLine).id;
}

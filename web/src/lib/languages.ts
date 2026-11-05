// What language a file is in, for syntax highlighting: by its name, its
// extension, or its first line (a script's shebang, a lockfile's format).

/** A language Shiki highlights, by its id, and what people call it. */
export interface Language {
  /** Shiki's id, or one of ours (`ignore`), or `text`. */
  id: string;
  label: string;
}

const lang = (id: string, label: string): Language => ({ id, label });

const TEXT = lang("text", "Plain text");
const RUST = lang("rust", "Rust");
const TS = lang("typescript", "TypeScript");
const TSX = lang("tsx", "TSX");
const JS = lang("javascript", "JavaScript");
const JSX = lang("jsx", "JSX");
const PY = lang("python", "Python");
const SH = lang("shellscript", "Shell");
const JSON_ = lang("json", "JSON");
const JSONC = lang("jsonc", "JSON with comments");
const TOML = lang("toml", "TOML");
const YAML = lang("yaml", "YAML");
const MD = lang("markdown", "Markdown");
const DOCKER = lang("docker", "Dockerfile");
const DOTENV = lang("dotenv", ".env");
const IGNORE = lang("ignore", "Ignore file");
const TERRAFORM = lang("terraform", "Terraform");
const HCL = lang("hcl", "HCL");
const RUBY = lang("ruby", "Ruby");
const INI = lang("ini", "INI");
const MAKE = lang("make", "Makefile");
const XML = lang("xml", "XML");
const GROOVY = lang("groovy", "Groovy");

/** Files known by their whole name, lowercase. */
const NAMES: Record<string, Language> = {
  dockerfile: DOCKER,
  containerfile: DOCKER,
  makefile: MAKE,
  gnumakefile: MAKE,
  justfile: lang("just", "Justfile"),
  ".justfile": lang("just", "Justfile"),
  "cmakelists.txt": lang("cmake", "CMake"),
  gemfile: RUBY,
  rakefile: RUBY,
  podfile: RUBY,
  brewfile: RUBY,
  fastfile: RUBY,
  appfile: RUBY,
  vagrantfile: RUBY,
  guardfile: RUBY,
  capfile: RUBY,
  dangerfile: RUBY,
  jenkinsfile: GROOVY,
  tiltfile: PY,
  "build.bazel": PY,
  "workspace.bazel": PY,
  "module.bazel": PY,
  pipfile: TOML,
  "cargo.lock": TOML,
  "poetry.lock": TOML,
  "uv.lock": TOML,
  "pdm.lock": TOML,
  "pixi.lock": YAML,
  "pipfile.lock": JSON_,
  "composer.lock": JSON_,
  "flake.lock": JSON_,
  "deno.lock": JSON_,
  "package.resolved": JSON_,
  "bun.lock": JSONC,
  "yarn.lock": YAML,
  "gemfile.lock": TEXT,
  "mix.lock": lang("elixir", "Elixir"),
  "go.sum": TEXT,
  "go.mod": TEXT,
  "go.work": TEXT,
  codeowners: lang("codeowners", "CODEOWNERS"),
  ".gitattributes": IGNORE,
  ".gitmodules": INI,
  ".gitconfig": INI,
  ".editorconfig": INI,
  ".npmrc": INI,
  ".yarnrc": INI,
  ".pypirc": INI,
  ".flake8": INI,
  ".pylintrc": INI,
  ".coveragerc": INI,
  "setup.cfg": INI,
  "tox.ini": INI,
  ".envrc": SH,
  ".bashrc": SH,
  ".bash_profile": SH,
  ".bash_aliases": SH,
  ".zshrc": SH,
  ".zprofile": SH,
  ".zshenv": SH,
  ".profile": SH,
  ".babelrc": JSONC,
  ".eslintrc": JSONC,
  ".prettierrc": JSON_,
  ".swcrc": JSONC,
  "tsconfig.json": JSONC,
  "jsconfig.json": JSONC,
  "biome.json": JSONC,
  "devcontainer.json": JSONC,
  ".devcontainer.json": JSONC,
  "deno.json": JSONC,
  "turbo.json": JSONC,
  procfile: YAML,
  "nginx.conf": lang("nginx", "nginx"),
  ssh_config: lang("ssh-config", "SSH config"),
  sshd_config: lang("ssh-config", "SSH config"),
};

/** Files known by their extension (after the last dot), lowercase. */
const EXTENSIONS: Record<string, Language> = {
  rs: RUST,
  ts: TS,
  mts: TS,
  cts: TS,
  tsx: TSX,
  js: JS,
  mjs: JS,
  cjs: JS,
  jsx: JSX,
  py: PY,
  pyi: PY,
  pyw: PY,
  bzl: PY,
  star: PY,
  sh: SH,
  bash: SH,
  zsh: SH,
  ksh: SH,
  command: SH,
  fish: lang("fish", "Fish"),
  nu: lang("nushell", "Nushell"),
  ps1: lang("powershell", "PowerShell"),
  psm1: lang("powershell", "PowerShell"),
  bat: lang("bat", "Batch"),
  cmd: lang("bat", "Batch"),
  json: JSON_,
  geojson: JSON_,
  webmanifest: JSON_,
  har: JSON_,
  ipynb: JSON_,
  tfstate: JSON_,
  jsonc: JSONC,
  code_workspace: JSONC,
  json5: lang("json5", "JSON5"),
  jsonl: lang("jsonl", "JSON Lines"),
  ndjson: lang("jsonl", "JSON Lines"),
  toml: TOML,
  yaml: YAML,
  yml: YAML,
  md: MD,
  markdown: MD,
  mdx: lang("mdx", "MDX"),
  rst: lang("rst", "reStructuredText"),
  adoc: lang("asciidoc", "AsciiDoc"),
  tex: lang("latex", "LaTeX"),
  dockerfile: DOCKER,
  containerfile: DOCKER,
  env: DOTENV,
  tf: TERRAFORM,
  tfvars: TERRAFORM,
  hcl: HCL,
  nomad: HCL,
  gitignore: IGNORE,
  dockerignore: IGNORE,
  go: lang("go", "Go"),
  rb: RUBY,
  rake: RUBY,
  gemspec: RUBY,
  ru: RUBY,
  erb: lang("erb", "ERB"),
  java: lang("java", "Java"),
  kt: lang("kotlin", "Kotlin"),
  kts: lang("kotlin", "Kotlin"),
  scala: lang("scala", "Scala"),
  sc: lang("scala", "Scala"),
  groovy: GROOVY,
  gradle: GROOVY,
  swift: lang("swift", "Swift"),
  c: lang("c", "C"),
  h: lang("c", "C"),
  cc: lang("cpp", "C++"),
  cpp: lang("cpp", "C++"),
  cxx: lang("cpp", "C++"),
  hpp: lang("cpp", "C++"),
  hh: lang("cpp", "C++"),
  hxx: lang("cpp", "C++"),
  m: lang("objective-c", "Objective-C"),
  mm: lang("objective-cpp", "Objective-C++"),
  cs: lang("csharp", "C#"),
  fs: lang("fsharp", "F#"),
  fsx: lang("fsharp", "F#"),
  vb: lang("vb", "Visual Basic"),
  dart: lang("dart", "Dart"),
  zig: lang("zig", "Zig"),
  nim: lang("nim", "Nim"),
  nix: lang("nix", "Nix"),
  lua: lang("lua", "Lua"),
  php: lang("php", "PHP"),
  pl: lang("perl", "Perl"),
  pm: lang("perl", "Perl"),
  r: lang("r", "R"),
  jl: lang("julia", "Julia"),
  ex: lang("elixir", "Elixir"),
  exs: lang("elixir", "Elixir"),
  erl: lang("erlang", "Erlang"),
  hrl: lang("erlang", "Erlang"),
  hs: lang("haskell", "Haskell"),
  ml: lang("ocaml", "OCaml"),
  mli: lang("ocaml", "OCaml"),
  clj: lang("clojure", "Clojure"),
  cljs: lang("clojure", "Clojure"),
  cljc: lang("clojure", "Clojure"),
  edn: lang("clojure", "Clojure"),
  elm: lang("elm", "Elm"),
  gleam: lang("gleam", "Gleam"),
  cr: lang("crystal", "Crystal"),
  d: lang("d", "D"),
  odin: lang("odin", "Odin"),
  sol: lang("solidity", "Solidity"),
  move: lang("move", "Move"),
  css: lang("css", "CSS"),
  scss: lang("scss", "SCSS"),
  sass: lang("sass", "Sass"),
  less: lang("less", "Less"),
  styl: lang("stylus", "Stylus"),
  html: lang("html", "HTML"),
  htm: lang("html", "HTML"),
  xhtml: lang("html", "HTML"),
  vue: lang("vue", "Vue"),
  svelte: lang("svelte", "Svelte"),
  astro: lang("astro", "Astro"),
  xml: XML,
  svg: XML,
  plist: XML,
  xsd: XML,
  xsl: lang("xsl", "XSL"),
  csproj: XML,
  sql: lang("sql", "SQL"),
  graphql: lang("graphql", "GraphQL"),
  gql: lang("graphql", "GraphQL"),
  proto: lang("proto", "Protocol Buffers"),
  prisma: lang("prisma", "Prisma"),
  wgsl: lang("wgsl", "WGSL"),
  glsl: lang("glsl", "GLSL"),
  vert: lang("glsl", "GLSL"),
  frag: lang("glsl", "GLSL"),
  comp: lang("glsl", "GLSL"),
  hlsl: lang("hlsl", "HLSL"),
  ini: INI,
  cfg: INI,
  conf: INI,
  properties: lang("properties", "Properties"),
  service: lang("systemd", "systemd"),
  socket: lang("systemd", "systemd"),
  timer: lang("systemd", "systemd"),
  mk: MAKE,
  cmake: lang("cmake", "CMake"),
  diff: lang("diff", "Diff"),
  patch: lang("diff", "Diff"),
  csv: lang("csv", "CSV"),
  tsv: lang("tsv", "TSV"),
  log: lang("log", "Log"),
  http: lang("http", "HTTP"),
  vim: lang("viml", "Vim script"),
  el: lang("emacs-lisp", "Emacs Lisp"),
  lisp: lang("common-lisp", "Common Lisp"),
  scm: lang("scheme", "Scheme"),
  rkt: lang("racket", "Racket"),
  tcl: lang("tcl", "Tcl"),
  awk: lang("awk", "AWK"),
  asm: lang("asm", "Assembly"),
  s: lang("asm", "Assembly"),
  wat: lang("wasm", "WebAssembly"),
  coffee: lang("coffee", "CoffeeScript"),
  j2: lang("jinja", "Jinja"),
  jinja: lang("jinja", "Jinja"),
  jinja2: lang("jinja", "Jinja"),
  hbs: lang("handlebars", "Handlebars"),
  handlebars: lang("handlebars", "Handlebars"),
  twig: lang("twig", "Twig"),
  liquid: lang("liquid", "Liquid"),
  pug: lang("pug", "Pug"),
  haml: lang("haml", "Haml"),
  typ: lang("typst", "Typst"),
  kdl: lang("kdl", "KDL"),
  ron: lang("ron", "RON"),
  pkl: lang("pkl", "Pkl"),
  cue: lang("cue", "CUE"),
  jsonnet: lang("jsonnet", "Jsonnet"),
  libsonnet: lang("jsonnet", "Jsonnet"),
  mermaid: lang("mermaid", "Mermaid"),
  mmd: lang("mermaid", "Mermaid"),
  applescript: lang("applescript", "AppleScript"),
  sv: lang("system-verilog", "SystemVerilog"),
  vhd: lang("vhdl", "VHDL"),
  vhdl: lang("vhdl", "VHDL"),
  txt: TEXT,
};

/** What a script's `#!` line names as its interpreter. */
const INTERPRETERS: Record<string, Language> = {
  sh: SH,
  bash: SH,
  zsh: SH,
  dash: SH,
  ksh: SH,
  fish: lang("fish", "Fish"),
  node: JS,
  nodejs: JS,
  deno: TS,
  bun: TS,
  tsx: TS,
  "ts-node": TS,
  python: PY,
  ruby: RUBY,
  perl: lang("perl", "Perl"),
  php: lang("php", "PHP"),
  lua: lang("lua", "Lua"),
  pwsh: lang("powershell", "PowerShell"),
  elixir: lang("elixir", "Elixir"),
  rscript: lang("r", "R"),
  make: MAKE,
};

/** Ignore files: git's, Docker's and their kin (`.prettierignore`, `.npmignore`…). */
const isIgnoreFile = (name: string) => name.startsWith(".") && name.endsWith("ignore") && name.length > 7;

/** A `.env` file: `.env`, `.env.example`, `.env.local`, `prod.env`, wrangler's `.dev.vars`… */
const isEnvFile = (name: string) =>
  name === ".env" || name.startsWith(".env.") || name.endsWith(".env") || /^\.dev\.vars(\..+)?$/.test(name);

/**
 * The language of the file at `path`, by its name or extension; for a file
 * those don't tell, by its first line (`firstLine`): a script's `#!`, or
 * the format of a lockfile.
 */
export function languageOf(path: string, firstLine?: string): Language {
  const name = (path.split("/").pop() ?? path).toLowerCase();
  const known = Object.hasOwn(NAMES, name) ? NAMES[name] : undefined;
  if (known) return known;
  if (isEnvFile(name)) return DOTENV;
  if (isIgnoreFile(name)) return IGNORE;
  if (/^(docker|container)file[.-]/.test(name) || name.endsWith(".dockerfile")) return DOCKER;
  if (/^requirements.*\.(txt|in)$/.test(name) || name === "constraints.txt") return TEXT;
  const dot = name.lastIndexOf(".");
  const ext = dot > 0 ? name.slice(dot + 1) : "";
  const byExt = ext && Object.hasOwn(EXTENSIONS, ext) ? EXTENSIONS[ext] : undefined;
  if (byExt) return byExt;
  if (ext === "lock") return sniffLock(firstLine);
  return (firstLine && shebang(firstLine)) || TEXT;
}

/** The language a script's `#!` line names, as `#!/usr/bin/env -S deno run` or `#!/bin/bash -e` do. */
export function shebang(line: string): Language | undefined {
  const m = /^#!\s*(\S+)(.*)$/.exec(line.trim());
  if (!m) return undefined;
  const [, program = "", rest = ""] = m;
  let name = program.split("/").pop() ?? "";
  if (name === "env") {
    const args = rest
      .trim()
      .split(/\s+/)
      .filter((a) => !a.startsWith("-") && !a.includes("="));
    name = args[0] ?? "";
  }
  // `python3.12`, `lua5.4`: the version doesn't matter.
  name = name.toLowerCase().replace(/[\d.]+$/, "");
  return Object.hasOwn(INTERPRETERS, name) ? INTERPRETERS[name] : undefined;
}

/** A lockfile's format by its first line: JSON, TOML (`[[package]]`, `version = 4`) or else YAML. */
function sniffLock(firstLine?: string): Language {
  const line = firstLine?.trim() ?? "";
  if (line.startsWith("{") || line.startsWith("[{") || line === "[") return JSON_;
  if (/^\[\[?[\w.-]+\]\]?$/.test(line) || /^[\w.-]+\s*=/.test(line) || line.startsWith("#")) return TOML;
  return line ? YAML : TEXT;
}

/**
 * A minimal grammar for ignore files (`.gitignore`, `.dockerignore`, and
 * `.gitattributes`' patterns), which Shiki has none of: comments, negation,
 * wildcards and character classes.
 */
export const IGNORE_GRAMMAR = {
  name: "ignore",
  scopeName: "source.ignore",
  patterns: [
    { match: "^\\s*#.*$", name: "comment.line.number-sign.ignore" },
    { match: "^\\s*!", name: "keyword.operator.negation.ignore" },
    { match: "\\\\.", name: "constant.character.escape.ignore" },
    { match: "\\*\\*|\\*|\\?", name: "keyword.operator.glob.ignore" },
    { match: "\\[[^\\]]*\\]", name: "constant.other.character-class.ignore" },
    { match: "(?<=\\s)[-\\w]+(=\\S*)?", name: "entity.other.attribute-name.ignore" },
  ],
};

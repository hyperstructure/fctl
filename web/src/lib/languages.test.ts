import { describe, expect, it } from "vitest";
import { languageOf, shebang } from "./languages";

const id = (path: string, first?: string) => languageOf(path, first).id;

describe("a file's language", () => {
  it("goes by the extension", () => {
    expect(id("src/main.rs")).toBe("rust");
    expect(id("web/src/lib/diff.ts")).toBe("typescript");
    expect(id("web/src/routes/session.tsx")).toBe("tsx");
    expect(id("scripts/build.mjs")).toBe("javascript");
    expect(id("tools/gen.py")).toBe("python");
    expect(id("scripts/install.sh")).toBe("shellscript");
    expect(id("README.md")).toBe("markdown");
    expect(id("infra/main.tf")).toBe("terraform");
    expect(id("infra/prod.tfvars")).toBe("terraform");
    expect(id("deploy/job.hcl")).toBe("hcl");
    expect(id("Cargo.toml")).toBe("toml");
    expect(id("web/package.json")).toBe("json");
    expect(id(".github/workflows/ci.yml")).toBe("yaml");
    expect(id("SRC/MAIN.RS")).toBe("rust");
  });

  it("goes by the whole name for files that have one of their own", () => {
    expect(id("Dockerfile")).toBe("docker");
    expect(id("deploy/standalone/Dockerfile")).toBe("docker");
    expect(id("Dockerfile.dev")).toBe("docker");
    expect(id("docker/web.Dockerfile")).toBe("docker");
    expect(id("Containerfile")).toBe("docker");
    expect(id("Makefile")).toBe("make");
    expect(id("justfile")).toBe("just");
    expect(id("Gemfile")).toBe("ruby");
    expect(id("Jenkinsfile")).toBe("groovy");
    expect(id(".github/CODEOWNERS")).toBe("codeowners");
    expect(id("tsconfig.json")).toBe("jsonc");
    expect(id(".envrc")).toBe("shellscript");
  });

  it("knows ignore files", () => {
    for (const f of [
      ".gitignore",
      "web/.gitignore",
      ".dockerignore",
      ".prettierignore",
      ".npmignore",
      ".gitattributes",
    ]) {
      expect(id(f), f).toBe("ignore");
    }
    expect(languageOf(".dockerignore").label).toBe("Ignore file");
  });

  it("knows .env files, examples included", () => {
    for (const f of [
      ".env",
      ".env.example",
      ".env.local",
      "deploy/.env.production",
      "prod.env",
      ".dev.vars",
    ]) {
      expect(id(f), f).toBe("dotenv");
    }
    expect(languageOf(".env.example").label).toBe(".env");
  });

  it("knows lockfiles in JSON and in TOML, and sniffs the others", () => {
    expect(id("Cargo.lock")).toBe("toml");
    expect(id("poetry.lock")).toBe("toml");
    expect(id("uv.lock")).toBe("toml");
    expect(id("package-lock.json")).toBe("json");
    expect(id("composer.lock")).toBe("json");
    expect(id("flake.lock")).toBe("json");
    expect(id("bun.lock")).toBe("jsonc");
    expect(id("yarn.lock")).toBe("yaml");
    expect(id("pnpm-lock.yaml")).toBe("yaml");
    expect(id(".terraform.lock.hcl")).toBe("hcl");
    expect(id("other.lock", "{")).toBe("json");
    expect(id("other.lock", "[[package]]")).toBe("toml");
    expect(id("other.lock", "version = 4")).toBe("toml");
    expect(id("other.lock", "# This file is generated")).toBe("toml");
    expect(id("other.lock", "lockfileVersion: 9")).toBe("yaml");
    expect(id("other.lock")).toBe("text");
  });

  it("reads a script's #! line when its name says nothing", () => {
    expect(id("bin/deploy", "#!/usr/bin/env bash")).toBe("shellscript");
    expect(id("bin/deploy", "#!/bin/sh -e")).toBe("shellscript");
    expect(id("scripts/tool", "#!/usr/bin/env python3.12")).toBe("python");
    expect(id("scripts/tool", "#!/usr/bin/env -S deno run --allow-net")).toBe("typescript");
    expect(id("scripts/tool", "#!/usr/bin/env node")).toBe("javascript");
    expect(id("scripts/tool", "#!/usr/bin/env FOO=1 ruby")).toBe("ruby");
    expect(id("scripts/tool", "echo hi")).toBe("text");
    expect(id("LICENSE")).toBe("text");
    // The extension wins over the line.
    expect(id("x.py", "#!/bin/bash")).toBe("python");
    expect(shebang("not a shebang")).toBeUndefined();
  });

  it("takes no keys of Object for names", () => {
    expect(id("constructor")).toBe("text");
    expect(id("a.constructor")).toBe("text");
    expect(id("toString")).toBe("text");
  });
});

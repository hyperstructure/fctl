import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import {
  asked,
  chosenModel,
  costsCache,
  effortOffers,
  modelChoice,
  modelLabel,
  modelOffers,
  startChoice,
  switchEfforts,
  switchModels,
} from "./models";

describe("project models", () => {
  it("shows Claude's default, an alias, or a model ID", () => {
    expect(modelChoice(undefined)).toBe("");
    expect(modelChoice("")).toBe("");
    expect(modelChoice("  ")).toBe("");
    expect(modelChoice("opus")).toBe("opus");
    expect(modelChoice("sonnet")).toBe("sonnet");
    expect(modelChoice("haiku")).toBe("haiku");
    expect(modelChoice("fable")).toBe("fable");
    expect(modelChoice("claude-opus-5-5")).toBe("id");
    // Other aliases, and the ones above spelled otherwise, show as they are.
    expect(modelChoice("opusplan")).toBe("id");
    expect(modelChoice("opus[1m]")).toBe("id");
    expect(modelChoice("Opus")).toBe("id");
  });

  it("names a model by its alias, or by its family and version", () => {
    expect(modelLabel("")).toBe("Claude's default");
    expect(modelLabel("sonnet")).toBe("Sonnet");
    expect(modelLabel("claude-sonnet-5-5")).toBe("Sonnet 5.5");
    expect(modelLabel("claude-opus-4-1-20250805")).toBe("Opus 4.1");
    expect(modelLabel("claude-opus-4-20250514")).toBe("Opus 4");
    expect(modelLabel("claude-haiku-4-5")).toBe("Haiku 4.5");
    // What it can't tell, as it is.
    expect(modelLabel("opus[1m]")).toBe("opus[1m]");
    expect(modelLabel("claude-sonnet-5-5[1m]")).toBe("claude-sonnet-5-5[1m]");
    expect(modelLabel("claude-3-5-sonnet-20241022")).toBe("claude-3-5-sonnet-20241022");
    expect(modelLabel("claude-mythos-preview")).toBe("claude-mythos-preview");
  });

  it("stores the alias, or the model ID typed for it", () => {
    expect(chosenModel("")).toBeUndefined();
    expect(chosenModel("", "claude-opus-5-5")).toBeUndefined();
    expect(chosenModel("sonnet", "claude-opus-5-5")).toBe("sonnet");
    expect(chosenModel("haiku")).toBe("haiku");
    expect(chosenModel("id", "claude-opus-5-5")).toBe("claude-opus-5-5");
    expect(chosenModel("id")).toBeUndefined();
    expect(chosenModel("id", " ")).toBeUndefined();
  });
});

const project = (
  settings: Partial<G.ProjectSettings>,
  last: Pick<G.Project, "last_model" | "last_effort"> = {},
) => ({ settings: settings as G.ProjectSettings, ...last });

describe("the composer's model and effort", () => {
  it("starts on what you last asked for in the project, else on the project's", () => {
    expect(startChoice(project({}))).toEqual({ model: "", effort: "", github: "push" });
    expect(startChoice(project({ model: "opus", effort: "high" }))).toEqual({
      model: "opus",
      effort: "high",
      github: "push",
    });
    expect(
      startChoice(project({ model: "opus", effort: "high" }, { last_model: "haiku", last_effort: "low" })),
    ).toEqual({ model: "haiku", effort: "low", github: "push" });
    // Where you took the project's last time, its settings now.
    expect(startChoice(project({ model: "opus", effort: "high" }, { last_effort: "max" }))).toEqual({
      model: "opus",
      effort: "max",
      github: "push",
    });
  });

  it("keeps what was picked, as far as the project allows it", () => {
    const p = project({ model: "opus" }, { last_model: "sonnet" });
    expect(startChoice(p, { model: "fable" })).toEqual({ model: "fable", effort: "", github: "push" });
    expect(startChoice(p, { effort: "xhigh" })).toEqual({ model: "sonnet", effort: "xhigh", github: "push" });
    // Claude's default, picked in a project without a model, isn't one a session can ask for here.
    expect(startChoice(p, { model: "" })).toEqual({ model: "sonnet", effort: "", github: "push" });
    expect(startChoice(project({}), { model: "" })).toEqual({ model: "", effort: "", github: "push" });
  });

  it("asks only for what differs from the project's settings", () => {
    const p = project({ model: "opus", effort: "high" });
    expect(asked(p, { model: "opus", effort: "high", github: "push" })).toEqual({});
    expect(asked(p, { model: "haiku", effort: "high", github: "push" })).toEqual({ model: "haiku" });
    expect(asked(p, { model: "opus", effort: "max", github: "push" })).toEqual({ effort: "max" });
    expect(asked(project({}), { model: "", effort: "", github: "push" })).toEqual({});
    expect(asked(project({}), { model: "fable", effort: "low", github: "push" })).toEqual({
      model: "fable",
      effort: "low",
    });
  });

  it("does on GitHub what was picked, as far as the project allows it", () => {
    const reviews = project({ github_access: "comment" });
    expect(startChoice(project({})).github).toBe("push");
    expect(startChoice(reviews).github).toBe("comment");
    expect(startChoice(reviews, { github: "read" }).github).toBe("read");
    // A project that holds its agents to commenting doesn't let one push.
    expect(startChoice(reviews, { github: "push" }).github).toBe("comment");
    // A session asks only for less than the project allows.
    expect(asked(reviews, { model: "", effort: "", github: "comment" })).toEqual({});
    expect(asked(reviews, { model: "", effort: "", github: "read" })).toEqual({ github_access: "read" });
    // Reaching GitHub's issues beyond the tracker is asked anew each time.
    expect(asked(reviews, { model: "", effort: "", github: "comment", issues: true })).toEqual({
      github_issues: true,
    });
    expect(asked(reviews, { model: "", effort: "", github: "comment", issues: false })).toEqual({});
  });

  it("offers the aliases, Claude's default where the project has it, and the models it names", () => {
    const values = (p: ReturnType<typeof project>, shown: string) =>
      modelOffers(p, shown).map((o) => o.value);
    expect(values(project({}), "")).toEqual(["", "opus", "sonnet", "haiku", "fable"]);
    expect(values(project({ model: "sonnet" }), "sonnet")).toEqual(["opus", "sonnet", "haiku", "fable"]);
    expect(
      values(project({ model: "claude-opus-5-5" }, { last_model: "claude-haiku-5-5" }), "claude-haiku-5-5"),
    ).toEqual(["opus", "sonnet", "haiku", "fable", "claude-opus-5-5", "claude-haiku-5-5"]);
    // One picked in another project shows too.
    expect(values(project({ model: "opus" }), "opus[1m]")).toEqual([
      "opus",
      "sonnet",
      "haiku",
      "fable",
      "opus[1m]",
    ]);
    const offers = modelOffers(project({ model: "haiku" }), "opus");
    expect(offers.filter((o) => o.own).map((o) => o.label)).toEqual(["Haiku"]);
    expect(modelOffers(project({}), "")[0]).toEqual({ value: "", label: "Claude's default", own: true });
  });

  it("offers the effort levels, and Claude's default where the project has it", () => {
    const values = (p: ReturnType<typeof project>, shown: string) =>
      effortOffers(p, shown).map((o) => o.value);
    expect(values(project({}), "")).toEqual(["", "low", "medium", "high", "xhigh", "max"]);
    expect(values(project({ effort: "high" }), "low")).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(effortOffers(project({ effort: "xhigh" }), "xhigh").find((o) => o.own)).toEqual({
      value: "xhigh",
      label: "Extra high",
      own: true,
    });
  });
});

describe("a running session's model and effort level", () => {
  const values = (list: { value: string }[]) => list.map((o) => o.value);

  it("offers the model it works on, then the aliases", () => {
    expect(values(switchModels("claude-opus-5-5"))).toEqual([
      "claude-opus-5-5",
      "opus",
      "sonnet",
      "haiku",
      "fable",
    ]);
    expect(values(switchModels("sonnet"))).toEqual(["opus", "sonnet", "haiku", "fable"]);
    // Before Claude said, on Claude's default.
    expect(switchModels("")[0]).toEqual({ value: "", label: "Claude's default" });
  });

  it("offers the level it works at, then the levels", () => {
    expect(values(switchEfforts("xhigh"))).toEqual(["low", "medium", "high", "xhigh", "max"]);
    expect(values(switchEfforts(""))).toEqual(["", "low", "medium", "high", "xhigh", "max"]);
  });

  it("knows which changes cost Claude its prompt cache", () => {
    expect(costsCache("model")).toBe(true);
    expect(costsCache("model", true)).toBe(true);
    expect(costsCache("effort", true)).toBe(false);
    expect(costsCache("effort")).toBe(false);
    expect(costsCache("effort", false)).toBe(true);
  });
});

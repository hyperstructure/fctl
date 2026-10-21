import { describe, expect, it } from "vitest";
import type * as G from "@/gen";
import { asked, chosenModel, effortOffers, modelChoice, modelOffers, startChoice } from "./models";

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
    expect(startChoice(project({}))).toEqual({ model: "", effort: "" });
    expect(startChoice(project({ model: "opus", effort: "high" }))).toEqual({
      model: "opus",
      effort: "high",
    });
    expect(
      startChoice(project({ model: "opus", effort: "high" }, { last_model: "haiku", last_effort: "low" })),
    ).toEqual({ model: "haiku", effort: "low" });
    // Where you took the project's last time, its settings now.
    expect(startChoice(project({ model: "opus", effort: "high" }, { last_effort: "max" }))).toEqual({
      model: "opus",
      effort: "max",
    });
  });

  it("keeps what was picked, as far as the project allows it", () => {
    const p = project({ model: "opus" }, { last_model: "sonnet" });
    expect(startChoice(p, { model: "fable" })).toEqual({ model: "fable", effort: "" });
    expect(startChoice(p, { effort: "xhigh" })).toEqual({ model: "sonnet", effort: "xhigh" });
    // Claude's default, picked in a project without a model, isn't one a session can ask for here.
    expect(startChoice(p, { model: "" })).toEqual({ model: "sonnet", effort: "" });
    expect(startChoice(project({}), { model: "" })).toEqual({ model: "", effort: "" });
  });

  it("asks only for what differs from the project's settings", () => {
    const p = project({ model: "opus", effort: "high" });
    expect(asked(p, { model: "opus", effort: "high" })).toEqual({});
    expect(asked(p, { model: "haiku", effort: "high" })).toEqual({ model: "haiku" });
    expect(asked(p, { model: "opus", effort: "max" })).toEqual({ effort: "max" });
    expect(asked(project({}), { model: "", effort: "" })).toEqual({});
    expect(asked(project({}), { model: "fable", effort: "low" })).toEqual({ model: "fable", effort: "low" });
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

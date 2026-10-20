import { describe, expect, it } from "vitest";
import { chosenModel, modelChoice } from "./models";

describe("project models", () => {
  it("shows Claude's default, an alias, or a model ID", () => {
    expect(modelChoice(undefined)).toBe("");
    expect(modelChoice("")).toBe("");
    expect(modelChoice("  ")).toBe("");
    expect(modelChoice("opus")).toBe("opus");
    expect(modelChoice("sonnet")).toBe("sonnet");
    expect(modelChoice("fable")).toBe("fable");
    expect(modelChoice("claude-opus-5-5")).toBe("id");
    // Other aliases, and the ones above spelled otherwise, show as they are.
    expect(modelChoice("haiku")).toBe("id");
    expect(modelChoice("opus[1m]")).toBe("id");
    expect(modelChoice("Opus")).toBe("id");
  });

  it("stores the alias, or the model ID typed for it", () => {
    expect(chosenModel("")).toBeUndefined();
    expect(chosenModel("", "claude-opus-5-5")).toBeUndefined();
    expect(chosenModel("sonnet", "claude-opus-5-5")).toBe("sonnet");
    expect(chosenModel("id", "claude-opus-5-5")).toBe("claude-opus-5-5");
    expect(chosenModel("id")).toBeUndefined();
    expect(chosenModel("id", " ")).toBeUndefined();
  });
});

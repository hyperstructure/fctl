// The model a project's agents use (ProjectSettings.model): Claude's default,
// the latest Opus, Sonnet or Fable by Claude Code's alias for it, or a model
// by its ID.

/** What the project's model field shows: "" for Claude's default, an alias, or "id" for a model ID. */
export type ModelChoice = "" | "opus" | "sonnet" | "fable" | "id";

const ALIASES = ["opus", "sonnet", "fable"] as const;

/** The choice that shows `model`: its alias, or "id" for anything else. */
export function modelChoice(model?: string): ModelChoice {
  const m = model?.trim();
  if (!m) return "";
  return ALIASES.find((a) => a === m) ?? "id";
}

/** The model to store for `choice`, with `id` the model ID typed for it. */
export function chosenModel(choice: ModelChoice, id?: string): string | undefined {
  if (choice === "id") return id?.trim() ? id : undefined;
  return choice || undefined;
}

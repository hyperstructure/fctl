// The model and effort level sessions run with. A project sets them for its
// sessions (ProjectSettings.model and .effort), and a session can ask for
// others as it starts (CreateSessionRequest), which the composer does. A model
// is Claude's default, the latest Opus, Sonnet, Haiku or Fable by Claude
// Code's alias for it, or a model by its ID; an effort level is Claude's
// default (the model's), or low … max.

import type * as G from "@/gen";
import { projectGithub, within } from "./github-access";

/** Claude Code's aliases for the latest model of each family, as lists show them. */
export const ALIASES = [
  { value: "opus", label: "Opus" },
  { value: "sonnet", label: "Sonnet" },
  { value: "haiku", label: "Haiku" },
  { value: "fable", label: "Fable" },
] as const;

type Alias = (typeof ALIASES)[number]["value"];

/** What the project's model field shows: "" for Claude's default, an alias, or "id" for a model ID. */
export type ModelChoice = "" | Alias | "id";

/** The choice that shows `model`: its alias, or "id" for anything else. */
export function modelChoice(model?: string): ModelChoice {
  const m = model?.trim();
  if (!m) return "";
  return ALIASES.find((a) => a.value === m)?.value ?? "id";
}

/** The model to store for `choice`, with `id` the model ID typed for it. */
export function chosenModel(choice: ModelChoice, id?: string): string | undefined {
  if (choice === "id") return id?.trim() ? id : undefined;
  return choice || undefined;
}

/** Claude's effort levels, from the quickest to the most thorough. */
export const EFFORTS = [
  { value: "low", label: "Low" },
  { value: "medium", label: "Medium" },
  { value: "high", label: "High" },
  { value: "xhigh", label: "Extra high" },
  { value: "max", label: "Max" },
] as const;

/** A model by its alias's name, or by its ID; "" is Claude's default. */
export const modelLabel = (model: string) =>
  model ? (ALIASES.find((a) => a.value === model)?.label ?? model) : "Claude's default";

/** An effort level by its name; "" is Claude's default. */
export const effortLabel = (effort: string) =>
  effort ? (EFFORTS.find((e) => e.value === effort)?.label ?? effort) : "Claude's default";

/**
 * The model and effort level of a session about to start ("" is Claude's default), and what its
 * agent may do on GitHub.
 */
export interface Choice {
  model: string;
  effort: string;
  github: G.GithubAccess;
}

type Project = Pick<G.Project, "settings" | "last_model" | "last_effort">;

/** What the project's sessions run with unless they ask for something else. */
export function projectChoice(p: Project): Choice {
  return {
    model: p.settings.model?.trim() ?? "",
    effort: p.settings.effort?.trim() ?? "",
    github: projectGithub(p),
  };
}

/**
 * What the composer shows in project `p`: what was picked in it (`picked`), else what you asked
 * for when you last started a session there, else the project's. A session can ask for a model
 * or an effort level but not for Claude's default, which stands only where the project has it.
 * Its agent does on GitHub what was picked, as far as the project allows it, else the most the
 * project allows: each session asks anew.
 */
export function startChoice(p: Project, picked: Partial<Choice> = {}): Choice {
  const own = projectChoice(p);
  const one = (pick: string | undefined, last: string | undefined, theirs: string) =>
    pick !== undefined && (pick || !theirs) ? pick : last?.trim() || theirs;
  return {
    model: one(picked.model, p.last_model, own.model),
    effort: one(picked.effort, p.last_effort, own.effort),
    github: picked.github && within(picked.github, own.github) ? picked.github : own.github,
  };
}

/**
 * What a session started with `c` in `p` asks for: what differs from the project's settings,
 * so that a session follows them where you took them (also when it resumes after they change),
 * and the next one starts on them again.
 */
export function asked(
  p: Project,
  c: Choice,
): Pick<G.CreateSessionRequest, "model" | "effort" | "github_access"> {
  const own = projectChoice(p);
  return {
    model: c.model && c.model !== own.model ? c.model : undefined,
    effort: c.effort && c.effort !== own.effort ? c.effort : undefined,
    github_access: c.github !== own.github ? c.github : undefined,
  };
}

/** A model or an effort level the composer offers: `own` is the project's. */
export interface Offer {
  value: string;
  label: string;
  own: boolean;
}

/**
 * The models the composer offers in `p`: Claude's default where the project has it, the
 * aliases, then the models it names by their IDs (the project's, yours, `shown`).
 */
export function modelOffers(p: Project, shown: string): Offer[] {
  const own = projectChoice(p).model;
  const ids = [own, p.last_model?.trim() ?? "", shown].filter(
    (m, i, all) => m && modelChoice(m) === "id" && all.indexOf(m) === i,
  );
  return [...(own ? [] : [""]), ...ALIASES.map((a) => a.value), ...ids].map((value) => ({
    value,
    label: modelLabel(value),
    own: value === own,
  }));
}

/** The effort levels the composer offers in `p`: Claude's default where the project has it, the levels, and `shown`. */
export function effortOffers(p: Project, shown: string): Offer[] {
  const own = projectChoice(p).effort;
  const levels: string[] = EFFORTS.map((e) => e.value);
  const other = [own, shown].filter((e, i, all) => e && !levels.includes(e) && all.indexOf(e) === i);
  return [...(own ? [] : [""]), ...levels, ...other].map((value) => ({
    value,
    label: effortLabel(value),
    own: value === own,
  }));
}

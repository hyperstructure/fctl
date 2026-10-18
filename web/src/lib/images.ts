// Sandbox images: where a project's sessions start from, and what a runner's
// owner lets it run and build (fctl-domain `image`).

import type * as G from "@/gen";

/** Where a project's sessions start from. */
export type ImageSource = "default" | "image" | "build";

export function imageSource(s: G.ProjectSettings): ImageSource {
  if (s.image_build) return "build";
  return s.image?.trim() && s.image.trim() !== "fctl-agent:latest" ? "image" : "default";
}

/** Settings that start sessions from `source`, keeping what was typed for it. */
export function withSource(s: G.ProjectSettings, source: ImageSource): G.ProjectSettings {
  switch (source) {
    case "default":
      return { ...s, image: undefined, image_build: null };
    case "image":
      return { ...s, image_build: null };
    case "build":
      return { ...s, image: undefined, image_build: s.image_build ?? { file: "" } };
  }
}

/** Whether a build's file is a dev container configuration, which names its own directory. */
export const isDevcontainer = (file: string) => file.trim().toLowerCase().endsWith(".json");

/** "runs any image a project chooses", "runs its own image and ghcr.io/acme/*". */
export function runsImages(allowed?: string[]): string {
  if (!allowed || allowed.some((p) => p.trim() === "*")) return "runs any image a project chooses";
  if (allowed.length === 0) return "runs only its own image";
  return `runs its own image and ${allowed.join(", ")}`;
}

/** "builds no images", "builds images from acme/*". */
export function buildsImages(builds?: string[]): string {
  if (!builds || builds.length === 0) return "builds no images";
  if (builds.some((p) => p.trim() === "*")) return "builds images from any repository";
  return `builds images from ${builds.join(", ")}`;
}
